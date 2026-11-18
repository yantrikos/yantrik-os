//! The answer each action is owed, built from its own result.
//!
//! An action's reply is read from the model when it settles, so each one settles in the command
//! that carries its own result: `m.answer` is written and `fx.settle` asked for in that one
//! command, and nothing can land between the two. What a picture's name resolved to, what a job
//! made, what a backend change wrote down is read by the worker that did it and told here, about
//! its own target, never about whatever the person has chosen or asked for meanwhile. A command
//! with no invocation (a button) never writes `m.answer`.

use serde_json::Value;
use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use crate::model::Model;
use crate::Studio;

type Fx = Effects<Studio>;

/// Whether the command being applied owes a surface call its answer.
pub fn owed(fx: &Fx) -> bool {
    fx.invocation().is_some()
}

/// Settle the invocation this command runs under with `answer`, if it has one.
pub fn settle(m: &mut Model, fx: &mut Fx, answer: Value) {
    if owed(fx) {
        m.answer = answer;
        fx.settle(Ok(()));
    }
}

/// Refuse the call this command answers, in the sentence the window shows too.
pub fn fail(fx: &mut Fx, sentence: String) {
    if owed(fx) {
        fx.settle(Err(Refused::new("failed", sentence)));
    }
}

/// What the intent being settled answers: what `update` wrote in the command that settled it.
pub fn reply(m: &Model) -> Value {
    m.answer.clone()
}
