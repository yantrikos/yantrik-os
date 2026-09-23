//! A surface for a process with no UI thread: a service, an adapter, a tool.

use std::sync::Arc;

use serde_json::Value;
use yantrik_ipc_contracts::control_surface::{Action, View};
use yantrik_ipc_contracts::email::ServiceError;
use yantrik_ipc_transport::gate::Authority;
use yantrik_ipc_transport::server::{PeerCred, RpcServer, ServiceHandler};

use crate::call::{finish_later, next_action_id, refusal, service_id_for, ActCall, NO_SUCH_METHOD};
use crate::context::{AgentTokenScope, Caller, CallerScope, LaterScope};
use crate::registry::{Registry, SharedDescriber, SharedHandler};

/// One surface — what it reports, what it can be asked to do — answering `app.describe` and
/// `app.act` on whatever thread the request arrives on.
///
/// Build it with [`Surface::new`], [`Surface::describe`] and [`Surface::action`], then either
/// [`Surface::serve`] it on its own socket, or hold it inside a service's own
/// `ServiceHandler` and hand it the two methods with [`Surface::answer`]. Either way a call meets
/// exactly what a call to an app window meets — the ceiling, the grant, the mode, the argument
/// checks, the revision guard — in the same order and the same words, because it is the same
/// dispatch ([`Registry::act`]).
pub struct Surface {
    service_id: String,
    registry: Registry<SharedDescriber, SharedHandler>,
}

impl Surface {
    /// A surface publishing as `app_id`, on the socket `app-<app_id>.sock` unless
    /// [`Surface::socket_name`] says otherwise.
    pub fn new(app_id: &str) -> Surface {
        Surface { service_id: service_id_for(app_id), registry: Registry::new(app_id) }
    }

    /// The socket this surface answers on is `<name>.sock` — for a service, its own name
    /// (`weather`), which it already serves its other methods on. Also the prefix of every
    /// `action_id` it hands out (`weather#12`).
    pub fn socket_name(mut self, name: &str) -> Surface {
        self.service_id = name.to_string();
        self
    }

    /// What this surface reports when asked. Keep it cheap: it runs on every `describe`, and
    /// around every `act`.
    pub fn describe(mut self, f: impl Fn() -> View + Send + Sync + 'static) -> Surface {
        self.registry.set_describe(Box::new(f));
        self
    }

    /// One thing this surface can be asked to do. `f` gets the arguments, checked against `spec`
    /// and with its defaults filled in; its `Err` is the caller's refusal, word for word.
    pub fn action(
        mut self,
        spec: Action,
        f: impl Fn(&Value) -> Result<Value, String> + Send + Sync + 'static,
    ) -> Surface {
        self.registry.add(spec, Box::new(f));
        self
    }

    /// Hold every act to a rule of this surface's own, asked before anything else about the call
    /// — see [`Registry::hold_with`].
    pub fn hold(mut self, rule: impl Fn(&str) -> Result<(), String> + Send + Sync + 'static) -> Surface {
        self.registry.hold_with(Box::new(rule));
        self
    }

    /// The id this surface publishes.
    pub fn app_id(&self) -> &str {
        self.registry.app_id()
    }

    /// The table underneath: grades, regrades, the live view, the declarations' problems.
    pub fn registry(&self) -> &Registry<SharedDescriber, SharedHandler> {
        &self.registry
    }

    /// Where [`Surface::serve`] binds.
    pub fn address(&self) -> String {
        RpcServer::default_address(&self.service_id)
    }

    /// The reply to `app.describe`.
    pub fn describe_json(&self) -> Value {
        self.registry.describe()
    }

    /// Answer `app.describe` or `app.act`; `None` for any other method, so a service can go on
    /// answering its own:
    ///
    /// ```rust,ignore
    /// fn handle_from(&self, method: &str, params: Value, peer: Option<PeerCred>) -> Result<Value, ServiceError> {
    ///     if let Some(answer) = self.surface.answer(method, &params, peer) {
    ///         return answer;
    ///     }
    ///     match method { "weather.current" => …, _ => … }
    /// }
    /// ```
    ///
    /// `app.act` reads the ceiling and the mode from the files the shell writes, per call,
    /// because a person can change either while this runs.
    pub fn answer(
        &self,
        method: &str,
        params: &Value,
        peer: Option<PeerCred>,
    ) -> Option<Result<Value, ServiceError>> {
        match method {
            "app.describe" => {
                // Handlers that care who is asking read `caller()` inside describe as well.
                let _scope = CallerScope::enter(peer.map(Caller::from));
                Some(Ok(self.registry.describe()))
            }
            "app.act" => Some(self.act(params, peer, Authority::now())),
            _ => None,
        }
    }

    /// `app.act` under `authority` — the ceiling and the mode as the caller read them, which for
    /// the socket is [`Authority::now`] and for a test is whatever it pins.
    ///
    /// The steps a window's dispatch takes, in its order: the call read (and its agent token
    /// lifted off the arguments, and the reach that token carries read); any grant spent — once
    /// the action, the reach, the arguments and the ceiling have passed, and against the
    /// arguments as sent; then the reach ([`Registry::within_reach`]) and [`Registry::act`] —
    /// unknown action, arguments, ceiling, mode, revision guard, the arguments converted to their
    /// declared types, handler, view — with the caller and the token installed for the handler;
    /// then any answer the handler left for later.
    pub fn act(
        &self,
        params: &Value,
        peer: Option<PeerCred>,
        mut authority: Authority,
    ) -> Result<Value, ServiceError> {
        let call = ActCall::parse(params)?;
        let who = peer.map(Caller::from);
        let reach = call.reach()?;
        let action_id = next_action_id(&self.service_id);
        // Before a grant is spent: the action, the agent's reach, the arguments as sent.
        call.spend_grant(&mut authority, self.registry.app_id(), || {
            self.registry
                .within_reach(reach.as_ref(), &call.action)
                .and_then(|()| self.registry.check_call(&call.action, &call.args))
                .map_err(refusal)
        })?;
        call.log(&action_id, &authority, who);

        let ActCall { action, args, agent_token, expect_revision, .. } = call;
        let (answer, later) = {
            let _caller = CallerScope::enter(who);
            let _token = AgentTokenScope::enter(agent_token);
            let later = LaterScope::enter();
            let answer = self.registry.within_reach(reach.as_ref(), &action).and_then(|()| {
                self.registry.act(&action, &args, expect_revision.as_deref(), &action_id, &authority)
            });
            (answer, later.take())
        };
        let envelope = answer.map_err(refusal)?;
        match later {
            Some(work) => finish_later(envelope, work, || Some(self.registry.snapshot())),
            None => Ok(envelope),
        }
    }

    /// Bind this surface's socket and answer on it until the process ends.
    ///
    /// Blocks. Two workers, so a caller waiting on an answer finished later
    /// ([`crate::answer_later`]) steps its worker aside and the other keeps serving everyone else.
    pub fn serve(self) -> std::io::Result<()> {
        let address = self.address();
        let runtime = tokio::runtime::Builder::new_multi_thread().worker_threads(2).enable_all().build()?;
        tracing::info!(
            address = %address,
            app = %self.app_id(),
            actions = self.registry.action_count(),
            "Surface listening (app.describe / app.act)"
        );
        runtime.block_on(RpcServer::new(&address).serve(Arc::new(self)))
    }
}

impl ServiceHandler for Surface {
    fn service_id(&self) -> &str {
        &self.service_id
    }

    fn handle(&self, method: &str, params: Value) -> Result<Value, ServiceError> {
        self.handle_from(method, params, None)
    }

    fn handle_from(
        &self,
        method: &str,
        params: Value,
        peer: Option<PeerCred>,
    ) -> Result<Value, ServiceError> {
        self.answer(method, &params, peer).unwrap_or_else(|| {
            Err(ServiceError {
                code: NO_SUCH_METHOD,
                message: format!("unknown method `{method}`; this app serves app.describe, app.act"),
            })
        })
    }
}
