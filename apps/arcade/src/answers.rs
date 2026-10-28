//! The answer each intent is owed, settled in the command where its own work lands.
//!
//! An intent's reply is read from the model when it settles, so each one settles in the command
//! that carries its own result: `m.answer` is written and `fx.settle` asked for in that one
//! command, and nothing can land between the two. The answer is built by the lane from what that
//! command read straight after its own work (`work.rs`), never from whatever the window shows when
//! it lands. A command with no invocation (a button, a verdict) never writes `m.answer`.

use serde_json::Value;
use yantrik_app::effects::Effects;
use yantrik_app::Refused;

use crate::model::Model;
use crate::Arcade;

type Fx = Effects<Arcade>;

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
