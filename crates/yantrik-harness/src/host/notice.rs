//! The host's half of notices: `harness.notice`, a `{text}` notice with no turn in flight, and
//! what `attach` and `poll` carry for them (design/initiative-inbox-2026-10-16.md).
//!
//! The host keeps no notices. It checks a notice, names who sent it from what it established at
//! attach, and hands it to the shell's [`NoticeDesk`], off its own lock: posting is a socket call
//! to the notifications service, and every harness and the whole UI wait on that lock. The answer
//! a harness gets is the store's, after the store acknowledged.

use std::sync::Arc;

use super::{Host, Peer};
use crate::notice::{self, answer, NoticeDesk, NoticeSend};
use crate::protocol;

/// Who sent a notice, as the host established it.
pub(super) struct Sender {
    pub(super) harness: String,
    pub(super) name: String,
    pub(super) pid: Option<u32>,
    pub(super) uid: Option<u32>,
}

impl Host {
    /// The same host, taking notices and handing them to `desk`. Without one, `attach`
    /// advertises nothing, and a harness that sends a notice anyway is told this desktop keeps
    /// none.
    pub fn with_notice_desk(mut self, desk: Arc<dyn NoticeDesk>) -> Host {
        self.notice_desk = Some(desk);
        self
    }

    /// What `attach` adds to its reply: the feature, and the person's settings.
    pub(super) fn notice_attach_fields(&self) -> Option<(serde_json::Value, serde_json::Value)> {
        let desk = self.notice_desk.as_ref()?;
        Some((serde_json::json!([notice::FEATURE]), desk.initiative()))
    }

    /// `harness.notice`.
    pub(super) fn notice(&self, params: &serde_json::Value, peer: Peer) -> Result<serde_json::Value, String> {
        let sender = {
            let mut state = self.lock();
            self.reap(&mut state);
            let harness = Self::touch(&mut state.attached, params, peer, &*self.descends)?;
            Sender {
                harness: harness.announced.id.clone(),
                name: harness.announced.name.clone(),
                pid: harness.pid,
                uid: harness.uid,
            }
        };
        let Some(desk) = self.notice_desk.clone() else {
            return Ok(answer::refused("this desktop keeps no notices; it did not advertise notice_inbox_v1"));
        };
        let raw = &params["notice"];
        let size = serde_json::to_string(raw).map(|s| s.len()).unwrap_or(usize::MAX);
        if size > protocol::MAX_EVENT_BYTES {
            return Ok(answer::refused(&format!("this notice is {size} bytes; one may be at most {}", protocol::MAX_EVENT_BYTES)));
        }
        // Checked here with the store's own rule, so a malformed notice is refused without a
        // round trip; the store checks it again, because it trusts no caller to have.
        if let Err(why) = notice::check(raw) {
            tracing::info!(harness = %sender.harness, why = %why, "notice refused");
            return Ok(answer::refused(&why));
        }
        Ok(post(&*desk, &sender, raw.clone(), false))
    }

    /// A `{text}` notice that came with no turn in flight (#396, #806): stored as a notice, so it
    /// reaches the person instead of a log line. `None` when there is no desk to store it with,
    /// and the old answer stands.
    pub(super) fn text_notice(&self, sender: Sender, text: &str) -> Option<serde_json::Value> {
        let desk = self.notice_desk.clone()?;
        let posted = post(&*desk, &sender, notice::from_text(text, &sender.name), true);
        Some(if posted["stored"] == true {
            tracing::info!(harness = %sender.harness, notice = %posted["notice_id"], "notice with no turn in flight; stored");
            serde_json::json!({ "kept": true, "stored": true, "notice_id": posted["notice_id"] })
        } else {
            // Not kept: the store could not take it. Said, so a harness that reads more than
            // `kept` can send it again.
            let mut answer = posted;
            answer["kept"] = false.into();
            answer
        })
    }

    /// What a poll by `harness` carries: the person's settings, and its feedback after `ack`.
    pub(super) fn notice_poll_fields(
        &self,
        harness: &str,
        pid: Option<u32>,
        uid: Option<u32>,
        ack: Option<u64>,
    ) -> Option<(serde_json::Value, Vec<serde_json::Value>)> {
        let desk = self.notice_desk.as_ref()?;
        let feedback = match desk.feedback(harness, pid, uid, ack) {
            Ok(items) => items,
            Err(why) => {
                // Not delivered this time, and still held: it comes on a later poll.
                tracing::debug!(harness, why = %why, "notice feedback could not be read");
                Vec::new()
            }
        };
        Some((desk.initiative(), feedback))
    }
}

/// Post on `sender`'s behalf, and answer as the wire says.
fn post(desk: &dyn NoticeDesk, sender: &Sender, notice: serde_json::Value, os_id: bool) -> serde_json::Value {
    let send = NoticeSend {
        harness: sender.harness.clone(),
        name: sender.name.clone(),
        pid: sender.pid,
        uid: sender.uid,
        notice,
        os_id,
    };
    match desk.post(&send) {
        Ok(reply) if reply.get("stored").is_some() => notice::wire_answer(&reply),
        // The store answered with a plain notification: it did not judge this desktop to be the
        // desktop, stripped the notice and stored the rest. Not a card, so not `stored`; and not
        // worth retrying, because the next try is judged the same way.
        Ok(_) => answer::refused("the notification store did not accept this desktop as the desktop; it was kept as a plain notification"),
        Err(why) => {
            tracing::warn!(harness = %sender.harness, why = %why, "the notification store is not answering; notice not stored");
            answer::retry(notice::RETRY_UNAVAILABLE_SECS, &format!("the notification store is not answering: {why}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Chunk, Turn};
    use serde_json::json;
    use std::sync::Mutex;

    /// A stand-in for the notifications service: it keys cards by the harness the HOST names, as
    /// the store does, and can be taken down mid-call.
    #[derive(Default)]
    struct Desk {
        down: Mutex<bool>,
        posted: Mutex<Vec<NoticeSend>>,
        feedback: Mutex<Vec<(String, serde_json::Value)>>,
        acked: Mutex<Vec<(String, u64)>>,
    }

    impl NoticeDesk for Desk {
        fn post(&self, send: &NoticeSend) -> Result<serde_json::Value, String> {
            if *self.down.lock().unwrap() {
                return Err("connection reset by peer".into());
            }
            let mut posted = self.posted.lock().unwrap();
            let id = if send.os_id { format!("os-{}", posted.len() + 1) } else { send.notice["notice_id"].as_str().unwrap().to_string() };
            let duplicate = !send.os_id
                && posted.iter().any(|p| p.harness == send.harness && p.notice["notice_id"] == send.notice["notice_id"]);
            if !duplicate {
                posted.push(send.clone());
            }
            Ok(notice::answer::stored(&id, duplicate, &[]))
        }
        fn initiative(&self) -> serde_json::Value {
            json!({ "quiet": false, "off": ["discovery"], "inbox_only": [] })
        }
        fn feedback(&self, harness: &str, _pid: Option<u32>, _uid: Option<u32>, ack: Option<u64>) -> Result<Vec<serde_json::Value>, String> {
            if let Some(ack) = ack {
                self.acked.lock().unwrap().push((harness.to_string(), ack));
                self.feedback.lock().unwrap().retain(|(h, f)| h != harness || f["seq"].as_u64().unwrap() > ack);
            }
            Ok(self.feedback.lock().unwrap().iter().filter(|(h, _)| h == harness).map(|(_, f)| f.clone()).collect())
        }
    }

    fn host_with(desk: &Arc<Desk>) -> Host {
        Host::new(vec![]).with_notice_desk(desk.clone())
    }

    fn attach(host: &Host, id: &str) -> (String, serde_json::Value) {
        let reply = host.handle(protocol::ATTACH, &json!({ "id": id, "name": format!("{id} mind") })).unwrap();
        (reply["session"].as_str().unwrap().to_string(), reply)
    }

    fn reminder(id: &str) -> serde_json::Value {
        json!({ "notice_id": id, "family": "reminder", "title": "Stretch", "why": "You asked.", "attention": "interrupt" })
    }

    fn send(host: &Host, session: &str, notice: serde_json::Value) -> serde_json::Value {
        host.handle(notice::NOTICE, &json!({ "session": session, "notice": notice })).unwrap()
    }

    #[test]
    fn attach_and_every_poll_carry_the_feature_and_the_persons_settings() {
        let desk = Arc::new(Desk::default());
        let host = host_with(&desk);
        let (session, reply) = attach(&host, "mind");
        assert_eq!(reply["features"], json!(["notice_inbox_v1"]));
        assert_eq!(reply["initiative"]["off"], json!(["discovery"]));
        let polled = host.handle(protocol::POLL, &json!({ "session": session })).unwrap();
        assert_eq!(polled["initiative"]["quiet"], false, "{polled}");
        assert!(polled.get("turn_id").is_none());

        // A desktop that keeps no notices advertises nothing, and says so to a harness that sends one.
        let bare = Host::new(vec![]);
        let (session, reply) = attach(&bare, "mind");
        assert!(reply.get("features").is_none(), "{reply}");
        assert!(bare.handle(protocol::POLL, &json!({ "session": session })).unwrap().get("initiative").is_none());
        let answer = send(&bare, &session, reminder("r1"));
        assert_eq!(answer["stored"], false);
        assert!(answer["refused"].is_string(), "{answer}");
    }

    #[test]
    fn stored_only_once_the_store_acknowledged_and_never_while_it_is_down() {
        let desk = Arc::new(Desk::default());
        let host = host_with(&desk);
        let (session, _) = attach(&host, "mind");
        assert_eq!(send(&host, &session, reminder("r1")), json!({ "stored": true, "notice_id": "r1" }));
        assert_eq!(desk.posted.lock().unwrap().len(), 1);

        // The service dies mid-call: the answer is "try again", never `stored: true`.
        *desk.down.lock().unwrap() = true;
        let answer = send(&host, &session, reminder("r2"));
        assert_eq!(answer["stored"], false, "{answer}");
        assert_eq!(answer["retry_after_secs"], notice::RETRY_UNAVAILABLE_SECS);
        assert!(answer.get("refused").is_none(), "a retry, not a refusal: {answer}");
    }

    #[test]
    fn a_malformed_notice_is_refused_before_it_reaches_the_store() {
        let desk = Arc::new(Desk::default());
        let host = host_with(&desk);
        let (session, _) = attach(&host, "mind");
        for bad in [json!({ "notice_id": "x", "family": "gossip", "title": "t", "why": "w" }), json!({ "notice_id": "x", "family": "reminder", "why": "w" }), json!("text")] {
            let answer = send(&host, &session, bad.clone());
            assert_eq!(answer["stored"], false, "{bad}");
            assert!(answer["refused"].is_string(), "{answer}");
        }
        assert!(desk.posted.lock().unwrap().is_empty());
        // And a session that is not attached is an error, as on every other method.
        assert!(host.handle(notice::NOTICE, &json!({ "session": "s9-nope", "notice": reminder("r") })).is_err());
    }

    #[test]
    fn who_sent_it_is_the_attach_never_the_request() {
        let desk = Arc::new(Desk::default());
        let host = host_with(&desk);
        let (mind, _) = attach(&host, "mind");
        let (hermes, _) = attach(&host, "hermes");
        let mut forged = reminder("n-1");
        forged["harness"] = json!({ "id": "hermes" });
        send(&host, &mind, forged);
        // Dedupe is by (harness, notice_id): the same id from the other harness is its own card.
        assert!(send(&host, &hermes, reminder("n-1")).get("duplicate").is_none());
        assert_eq!(send(&host, &mind, reminder("n-1"))["duplicate"], true);
        let posted = desk.posted.lock().unwrap();
        assert_eq!(posted.iter().map(|p| p.harness.as_str()).collect::<Vec<_>>(), ["mind", "hermes"]);
        assert_eq!(posted[0].name, "mind mind");
    }

    #[test]
    fn a_text_notice_with_no_turn_in_flight_is_stored_and_with_one_is_still_a_status_line() {
        let desk = Arc::new(Desk::default());
        let host = host_with(&desk);
        let (session, _) = attach(&host, "hermes");
        // #806's case: nothing is running.
        let answer = host
            .handle(protocol::EVENT, &json!({ "session": session, "turn_id": 0, "event": { "kind": "notice", "text": "Backup finished\n42 files." } }))
            .unwrap();
        assert_eq!(answer["kept"], true);
        assert_eq!(answer["stored"], true);
        assert!(answer["notice_id"].as_str().unwrap().starts_with("os-"), "{answer}");
        {
            let posted = desk.posted.lock().unwrap();
            assert_eq!(posted.len(), 1);
            assert!(posted[0].os_id);
            assert_eq!(posted[0].notice["title"], "Backup finished");
            assert_eq!(posted[0].notice["why"], "Sent by hermes mind while nothing was running");
        }

        // With a turn in flight it is the agent's status line, unchanged, and nothing is stored.
        let reply = host.send(Turn::new("how is the backup?"));
        let turn = host.handle(protocol::POLL, &json!({ "session": session })).unwrap()["turn_id"].as_u64().unwrap();
        let answer = host
            .handle(protocol::EVENT, &json!({ "session": session, "turn_id": turn, "event": { "kind": "notice", "text": "still copying" } }))
            .unwrap();
        assert_eq!(answer, json!({ "kept": true }));
        assert!(matches!(reply.try_recv(), Ok(Chunk::Event(crate::Event::Notice { text })) if text == "still copying"));
        assert_eq!(desk.posted.lock().unwrap().len(), 1, "a status line is not stored");

        // With the store down, it says it was not kept rather than claiming it was.
        *desk.down.lock().unwrap() = true;
        let answer = host
            .handle(protocol::EVENT, &json!({ "session": session, "turn_id": 0, "event": { "kind": "notice", "text": "done" } }))
            .unwrap();
        assert_eq!((answer["kept"].clone(), answer["stored"].clone()), (json!(false), json!(false)), "{answer}");
        assert!(answer["retry_after_secs"].is_u64());
    }

    #[test]
    fn feedback_reaches_only_the_sender_and_comes_back_until_acked() {
        let desk = Arc::new(Desk::default());
        desk.feedback.lock().unwrap().push(("mind".into(), json!({ "seq": 12, "notice_id": "rem-1", "kind": "not_now" })));
        let host = host_with(&desk);
        let (mind, _) = attach(&host, "mind");
        let (hermes, _) = attach(&host, "hermes");
        let poll = |session: &str, ack: Option<u64>| {
            let mut params = json!({ "session": session });
            if let Some(ack) = ack {
                params["feedback_ack"] = ack.into();
            }
            host.handle(protocol::POLL, &params).unwrap()
        };
        assert!(poll(&hermes, None).get("notice_feedback").is_none(), "another harness sees none of it");
        assert_eq!(poll(&mind, None)["notice_feedback"][0]["seq"], 12);
        assert_eq!(poll(&mind, None)["notice_feedback"][0]["kind"], "not_now", "again, until acknowledged");
        assert!(poll(&mind, Some(12)).get("notice_feedback").is_none());
        assert_eq!(desk.acked.lock().unwrap().as_slice(), [("mind".to_string(), 12)]);
    }
}
