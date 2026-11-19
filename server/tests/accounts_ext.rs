//! Funzioni sugli utenti dentro la transazione di chi chiama, e osservatori degli account.
mod support;

use std::sync::{Arc, Mutex};

use qamille_server::ServerError;
use qamille_server::accounts::{Accounts, NewUser};
use qamille_server::ext::AccountObserver;
use rusqlite::TransactionBehavior;
use support::{TestServer, ip, loopback, test_config, user_with_device};

const NOW: &str = "2026-11-13T09:00:00Z";

async fn count(ts: &TestServer, table: &'static str) -> i64 {
    ts.accounts()
        .db()
        .blocking(move |conn| {
            Ok(conn.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |r| r.get(0))?)
        })
        .await
        .unwrap()
}

#[derive(Default)]
struct Seen {
    deleted: Mutex<Vec<String>>,
    revoked: Mutex<Vec<String>>,
    /// Utenti ancora presenti nel database quando è stato chiamato `user_deleted`.
    users_at_call: Mutex<Vec<i64>>,
    ts_db: Mutex<Option<qamille_server::Db>>,
}

struct Watch(Arc<Seen>);

impl AccountObserver for Watch {
    fn user_deleted(&self, user_id: &str) {
        let db = self.0.ts_db.lock().unwrap().clone().unwrap();
        // Il database è libero (il commit è già avvenuto): la lettura non resta in attesa.
        let users = std::thread::spawn(move || {
            tokio::runtime::Builder::new_current_thread()
                .build()
                .unwrap()
                .block_on(db.blocking(|conn| {
                    Ok(conn.query_row("SELECT COUNT(*) FROM users", [], |r| r.get::<_, i64>(0))?)
                }))
                .unwrap()
        })
        .join()
        .unwrap();
        self.0.users_at_call.lock().unwrap().push(users);
        self.0.deleted.lock().unwrap().push(user_id.to_owned());
    }
    fn device_revoked(&self, device_id: &str) {
        self.0.revoked.lock().unwrap().push(device_id.to_owned());
    }
}

#[tokio::test]
async fn a_rolled_back_transaction_leaves_no_user_and_no_device() {
    let ts = TestServer::new(test_config());
    ts.accounts()
        .db()
        .blocking(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let id = Accounts::insert_user(
                &tx,
                NewUser {
                    username: "giulia",
                    email: Some("giulia@example.com"),
                    password_hash: Some("hash"),
                    created_at: NOW,
                },
            )?;
            Accounts::insert_device(&tx, &id, 0, "PC", NOW)?;
            drop(tx);
            Ok(())
        })
        .await
        .unwrap();
    assert_eq!(
        (
            count(&ts, "users").await,
            count(&ts, "devices").await,
            count(&ts, "device_keys").await
        ),
        (0, 0, 0)
    );
}

#[tokio::test]
async fn a_committed_user_has_the_fields_it_was_given() {
    let ts = TestServer::new(test_config());
    let (id, found) = ts
        .accounts()
        .db()
        .blocking(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let id = Accounts::insert_user(
                &tx,
                NewUser {
                    username: "giulia",
                    email: Some("Giulia@Example.com"),
                    password_hash: None,
                    created_at: NOW,
                },
            )?;
            let found = Accounts::user_by_email(&tx, "giulia@example.com")?;
            let missing = Accounts::user_by_email(&tx, "nessuno@example.com")?;
            assert_eq!(missing, None);
            tx.commit()?;
            Ok((id, found))
        })
        .await
        .unwrap();
    assert_eq!(found, Some((id, false)));
}

#[tokio::test]
async fn set_password_replaces_the_hash_and_clears_the_temporary_state() {
    let ts = TestServer::new(test_config());
    let created = ts.accounts().create_user("mario", None, None).await.unwrap();
    let id = created.id.clone();
    let hash = ts
        .accounts()
        .db()
        .blocking(move |conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            Accounts::set_password(&tx, &id, "nuovo-hash", NOW)?;
            tx.commit()?;
            Ok(conn.query_row(
                "SELECT password_hash || '|' || password_temporary || '|' || COALESCE(temporary_expires_at, 'null') FROM users",
                [],
                |r| r.get::<_, String>(0),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(hash, "nuovo-hash|0|null");
}

#[tokio::test]
async fn a_user_without_password_cannot_log_in_with_one() {
    let ts = TestServer::new(test_config());
    ts.accounts()
        .db()
        .blocking(|conn| {
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            Accounts::insert_user(
                &tx,
                NewUser {
                    username: "giulia",
                    email: None,
                    password_hash: None,
                    created_at: NOW,
                },
            )?;
            tx.commit()?;
            Ok(())
        })
        .await
        .unwrap();
    let result = ts
        .accounts()
        .login(ip(loopback()), "giulia", "qualunque", "PC")
        .await;
    assert!(matches!(result, Err(ServerError::InvalidCredentials)));
}

#[tokio::test]
async fn observers_hear_about_deleted_users_and_revoked_devices_after_the_commit() {
    let ts = TestServer::new(test_config());
    let seen = Arc::new(Seen::default());
    *seen.ts_db.lock().unwrap() = Some(ts.accounts().db().clone());
    ts.accounts().add_observer(Arc::new(Watch(seen.clone())));

    let login = user_with_device(&ts, "mario", "una password lunga").await;
    ts.accounts()
        .revoke_device_admin(&login.user_id, &login.device_id)
        .await
        .unwrap();
    assert_eq!(*seen.revoked.lock().unwrap(), std::slice::from_ref(&login.device_id));

    ts.accounts().delete_user(&login.user_id).await.unwrap();
    assert_eq!(*seen.deleted.lock().unwrap(), std::slice::from_ref(&login.user_id));
    assert_eq!(*seen.users_at_call.lock().unwrap(), [0]);
    assert_eq!(count(&ts, "users").await, 0);
}

#[tokio::test]
async fn observers_are_not_called_when_the_operation_fails() {
    let ts = TestServer::new(test_config());
    let seen = Arc::new(Seen::default());
    ts.accounts().add_observer(Arc::new(Watch(seen.clone())));
    assert!(ts.accounts().delete_user("inesistente").await.is_err());
    assert!(
        ts.accounts()
            .revoke_device_admin("inesistente", "nessuno")
            .await
            .is_err()
    );
    assert!(seen.deleted.lock().unwrap().is_empty());
    assert!(seen.revoked.lock().unwrap().is_empty());
}

#[tokio::test]
async fn revoke_all_devices_revokes_every_active_device_and_key_of_that_user_only() {
    let ts = TestServer::new(test_config());
    let mario = user_with_device(&ts, "mario", "una password lunga").await;
    let luigi = user_with_device(&ts, "luigi", "una password lunga").await;
    let mario_id = mario.user_id.clone();
    let luigi_id = luigi.user_id.clone();
    let (revoked, again, active_left, keys_left) = ts
        .accounts()
        .db()
        .blocking(move |conn| {
            // A second device of Mario, already revoked earlier: not returned, kept as is.
            conn.execute(
                "INSERT INTO devices (id, user_id, name, created_at, revoked_at)
                 VALUES ('old-dev', ?1, 'old', ?2, '2026-01-01T00:00:00Z')",
                rusqlite::params![mario_id, NOW],
            )?;
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            let mut revoked = Accounts::revoke_all_devices(&tx, &mario_id, NOW)?;
            revoked.sort();
            let again = Accounts::revoke_all_devices(&tx, &mario_id, NOW)?;
            tx.commit()?;
            let active_left: i64 = conn.query_row(
                "SELECT COUNT(*) FROM devices WHERE revoked_at IS NULL AND user_id = ?1",
                [&mario_id],
                |r| r.get(0),
            )?;
            let other: i64 = conn.query_row(
                "SELECT COUNT(*) FROM devices WHERE revoked_at IS NULL AND user_id = ?1",
                [&luigi_id],
                |r| r.get(0),
            )?;
            let keys_left: i64 = conn.query_row(
                "SELECT COUNT(*) FROM device_keys WHERE revoked_at IS NULL
                 AND device_id IN (SELECT id FROM devices WHERE user_id = ?1)",
                [&mario_id],
                |r| r.get(0),
            )?;
            assert_eq!(other, 1, "the other user keeps their device");
            let stamp: String = conn.query_row(
                "SELECT revoked_at FROM devices WHERE id = 'old-dev'",
                [],
                |r| r.get(0),
            )?;
            assert_eq!(stamp, "2026-01-01T00:00:00Z", "an old revocation keeps its time");
            Ok((revoked, again, active_left, keys_left))
        })
        .await
        .unwrap();
    assert_eq!(revoked, vec![mario.device_id.clone()]);
    assert!(again.is_empty());
    assert_eq!((active_left, keys_left), (0, 0));
}

async fn observed(ts: &TestServer) -> Arc<Seen> {
    let seen = Arc::new(Seen::default());
    *seen.ts_db.lock().unwrap() = Some(ts.accounts().db().clone());
    ts.accounts().add_observer(Arc::new(Watch(seen.clone())));
    seen
}

#[tokio::test]
async fn an_admin_reset_tells_the_observers_about_every_revoked_device() {
    let ts = TestServer::new(test_config());
    let seen = observed(&ts).await;
    let first = user_with_device(&ts, "mario", "una password lunga").await;
    let second = ts
        .accounts()
        .login(ip(loopback()), "mario", "una password lunga", "Altro PC")
        .await
        .unwrap();
    ts.accounts().reset_password(&first.user_id).await.unwrap();
    let mut revoked = seen.revoked.lock().unwrap().clone();
    revoked.sort();
    let mut expected = vec![first.device_id.clone(), second.device_id.clone()];
    expected.sort();
    assert_eq!(revoked, expected);
}

#[tokio::test]
async fn a_password_change_that_signs_out_the_others_tells_the_observers() {
    let ts = TestServer::new(test_config());
    let seen = observed(&ts).await;
    let first = user_with_device(&ts, "mario", "una password lunga").await;
    let second = ts
        .accounts()
        .login(ip(loopback()), "mario", "una password lunga", "Altro PC")
        .await
        .unwrap();
    let auth = ts.accounts().authenticate(&first.device_key).await.unwrap();
    ts.accounts()
        .change_password(ip(loopback()), &auth, "una password lunga", "nuova password 1", true)
        .await
        .unwrap();
    assert_eq!(*seen.revoked.lock().unwrap(), std::slice::from_ref(&second.device_id));
}

#[tokio::test]
async fn the_tenth_wrong_current_password_tells_the_observers_about_the_revoked_device() {
    let ts = TestServer::new(test_config());
    let seen = observed(&ts).await;
    let first = user_with_device(&ts, "mario", "una password lunga").await;
    let auth = ts.accounts().authenticate(&first.device_key).await.unwrap();
    for n in 0..12u8 {
        let origin = std::net::IpAddr::from([203, 0, 113, n + 1]);
        let _ = ts
            .accounts()
            .change_password(origin, &auth, "sbagliata!!", "nuova password 1", true)
            .await;
    }
    assert!(ts.accounts().authenticate(&first.device_key).await.is_err());
    assert_eq!(*seen.revoked.lock().unwrap(), std::slice::from_ref(&first.device_id));
}

#[tokio::test]
async fn revoking_a_device_of_another_user_leaves_its_keys_alone() {
    let ts = TestServer::new(test_config());
    let mario = user_with_device(&ts, "mario", "una password lunga").await;
    let anna = user_with_device(&ts, "anna", "una password lunga").await;
    let (changed, active) = ts
        .accounts()
        .db()
        .blocking({
            let (user, device) = (anna.user_id.clone(), mario.device_id.clone());
            move |conn| {
                let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
                let changed = Accounts::revoke_device_rows(&tx, &user, &device, NOW)?;
                tx.commit()?;
                let active: i64 = conn.query_row(
                    "SELECT COUNT(*) FROM device_keys WHERE device_id = ?1 AND revoked_at IS NULL",
                    [&device],
                    |r| r.get(0),
                )?;
                Ok((changed, active))
            }
        })
        .await
        .unwrap();
    assert!(!changed);
    assert_eq!(active, 1, "la chiave di mario non si tocca");
    ts.accounts().authenticate(&mario.device_key).await.unwrap();
}

#[tokio::test]
async fn a_deletion_guard_can_stop_both_ways_to_delete_an_account() {
    let ts = TestServer::new(test_config());
    let seen = observed(&ts).await;
    let mario = user_with_device(&ts, "mario", "una password lunga").await;
    let asked = Arc::new(Mutex::new(Vec::<String>::new()));
    let allow = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let (asked, allow) = (asked.clone(), allow.clone());
        ts.accounts().set_deletion_guard(Arc::new(move |user_id: String| {
            let (asked, allow) = (asked.clone(), allow.clone());
            Box::pin(async move {
                asked.lock().unwrap().push(user_id);
                if allow.load(std::sync::atomic::Ordering::SeqCst) {
                    Ok(())
                } else {
                    Err(ServerError::InvalidRequest("abbonamento attivo".to_owned()))
                }
            })
        }));
    }
    // The administrator and the user themselves are both stopped, and nothing is deleted.
    let err = ts.accounts().delete_user(&mario.user_id).await.unwrap_err();
    assert!(matches!(err, ServerError::InvalidRequest(_)), "{err:?}");
    let auth = ts.accounts().authenticate(&mario.device_key).await.unwrap();
    let err = ts
        .accounts()
        .delete_account(ip(loopback()), &auth, "una password lunga")
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidRequest(_)), "{err:?}");
    assert_eq!(count(&ts, "users").await, 1);
    assert!(seen.deleted.lock().unwrap().is_empty(), "no observer heard of a deletion that did not happen");
    // The refusal of the guard leaves the key alive and the wrong-password series untouched.
    ts.accounts().authenticate(&mario.device_key).await.unwrap();
    // The guard is not asked for a user that does not exist.
    assert!(matches!(ts.accounts().delete_user("nessuno").await, Err(ServerError::NotFound(_))));
    assert_eq!(*asked.lock().unwrap(), vec![mario.user_id.clone(), mario.user_id.clone()]);
    // Allowed: the user goes, and the observers hear it.
    allow.store(true, std::sync::atomic::Ordering::SeqCst);
    ts.accounts().delete_account(ip(loopback()), &auth, "una password lunga").await.unwrap();
    assert_eq!(count(&ts, "users").await, 0);
    assert_eq!(*seen.deleted.lock().unwrap(), std::slice::from_ref(&mario.user_id));
}
