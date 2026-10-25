//! Changing an appointment: `update_event` and `update_own_event`, on the same terms as the
//! delete. What comes back is read from the store afterwards (`work::update_through`), and the
//! window goes to the day the event is on now and reads it again.

use serde_json::json;
use yantrik_app::Refused;

use super::{read_back, show_date, Fx};
use crate::answers;
use crate::backend::calls;
use crate::command::{Command, Kept, Moved, Work};
use crate::model::{Model, NOTHING, PART_NOTICE};
use crate::notice;
use crate::ownership;
use crate::work;

pub fn update(m: &mut Model, command: Command, fx: &mut Fx) -> Result<(), Refused> {
    match command {
        Command::Update { ask, own, caller } => {
            fx.changed(NOTHING);
            let me = if own { m.machine.who.identify(&caller) } else { None };
            let api = m.machine.api.clone();
            m.writes += 1;
            fx.spawn_or(Command::WorkFailed(Work::Change), move |_| {
                if own {
                    // The record the service kept at creation, read back from the event's own
                    // file — so the check survives this app restarting between the create and
                    // this change, as the delete's does (#201).
                    let event = match calls::get(api.as_ref(), &ask.id) {
                        Ok(event) => event,
                        Err(e) => return Command::Declined(e),
                    };
                    if !ownership::may_change_unasked(event.creator.as_deref(), me.as_deref()) {
                        let why = own_change_refused(&ask.id, event.creator.as_deref(), me.as_deref());
                        return Command::Kept(Box::new(Kept {
                            refused: notice::Refused::Change,
                            title: event.title,
                            who: me,
                            id: ask.id.clone(),
                            why,
                        }));
                    }
                }
                let result = work::update_through(api.as_ref(), &ask);
                Command::Moved(Box::new(Moved { id: ask.id.clone(), result }))
            });
        }
        Command::Moved(moved) => {
            let Moved { id, result } = *moved;
            match result {
                // Answered here, in the command that carries this update's own result
                // (`answers.rs`); the window then goes to the day the event is on now.
                Ok(event) => {
                    m.notice.say("");
                    show_date(m, &event.start);
                    let answer = json!({
                        "id": event.id,
                        "title": event.title,
                        "start": event.start,
                        "end": event.end,
                        "all_day": event.is_all_day,
                        "reminder_minutes": event.reminder_minutes,
                    });
                    answers::settle(m, fx, answer);
                }
                Err(e) => {
                    m.notice.say(format!("Could not change the event {id}: {e}"));
                    fx.settle(Err(Refused::new("not_changed", e)));
                }
            }
            fx.changed(PART_NOTICE);
            read_back(m, fx);
        }
        other => unreachable!("not a change command: {other:?}"),
    }
    Ok(())
}

/// The sentence `update_own_event` refuses with: the rule, and the door that asks a person.
fn own_change_refused(id: &str, creator: Option<&str>, me: Option<&str>) -> String {
    let who = me.unwrap_or("nobody this machine could identify");
    let why = match creator {
        Some(made_by) => format!(
            "{id} was created by {made_by} and this call is {who}: only the caller that created \
             an event may change it without a person being asked"
        ),
        None => format!(
            "{id} has no creator on record — it is older than the record, a person made it in the \
             window, or a sync stored it — and this call is {who}: only the caller that created \
             an event may change it without a person being asked"
        ),
    };
    format!("{why}. Any event changes through `update_event`, which asks first")
}
