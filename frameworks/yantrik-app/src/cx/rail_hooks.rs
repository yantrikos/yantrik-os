//! What the core raises through the app's own surface for the agent rail (§5): a proposal
//! applied as an agent through the admitted path, and the rail's refresh after a change.

use std::cell::Cell;
use std::rc::Rc;

use yantrik_surface::{AgentTokenScope, CallerScope, LaterScope, LocalRegistry};

use super::Core;
use crate::app::{App, Changed};
use crate::rail::{self, RailCommand};
use crate::timers::Fired;

thread_local! {
    /// Set while the core itself runs the rail's apply, for the person's press.
    static RAIL_APPLYING: Cell<bool> = const { Cell::new(false) };
}

/// Whether the intent being dispatched on this thread is the rail's apply, which the person
/// pressed on their own window. Only the core sets it, around its own call into the registry: a
/// caller over the socket cannot, whatever token it presents. An app's agent-only rules (which
/// files an agent may be shown) are for the agents that call over the socket, not for the
/// person's own button, which acts on the person's own document.
pub fn is_person_applying() -> bool {
    RAIL_APPLYING.with(Cell::get)
}

struct PersonApplying;

impl PersonApplying {
    fn enter() -> Self {
        RAIL_APPLYING.with(|c| c.set(true));
        PersonApplying
    }
}

impl Drop for PersonApplying {
    fn drop(&mut self) {
        RAIL_APPLYING.with(|c| c.set(false));
    }
}

impl<A: App> Core<A> {
    /// The surface this app serves, for what the core itself raises through it (the rail).
    pub fn attach_registry(&self, registry: Rc<LocalRegistry>) {
        *self.registry.borrow_mut() = Some(registry);
    }

    /// One of the app's intents, run as an agent through the admitted path: the surface's state
    /// rule, the arguments, the ceiling and the mode in `authority`, the app's agent rules. The
    /// reply is not waited for (an intent that settles later settles on its own); what is
    /// returned is whether the call was admitted.
    pub(super) fn act_as_agent(
        &self,
        apply: rail::Apply,
        authority: yantrik_ipc_transport::gate::Authority,
    ) -> Result<(), String> {
        let registry = self.registry.borrow().clone();
        let Some(registry) = registry else {
            return Err("this app published no control surface".into());
        };
        let _agent = AgentTokenScope::enter(Some(rail::AGENT_TOKEN.into()));
        let _who = CallerScope::enter(None);
        let later = LaterScope::enter();
        let _person = PersonApplying::enter();
        let admitted = registry
            .act(&apply.intent, &apply.args, None, "rail", &authority)
            .map(|_| ());
        // An explicit settlement's wait is the RPC thread's business; here nobody waits on it.
        drop(later.take());
        admitted
    }

    /// The rail refreshes after a command changed something other than the rail itself,
    /// debounced; an app without a rail arms nothing.
    pub(super) fn rail_changed(&self, changed: &Changed) {
        let outside_rail = match changed {
            Changed::All => true,
            Changed::Parts(parts) => parts.iter().any(|p| *p != rail::PART),
        };
        if !outside_rail {
            return;
        }
        if let Some(refresh) = A::rail(RailCommand::Refresh) {
            let fire = Box::new(move || Fired::Command(refresh.clone()));
            self.timers.borrow_mut().after(rail::DEBOUNCE, rail::DEBOUNCE_FOR, fire);
        }
    }
}
