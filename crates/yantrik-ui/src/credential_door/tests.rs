//! The credential door, over a real socket, against the client the mail service uses. Every value
//! here is obviously fake.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

use yantrik_foundation::vault::memory::MemoryVault;
use yantrik_foundation::vault::keep;
use yantrik_service_sdk::credentials::ShellVault;

use super::*;

const SECRET: &str = "fake-mail-secret-not-a-password-0001";

fn anyone(_: Option<PeerCred>) -> Result<(), String> {
    Ok(())
}

fn nobody(_: Option<PeerCred>) -> Result<(), String> {
    Err("not the mail service".into())
}

fn vault_of(vault: &Arc<MemoryVault>) -> Vault {
    let vault: Arc<dyn KeyVault> = vault.clone();
    Arc::new(move || Some(vault.clone()))
}

/// Serve `door` on a socket of its own, and answer a client for it that accepts any answerer.
fn serve_on_a_socket(door: Door) -> ShellVault {
    ShellVault::at(serve_at(door), |_| Ok(()))
}

/// Serve `door` on a socket of its own, and answer the socket's address.
fn serve_at(door: Door) -> String {
    static N: AtomicUsize = AtomicUsize::new(0);
    let dir: PathBuf = std::env::temp_dir().join(format!("yantrik-door-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let address = dir.join("credentials.sock").display().to_string();
    let bound = address.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async move {
            let _ = RpcServer::new(&bound).serve(Arc::new(door)).await;
        });
    });
    let probe = yantrik_ipc_transport::SyncRpcClient::new(&address);
    for _ in 0..100 {
        if probe.ping() {
            break;
        }
        yantrik_ipc_transport::SyncRpcClient::clear_breaker(&address);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    address
}

#[test]
fn a_secret_goes_in_over_the_socket_and_comes_back_the_same() {
    let held = Arc::new(MemoryVault::new());
    let client = serve_on_a_socket(Door { vault: vault_of(&held), caller: anyone });

    assert_eq!(keep(&client, "mail:a-example-org", SECRET), Ok(()), "stored, and read back through the door");
    assert_eq!(held.peek("mail:a-example-org").as_deref(), Some(SECRET), "it is in the shell's vault");
    assert_eq!(client.value("mail:nobody-example-org"), Ok(None), "nothing kept is not a failure");

    assert_eq!(client.remove("mail:a-example-org"), Ok(()));
    assert!(!held.has("mail:a-example-org"), "forgotten in the vault");
    assert_eq!(client.value("mail:a-example-org"), Ok(None));
}

#[test]
fn a_shut_vault_is_said_as_shut_and_nothing_is_stored() {
    let held = Arc::new(MemoryVault::new());
    held.shut(true);
    let client = serve_on_a_socket(Door { vault: vault_of(&held), caller: anyone });

    assert_eq!(client.store("mail:a-example-org", SECRET), Err(VaultError::Locked));
    assert_eq!(client.value("mail:a-example-org"), Err(VaultError::Locked));
    assert_eq!(client.remove("mail:a-example-org"), Err(VaultError::Locked));
    assert!(!held.has("mail:a-example-org"));

    held.shut(false);
    assert_eq!(keep(&client, "mail:a-example-org", SECRET), Ok(()), "the same door works once the vault is open");
}

#[test]
fn a_vault_that_does_not_answer_is_a_failure_and_not_a_shut_vault() {
    let held = Arc::new(MemoryVault::new());
    held.refuse_writes(true);
    let client = serve_on_a_socket(Door { vault: vault_of(&held), caller: anyone });
    assert_eq!(client.store("mail:a-example-org", SECRET), Err(VaultError::Failed));

    // The shell's worker is not up yet: no vault at all.
    let none = serve_on_a_socket(Door { vault: Arc::new(|| None), caller: anyone });
    assert_eq!(none.value("mail:a-example-org"), Err(VaultError::Failed));
}

#[test]
fn the_door_keeps_mail_secrets_and_nothing_else_the_vault_holds() {
    let held = Arc::new(MemoryVault::new());
    held.store("saved:p1", "fake-provider-key-0002").unwrap();
    held.store("groq", "fake-free-tier-key-0003").unwrap();
    let client = serve_on_a_socket(Door { vault: vault_of(&held), caller: anyone });

    for id in ["saved:p1", "groq", "mail:", "", "mail:a\nb"] {
        assert_eq!(client.value(id), Err(VaultError::Failed), "{id:?} is read");
        assert_eq!(client.store(id, SECRET), Err(VaultError::Failed), "{id:?} is written");
        assert_eq!(client.remove(id), Err(VaultError::Failed), "{id:?} is removed");
    }
    assert_eq!(held.peek("saved:p1").as_deref(), Some("fake-provider-key-0002"), "a provider's key was not touched");
    assert_eq!(held.peek("groq").as_deref(), Some("fake-free-tier-key-0003"));
    assert!(!held.has("mail:"), "nothing was written under a bare prefix");
}

#[test]
fn a_caller_that_is_not_the_mail_service_is_told_nothing_and_changes_nothing() {
    let held = Arc::new(MemoryVault::new());
    held.store("mail:a-example-org", SECRET).unwrap();
    let client = serve_on_a_socket(Door { vault: vault_of(&held), caller: nobody });

    assert_eq!(client.value("mail:a-example-org"), Err(VaultError::Failed), "the secret is not handed over");
    assert_eq!(client.store("mail:a-example-org", "fake-other-0004"), Err(VaultError::Failed));
    assert_eq!(client.remove("mail:a-example-org"), Err(VaultError::Failed));
    assert_eq!(held.peek("mail:a-example-org").as_deref(), Some(SECRET), "untouched");
}

#[test]
fn the_real_rule_refuses_a_program_that_is_not_the_mail_service() {
    // This test binary is the caller, and it is not `email-service`.
    let me = PeerCred { pid: std::process::id() as i32, uid: owner::own_euid(), gid: 0 };
    assert!(mail_service(Some(me)).is_err());
    assert!(mail_service(None).is_err(), "a caller the kernel would not name");

    let held = Arc::new(MemoryVault::new());
    held.store("mail:a-example-org", SECRET).unwrap();
    let door = Door { vault: vault_of(&held), caller: mail_service };
    let asked = door.handle_from(method::VALUE, serde_json::json!({ "id": "mail:a-example-org" }), Some(me));
    let refusal = asked.unwrap_err();
    assert_eq!(refusal.code, ERR_REFUSED);
    assert!(!refusal.message.contains(SECRET));
    assert!(door.handle(method::VALUE, serde_json::json!({ "id": "mail:a-example-org" })).is_err(), "no peer, no answer");
}

#[test]
fn the_client_will_not_hand_a_secret_to_an_answerer_that_is_not_the_shell() {
    let held = Arc::new(MemoryVault::new());
    let address = serve_at(Door { vault: vault_of(&held), caller: anyone });
    // The same door, dialled by a client that insists on the real shell: this test binary is not
    // `yantrik-ui`, so the secret is never sent.
    let strict = ShellVault::at(address, yantrik_ipc_transport::owner::must_be_the_shell);
    assert_eq!(strict.store("mail:a-example-org", SECRET), Err(VaultError::Failed));
    assert!(!held.has("mail:a-example-org"), "the secret never reached the vault");
}

/// A secret is never put in a string by this file: not in a log line, an error or a `Debug`.
#[test]
fn no_value_reaches_a_string() {
    let source = include_str!("mod.rs");
    let production = source.split("#[cfg(test)]").next().unwrap();
    for (n, line) in production.lines().enumerate() {
        let code = line.trim_start();
        if code.starts_with("//") {
            continue;
        }
        let prints = ["format!", "tracing::", "println!", "eprintln!", "panic!", "write!", "{:?}", "{value", "{params"];
        if prints.iter().any(|p| code.contains(p)) {
            assert!(
                !code.contains("value") && !code.contains("params"),
                "mod.rs line {}: a secret could reach a string: {code}",
                n + 1
            );
        }
    }
}
