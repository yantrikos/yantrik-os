//! Trying a service call again when it got no answer.
//!
//! Only the failure to be heard is retried: the connection, a timeout, a service that is busy or
//! restarting. The transport marks those with its own code. An answer that refuses the call is the
//! service's word, and asking again would get the same.

use std::time::Duration;

use yantrik_ipc_transport::RpcError;

/// The code the client gives a call that got no answer (`SyncRpcClient::call`).
const TRANSPORT_ERROR: i32 = -32000;

/// How far apart the tries are.
pub(crate) const GAP: Duration = Duration::from_secs(1);

/// Run `call`, and again up to `retries` more times, `gap` apart, while it fails to be heard.
/// Blocks: for a worker thread, never the UI's.
pub(crate) fn retrying<T>(retries: u32, gap: Duration, mut call: impl FnMut() -> Result<T, RpcError>) -> Result<T, RpcError> {
    let mut left = retries;
    loop {
        match call() {
            Err(e) if e.code == TRANSPORT_ERROR && left > 0 => {
                left -= 1;
                std::thread::sleep(gap);
            }
            other => return other,
        }
    }
}

/// The failure to be heard, from anything that is not an answer (a service that would not start).
pub(crate) fn unheard(why: impl std::fmt::Display) -> RpcError {
    RpcError { code: TRANSPORT_ERROR, message: why.to_string(), data: None }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused() -> RpcError {
        RpcError { code: -32602, message: "no such notification".into(), data: None }
    }

    #[test]
    fn a_call_that_is_not_heard_is_tried_again_until_it_is() {
        let mut calls = 0;
        let out = retrying(3, Duration::ZERO, || {
            calls += 1;
            if calls < 3 { Err(unheard("timed out")) } else { Ok(calls) }
        });
        assert_eq!(out.unwrap(), 3);
    }

    #[test]
    fn it_gives_up_after_the_retries_and_says_the_last_failure() {
        let mut calls = 0;
        let out: Result<(), _> = retrying(3, Duration::ZERO, || {
            calls += 1;
            Err(unheard(format!("try {calls}")))
        });
        assert_eq!((calls, out.unwrap_err().message.as_str()), (4, "try 4"), "the first try and three more");
    }

    #[test]
    fn an_answer_that_refuses_is_not_asked_again() {
        let mut calls = 0;
        let out: Result<(), _> = retrying(3, Duration::ZERO, || {
            calls += 1;
            Err(refused())
        });
        assert_eq!((calls, out.is_err()), (1, true));
    }
}
