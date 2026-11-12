//! Who may attach under the first-party Mind's id (#447, #1176).

use yantrik_ipc_transport::PeerCred;

/// Why an attach under the first-party Yantrik Mind's id is refused, or `None` to let it through.
///
/// The id is only a name the harness gives itself, and it is the one the person's memory grants
/// treat as the person's own Mind (#447). Where the mind door is served, the real Mind runs as the
/// mind account, in its own unit, so a process that is not both (`PeerCred::the_mind`, decided at accept) and calls itself
/// `mind` is an impostor. The uid alone is not enough: another harness (Hermes) shares the account
/// (#1076), and under the bare id it would replace the live Mind. Refused, it cannot take the name at all, nor replace the real
/// Mind's session by re-attaching under it. A caller the kernel could not name (the TCP dev path)
/// cannot show it is the account either.
///
/// Where no door is served, as on an install the account migration has not reached, the Mind
/// runs as a unit of the person's own and attaches with the person's uid, so the name is let
/// through. It earns nothing by it: the first-party memory defaults still go only to the mind
/// account, which is decided from the uid kept at attach.
pub(super) fn first_party_claim_refused(
    method: &str,
    params: &serde_json::Value,
    peer: Option<PeerCred>,
    door_served: bool,
) -> Option<String> {
    if method != yantrik_harness::protocol::ATTACH
        || params["id"].as_str() != Some(crate::memory_grants::FIRST_PARTY_MIND)
    {
        return None;
    }
    // The Mind never attaches shared, so a shared `mind` is always an impostor, whatever the uid
    // and whether or not the door is served.
    if params["shared"].as_bool() == Some(true) {
        return Some(format!(
            "`{}` is the id of the person's own Yantrik Mind, which never attaches shared; attach under a different id",
            crate::memory_grants::FIRST_PARTY_MIND
        ));
    }
    if !door_served || peer.is_some_and(|p| p.the_mind) {
        return None;
    }
    Some(format!(
        "`{}` is the id of the person's own Yantrik Mind, which attaches from its own account; attach under a different id",
        crate::memory_grants::FIRST_PARTY_MIND
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use yantrik_harness::protocol;

    /// The mind account's uid is 990. Pid 4242 is the Mind, in its unit; 5151 is a Hermes that
    /// shares the account (#1076) but runs in its own unit: it was not the Mind at accept.
    fn peer(pid: i32, uid: u32) -> Option<PeerCred> {
        Some(PeerCred { pid, uid, gid: uid, the_mind: uid == 990 && pid == 4242 })
    }

    fn attach(id: &str) -> serde_json::Value {
        serde_json::json!({ "id": id, "name": "Yantrik Mind" })
    }

    fn refused(params: &serde_json::Value, peer: Option<PeerCred>, door: bool) -> Option<String> {
        first_party_claim_refused(protocol::ATTACH, params, peer, door)
    }

    /// #447: the first-party Mind's id is the Mind's alone wherever the door is served.
    #[test]
    fn only_the_mind_attaches_as_the_first_party_mind_where_the_door_is_served() {
        // The door is served: the person's own processes, and a caller nobody can name, are
        // refused the name; the Mind is not.
        let why = refused(&attach("mind"), peer(300, 1000), true).expect("a person's process is refused");
        assert!(why.contains("`mind`") && why.contains("different id"), "{why}");
        assert!(refused(&attach("mind"), None, true).is_some(), "an unnamed caller cannot show it is the Mind");
        assert_eq!(refused(&attach("mind"), peer(4242, 990), true), None);

        // Any other id is anybody's, and any other method is not an attach.
        assert_eq!(refused(&attach("pi"), peer(300, 1000), true), None);
        assert_eq!(refused(&attach("Mind"), peer(300, 1000), true), None, "another name, and no defaults with it");
        let poll = first_party_claim_refused(protocol::POLL, &attach("mind"), peer(300, 1000), true);
        assert_eq!(poll, None);

        // No door, as before the account migration: the Mind runs as the person and attaches so.
        assert_eq!(refused(&attach("mind"), peer(300, 1000), false), None);

        // A shared `mind` is always an impostor: the Mind never attaches shared, so it is refused
        // from the person's uid and from the Mind, with the door served and not served.
        let shared = serde_json::json!({ "id": "mind", "name": "Yantrik Mind", "shared": true });
        for (who, door) in [(peer(300, 1000), true), (peer(4242, 990), true), (peer(300, 1000), false), (peer(4242, 990), false)] {
            let why = refused(&shared, who, door).expect("a shared mind is refused");
            assert!(why.contains("never attaches shared"), "{why}");
        }
    }

    /// #1176: after #1076 a moved Hermes runs under the mind uid. It is a mind-uid peer outside the
    /// Mind's unit, and it cannot take the name `mind`, nor replace the live Mind under it.
    #[test]
    fn a_mind_uid_peer_outside_the_minds_unit_is_refused_the_name_mind() {
        let why = refused(&attach("mind"), peer(5151, 990), true).expect("a Hermes under the mind uid is refused");
        assert!(why.contains("`mind`") && why.contains("different id"), "{why}");
        assert_eq!(refused(&attach("hermes"), peer(5151, 990), true), None, "its own id is still its own");
    }
}
