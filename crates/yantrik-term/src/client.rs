//! The one door this client uses: the shell's own socket, as the person.
//!
//! Every call is an `app.act` on `app-shell`, and the process answering must be the desktop's own
//! shell (`owner::must_be_the_shell`) before anything is written to it — the same rule `yos` and
//! every app hold the shell's name to.

use std::time::Duration;

use serde_json::{json, Value};
use yantrik_ipc_transport::SyncRpcClient;

pub struct Shell {
    rpc: SyncRpcClient,
}

impl Shell {
    pub fn connect() -> Shell {
        Shell {
            rpc: SyncRpcClient::for_service("app-shell")
                .with_timeout(Duration::from_secs(10))
                .expecting_peer(yantrik_ipc_transport::owner::must_be_the_shell),
        }
    }

    /// An act on the shell; its `result`, or the refusal in the shell's words.
    pub fn act(&self, action: &str, args: Value) -> Result<Value, String> {
        let reply = self
            .rpc
            .call("app.act", json!({ "action": action, "args": args }))
            .map_err(|e| e.message)?;
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }

    /// The chat, from message `since` on (`chat_view`).
    pub fn chat_view(&self, since: Option<usize>) -> Result<Value, String> {
        let args = match since {
            Some(i) => json!({ "since": i }),
            None => json!({}),
        };
        self.act("chat_view", args)
    }
}
