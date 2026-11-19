//! Gli eventi di controllo (`target: "audit"`): chi fa che cosa, senza segreti. Un'azione
//! dell'amministratore, un utente creato, cancellato, disattivato, un dispositivo revocato, una
//! password cambiata e un accesso fallito lasciano una riga; nessuna riga porta password,
//! chiavi dei dispositivi, password provvisorie né ciò che la persona ha scritto come
//! identificativo sconosciuto.
mod support;

use std::io::Write;
use std::net::IpAddr;
use std::sync::{Arc, Mutex, OnceLock};

use support::{TestServer, ip, loopback, test_config};

#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<u8>>>);

impl Write for Capture {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Capture {
    type Writer = Capture;
    fn make_writer(&'a self) -> Self::Writer {
        self.clone()
    }
}

impl Capture {
    fn text(&self) -> String {
        String::from_utf8_lossy(&self.0.lock().unwrap()).into_owned()
    }
}

fn capture() -> &'static Capture {
    static CAPTURE: OnceLock<Capture> = OnceLock::new();
    CAPTURE.get_or_init(|| {
        let capture = Capture::default();
        tracing_subscriber::fmt()
            .with_writer(capture.clone())
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .init();
        capture
    })
}

const PASSWORD: &str = "SEGRETO-password-lunga-1";
const NEW_PASSWORD: &str = "SEGRETO-nuova-password-2";
const WRONG: &str = "SEGRETO-sbagliata-3";
const GHOST: &str = "fantasma-segreto";

fn origin(n: u8) -> IpAddr {
    IpAddr::from([203, 0, 113, n])
}

#[tokio::test]
async fn the_audit_events_name_who_and_what_and_never_a_secret() {
    let log = capture();
    let ts = TestServer::new(test_config());
    let accounts = ts.accounts();

    // Created by the administrator, first password chosen by the person.
    let created = accounts.create_user("mario", Some("mario@example.com"), None).await.unwrap();
    let temporary = created.temporary_password.clone();
    let first = accounts
        .change_temporary_password(ip(loopback()), "mario", &temporary, PASSWORD, "PC di Mario")
        .await
        .unwrap();
    let second = accounts.login(ip(loopback()), "mario", PASSWORD, "Portatile").await.unwrap();

    // Failed sign-ins: a known user and a name nobody has.
    assert!(accounts.login(origin(7), "mario", WRONG, "PC").await.is_err());
    assert!(accounts.login(origin(8), GHOST, WRONG, "PC").await.is_err());

    // A wrong current password, then the change of password that signs the other device out.
    let auth = accounts.authenticate(&first.device_key).await.unwrap();
    assert!(accounts.change_password(origin(9), &auth, WRONG, NEW_PASSWORD, true).await.is_err());
    accounts.change_password(ip(loopback()), &auth, PASSWORD, NEW_PASSWORD, true).await.unwrap();

    // The administrator: reset, disable, enable, revoke a device, delete.
    let reset = accounts.reset_password(&created.id).await.unwrap();
    let third = accounts
        .change_temporary_password(ip(loopback()), "mario", &reset.temporary_password, PASSWORD, "Altro PC")
        .await
        .unwrap();
    accounts.revoke_device_admin(&created.id, &third.device_id).await.unwrap();
    accounts.disable_user(&created.id).await.unwrap();
    accounts.enable_user(&created.id).await.unwrap();
    accounts.delete_user(&created.id).await.unwrap();

    let text = log.text();
    let audit: Vec<&str> = text.lines().filter(|line| line.contains("audit")).collect();
    let joined = audit.join("\n");
    for event in [
        "user_created",
        "password_changed",
        "login_failed",
        "password_check_failed",
        "device_revoked",
        "admin_password_reset",
        "user_disabled",
        "user_enabled",
        "user_deleted",
    ] {
        assert!(joined.contains(&format!("event=\"{event}\"")) || joined.contains(&format!("event={event}")), "{event} manca:\n{joined}");
    }
    // Who: the user id (the id of the account, never its name or address) and where from.
    assert!(joined.contains(&created.id), "{joined}");
    assert!(joined.contains("203.0.113.7"), "origin of the failed sign-in:\n{joined}");
    assert!(joined.contains("203.0.113.9"), "origin of the failed password check:\n{joined}");
    // A name nobody has shows as a short hash, never as it was written.
    assert!(joined.contains("identifier_hash"), "{joined}");

    // No secret anywhere in the log: not in the audit lines, not in any other line.
    for secret in [
        temporary.as_str(),
        reset.temporary_password.as_str(),
        first.device_key.as_str(),
        second.device_key.as_str(),
        third.device_key.as_str(),
        "SEGRETO",
        GHOST,
    ] {
        assert!(!text.contains(secret), "il log contiene un segreto ({secret}):\n{text}");
    }
    // The key hashes are not logged either.
    let key_hash = hex::encode(sha2::Sha256::digest(first.device_key.as_bytes()));
    assert!(!text.contains(&key_hash));
    let _ = (second, auth);
}

use sha2::Digest;
