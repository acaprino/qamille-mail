mod support;

use std::net::IpAddr;

use qamille_server::ServerError;
use support::{TestServer, ip, loopback, test_config, user_with_device};

const PASSWORD: &str = "una password lunga";
const DEVICE: &str = "Qamille su Windows";

fn origin() -> IpAddr {
    ip(loopback())
}

fn other_origin(n: u8) -> IpAddr {
    IpAddr::from([203, 0, 113, n])
}

#[tokio::test]
async fn temporary_password_then_change_then_login() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let created = accounts.create_user("mario", None, None).await.unwrap();

    let err = accounts
        .login(origin(), "mario", &created.temporary_password, DEVICE)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ServerError::PasswordChangeRequired),
        "{err:?}"
    );
    assert!(accounts.list_devices(&created.id).await.unwrap().is_empty());

    let first = accounts
        .change_temporary_password(
            origin(),
            "mario",
            &created.temporary_password,
            PASSWORD,
            DEVICE,
        )
        .await
        .unwrap();
    assert_eq!(first.user_id, created.id);
    assert_eq!(first.username, "mario");
    assert!(first.device_key.starts_with("spk_dev_"));
    assert_eq!(first.device_key.len(), 8 + 64);

    // La provvisoria non vale più.
    let err = accounts
        .login(origin(), "mario", &created.temporary_password, DEVICE)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidCredentials), "{err:?}");

    let second = accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();
    assert_ne!(first.device_id, second.device_id);
    assert_eq!(accounts.list_devices(&created.id).await.unwrap().len(), 2);

    // Nel database c'è solo lo SHA-256 della chiave.
    let key = second.device_key.clone();
    let stored: Vec<String> = accounts
        .db()
        .blocking(|conn| {
            let mut stmt = conn.prepare("SELECT key_sha256 FROM device_keys")?;
            Ok(stmt
                .query_map([], |r| r.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?)
        })
        .await
        .unwrap();
    assert!(
        stored
            .iter()
            .all(|h| h.len() == 64 && !h.contains("spk_dev_"))
    );
    assert!(!stored.contains(&key));
}

#[tokio::test]
async fn new_password_rules_apply_to_the_change() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let created = accounts.create_user("mario", None, None).await.unwrap();
    let err = accounts
        .change_temporary_password(
            origin(),
            "mario",
            &created.temporary_password,
            "corta",
            DEVICE,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(&err, ServerError::InvalidRequest(m) if m == "La password deve avere almeno 10 caratteri."),
        "{err:?}"
    );
    let err = accounts
        .change_temporary_password(
            origin(),
            "mario",
            &created.temporary_password,
            &created.temporary_password,
            DEVICE,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidRequest(_)), "{err:?}");
}

#[tokio::test]
async fn expired_temporary_password_behaves_like_a_wrong_password() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let created = accounts.create_user("mario", None, None).await.unwrap();
    ts.clock.advance(chrono::TimeDelta::days(7));

    let err = accounts
        .login(origin(), "mario", &created.temporary_password, DEVICE)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidCredentials), "{err:?}");
    let err = accounts
        .change_temporary_password(
            origin(),
            "mario",
            &created.temporary_password,
            PASSWORD,
            DEVICE,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidCredentials), "{err:?}");
}

#[tokio::test]
async fn identifiers_ignore_case_and_surrounding_spaces() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let created = accounts
        .create_user("Mario", Some("Mario.Rossi@Example.com"), None)
        .await
        .unwrap();
    accounts
        .change_temporary_password(
            origin(),
            "  MARIO ",
            &created.temporary_password,
            PASSWORD,
            DEVICE,
        )
        .await
        .unwrap();
    let by_name = accounts
        .login(origin(), " mario  ", PASSWORD, DEVICE)
        .await
        .unwrap();
    assert_eq!(by_name.username, "Mario");
    let by_email = accounts
        .login(origin(), "  mario.rossi@EXAMPLE.COM", PASSWORD, DEVICE)
        .await
        .unwrap();
    assert_eq!(by_email.user_id, created.id);
}

#[tokio::test]
async fn duplicate_usernames_and_emails_are_rejected() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    accounts
        .create_user("mario", Some("mario@example.com"), None)
        .await
        .unwrap();

    let err = accounts.create_user("MARIO", None, None).await.unwrap_err();
    assert!(matches!(err, ServerError::UsernameTaken), "{err:?}");
    let err = accounts
        .create_user("luigi", Some(" Mario@Example.COM "), None)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::EmailTaken), "{err:?}");
    let err = accounts
        .create_user("luigi@example.com", None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidRequest(_)), "{err:?}");
    let err = accounts
        .create_user("luigi", Some("non-email"), None)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidRequest(_)), "{err:?}");
    // Due utenti senza email convivono.
    accounts.create_user("luigi", None, None).await.unwrap();
    accounts.create_user("anna", None, None).await.unwrap();
}

#[tokio::test]
async fn the_five_causes_of_invalid_credentials_are_indistinguishable() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();

    // Disattivato (con la password giusta).
    let disabled = user_with_device(&ts, "disabled", PASSWORD).await;
    accounts.disable_user(&disabled.user_id).await.unwrap();
    // Senza password.
    let no_password = accounts
        .create_user("nopassword", None, None)
        .await
        .unwrap();
    let id = no_password.id.clone();
    accounts
        .db()
        .blocking(move |conn| {
            conn.execute("UPDATE users SET password_hash = NULL WHERE id = ?1", [&id])?;
            Ok(())
        })
        .await
        .unwrap();
    // Provvisoria scaduta.
    let expired = accounts.create_user("expired", None, None).await.unwrap();
    ts.clock.advance(chrono::TimeDelta::days(8));
    // Password sbagliata.
    user_with_device(&ts, "wrong", PASSWORD).await;

    let cases: [(&str, &str); 5] = [
        ("unknown", PASSWORD),
        ("disabled", PASSWORD),
        ("nopassword", PASSWORD),
        ("expired", &expired.temporary_password),
        ("wrong", "un'altra password"),
    ];
    let mut messages = Vec::new();
    for (n, (identifier, password)) in cases.iter().enumerate() {
        let before = accounts.argon2_verifications();
        let err = accounts
            .login(other_origin(n as u8 + 1), identifier, password, DEVICE)
            .await
            .unwrap_err();
        assert_eq!(
            accounts.argon2_verifications(),
            before + 1,
            "{identifier}: una verifica argon2"
        );
        assert!(
            matches!(err, ServerError::InvalidCredentials),
            "{identifier}: {err:?}"
        );
        messages.push((
            err.status(),
            err.code(),
            err.to_string(),
            err.retry_after_s(),
        ));
    }
    assert!(messages.windows(2).all(|w| w[0] == w[1]), "{messages:?}");
    assert_eq!(messages[0].2, "Username o password non corretti.");
}

#[tokio::test(start_paused = true)]
async fn twenty_wrong_passwords_then_the_right_one_gets_in_within_four_seconds() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    user_with_device(&ts, "mario", PASSWORD).await;
    for _ in 0..20 {
        let err = accounts
            .login(origin(), "mario", "password sbagliata", DEVICE)
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::InvalidCredentials), "{err:?}");
    }
    let started = tokio::time::Instant::now();
    accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();
    assert!(
        started.elapsed() <= std::time::Duration::from_millis(4100),
        "{:?}",
        started.elapsed()
    );
    let key = accounts.throttle_key_for("mario").await.unwrap();
    assert_eq!(
        accounts
            .throttle()
            .failures(&key, tokio::time::Instant::now()),
        0
    );
}

#[tokio::test]
async fn origin_limit_answers_before_argon2() {
    let mut config = test_config();
    config.login_max_failures_per_origin = 2;
    let ts = TestServer::new(config);
    let accounts = ts.accounts();
    user_with_device(&ts, "mario", PASSWORD).await;
    let attacker = other_origin(9);
    for _ in 0..2 {
        let err = accounts
            .login(attacker, "mario", "sbagliata!!", DEVICE)
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::InvalidCredentials));
    }
    let before = accounts.argon2_verifications();
    let err = accounts
        .login(attacker, "mario", PASSWORD, DEVICE)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ServerError::RateLimited { retry_after_s } if retry_after_s > 0),
        "{err:?}"
    );
    assert_eq!(accounts.argon2_verifications(), before);
    // Un'altra origine entra.
    accounts
        .login(other_origin(10), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();
}

#[tokio::test(start_paused = true)]
async fn saturated_argon2_answers_server_busy() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    user_with_device(&ts, "mario", PASSWORD).await;
    let permits = accounts
        .argon2_semaphore()
        .clone()
        .acquire_many_owned(4)
        .await
        .unwrap();
    let err = accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap_err();
    assert!(
        matches!(err, ServerError::ServerBusy { retry_after_s: 2 }),
        "{err:?}"
    );
    drop(permits);
    accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();
}

#[tokio::test]
async fn device_limit_then_disable_one_with_the_password() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let created = accounts.create_user("mario", None, Some(2)).await.unwrap();
    let first = accounts
        .change_temporary_password(
            origin(),
            "mario",
            &created.temporary_password,
            PASSWORD,
            DEVICE,
        )
        .await
        .unwrap();
    accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();

    let err = accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap_err();
    let ServerError::DeviceLimit(devices) = err else {
        panic!("atteso device_limit, ricevuto {err:?}");
    };
    assert_eq!(devices.len(), 2);
    assert!(devices.iter().any(|d| d.id == first.device_id));

    // Password sbagliata: non disattiva nulla.
    let err = accounts
        .disable_device_with_password(origin(), "mario", "sbagliata!!", &first.device_id)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidCredentials));
    accounts
        .disable_device_with_password(origin(), "mario", PASSWORD, &first.device_id)
        .await
        .unwrap();
    assert!(accounts.authenticate(&first.device_key).await.is_err());
    accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();

    // Dispositivo inesistente o già disattivato.
    let err = accounts
        .disable_device_with_password(origin(), "mario", PASSWORD, &first.device_id)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::NotFound(_)));
}

#[tokio::test]
async fn an_admin_reset_revokes_every_device_and_key_of_the_user() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let created = accounts.create_user("mario", None, Some(2)).await.unwrap();
    let first = accounts
        .change_temporary_password(origin(), "mario", &created.temporary_password, PASSWORD, DEVICE)
        .await
        .unwrap();
    let second = accounts.login(origin(), "mario", PASSWORD, DEVICE).await.unwrap();
    let other = user_with_device(&ts, "anna", PASSWORD).await;

    let reset = accounts.reset_password(&created.id).await.unwrap();

    // Le chiavi di prima non valgono più, quelle degli altri utenti sì.
    for key in [&first.device_key, &second.device_key] {
        assert!(matches!(accounts.authenticate(key).await, Err(ServerError::Unauthorized)));
    }
    accounts.authenticate(&other.device_key).await.unwrap();
    assert!(accounts.list_devices(&created.id).await.unwrap().is_empty());

    // La provvisoria chiede il cambio e registra un dispositivo nuovo.
    let err = accounts
        .login(origin(), "mario", &reset.temporary_password, DEVICE)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::PasswordChangeRequired), "{err:?}");
    let third = accounts
        .change_temporary_password(origin(), "mario", &reset.temporary_password, "nuova password 1", DEVICE)
        .await
        .unwrap();
    accounts.authenticate(&third.device_key).await.unwrap();
}


#[tokio::test]
async fn two_concurrent_logins_for_the_last_slot() {
    let ts = TestServer::standard();
    let accounts = ts.accounts().clone();
    let created = accounts.create_user("mario", None, Some(2)).await.unwrap();
    accounts
        .change_temporary_password(
            origin(),
            "mario",
            &created.temporary_password,
            PASSWORD,
            DEVICE,
        )
        .await
        .unwrap();
    let a = {
        let accounts = accounts.clone();
        tokio::spawn(async move {
            accounts
                .login(other_origin(1), "mario", PASSWORD, DEVICE)
                .await
        })
    };
    let b = {
        let accounts = accounts.clone();
        tokio::spawn(async move {
            accounts
                .login(other_origin(2), "mario", PASSWORD, DEVICE)
                .await
        })
    };
    let results = [a.await.unwrap(), b.await.unwrap()];
    let ok = results.iter().filter(|r| r.is_ok()).count();
    let limited = results
        .iter()
        .filter(|r| matches!(r, Err(ServerError::DeviceLimit(_))))
        .count();
    assert_eq!((ok, limited), (1, 1), "{results:?}");
    assert_eq!(accounts.list_devices(&created.id).await.unwrap().len(), 2);
}

#[tokio::test]
async fn unlimited_devices_with_zero() {
    let mut config = test_config();
    config.default_max_devices = 0;
    let ts = TestServer::new(config);
    let accounts = ts.accounts();
    user_with_device(&ts, "mario", PASSWORD).await;
    for _ in 0..5 {
        accounts
            .login(origin(), "mario", PASSWORD, DEVICE)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn revocations_and_disabling_close_authenticate() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let b = accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();
    let c = accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();

    let auth_a = accounts.authenticate(&a.device_key).await.unwrap();
    assert_eq!(auth_a.user_id, a.user_id);
    assert_eq!(auth_a.device_id, a.device_id);

    // Un altro dispositivo dello stesso utente.
    accounts.revoke_device(&auth_a, &b.device_id).await.unwrap();
    assert!(matches!(
        accounts.authenticate(&b.device_key).await,
        Err(ServerError::Unauthorized)
    ));

    // L'amministratore.
    accounts
        .revoke_device_admin(&a.user_id, &c.device_id)
        .await
        .unwrap();
    assert!(matches!(
        accounts.authenticate(&c.device_key).await,
        Err(ServerError::Unauthorized)
    ));

    // Utente disattivato: i suoi dispositivi si revocano, riattivarlo non li fa rivivere.
    accounts.disable_user(&a.user_id).await.unwrap();
    assert!(matches!(
        accounts.authenticate(&a.device_key).await,
        Err(ServerError::Unauthorized)
    ));
    let err = accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidCredentials));
    accounts.enable_user(&a.user_id).await.unwrap();
    assert!(matches!(
        accounts.authenticate(&a.device_key).await,
        Err(ServerError::Unauthorized)
    ));
    let again = accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();
    accounts.authenticate(&again.device_key).await.unwrap();
    let auth_a = accounts.authenticate(&again.device_key).await.unwrap();

    // Uscita da questo dispositivo.
    accounts.revoke_own_device(&auth_a).await.unwrap();
    assert!(matches!(
        accounts.authenticate(&again.device_key).await,
        Err(ServerError::Unauthorized)
    ));

    // Chiavi malformate o sconosciute.
    for key in [
        "",
        "spk_dev_",
        "spk_dev_zz",
        &format!("spk_dev_{}", "0".repeat(64)),
    ] {
        assert!(matches!(
            accounts.authenticate(key).await,
            Err(ServerError::Unauthorized)
        ));
    }
}

#[tokio::test]
async fn last_seen_is_updated_at_most_every_five_minutes() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let seen = |ts: &TestServer| {
        let accounts = ts.accounts().clone();
        let user_id = a.user_id.clone();
        async move {
            accounts.list_devices(&user_id).await.unwrap()[0]
                .last_seen_at
                .clone()
        }
    };
    let initial = seen(&ts).await;
    ts.clock.advance(chrono::TimeDelta::minutes(1));
    accounts.authenticate(&a.device_key).await.unwrap();
    assert_eq!(seen(&ts).await, initial);
    ts.clock.advance(chrono::TimeDelta::minutes(5));
    accounts.authenticate(&a.device_key).await.unwrap();
    assert_ne!(seen(&ts).await, initial);
}

#[tokio::test]
async fn ten_wrong_current_passwords_in_a_row_revoke_the_key() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = accounts.authenticate(&a.device_key).await.unwrap();

    // Un successo azzera la serie.
    for n in 0..5 {
        let err = accounts
            .change_password(
                other_origin(n),
                &auth,
                "sbagliata!!",
                "nuova password 1",
                true,
            )
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::InvalidCredentials));
    }
    accounts
        .change_password(origin(), &auth, PASSWORD, "nuova password 1", true)
        .await
        .unwrap();

    for n in 0..9 {
        let _ = accounts
            .change_password(
                other_origin(10 + n),
                &auth,
                "sbagliata!!",
                "nuova password 2",
                true,
            )
            .await;
    }
    accounts.authenticate(&a.device_key).await.unwrap();
    let err = accounts
        .change_password(
            other_origin(20),
            &auth,
            "sbagliata!!",
            "nuova password 2",
            true,
        )
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidCredentials));
    assert!(matches!(
        accounts.authenticate(&a.device_key).await,
        Err(ServerError::Unauthorized)
    ));
    // La password non è cambiata e si può rientrare.
    accounts
        .login(origin(), "mario", "nuova password 1", DEVICE)
        .await
        .unwrap();
}

#[tokio::test]
async fn change_password_signs_out_other_devices_on_request() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let b = accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();
    let auth_a = accounts.authenticate(&a.device_key).await.unwrap();

    accounts
        .change_password(origin(), &auth_a, PASSWORD, "nuova password 1", false)
        .await
        .unwrap();
    accounts.authenticate(&b.device_key).await.unwrap();

    accounts
        .change_password(
            origin(),
            &auth_a,
            "nuova password 1",
            "nuova password 2",
            true,
        )
        .await
        .unwrap();
    assert!(matches!(
        accounts.authenticate(&b.device_key).await,
        Err(ServerError::Unauthorized)
    ));
    accounts.authenticate(&a.device_key).await.unwrap();
    let err = accounts
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidCredentials));
    accounts
        .login(origin(), "mario", "nuova password 2", DEVICE)
        .await
        .unwrap();
}

#[tokio::test]
async fn delete_account_removes_user_devices_and_keys() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = accounts.authenticate(&a.device_key).await.unwrap();

    let err = accounts
        .delete_account(origin(), &auth, "sbagliata!!")
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidCredentials));
    accounts
        .delete_account(origin(), &auth, PASSWORD)
        .await
        .unwrap();

    assert!(accounts.list_users().await.unwrap().is_empty());
    assert!(matches!(
        accounts.authenticate(&a.device_key).await,
        Err(ServerError::Unauthorized)
    ));
    let counts: (i64, i64) = accounts
        .db()
        .blocking(|conn| {
            Ok(conn.query_row(
                "SELECT (SELECT COUNT(*) FROM devices), (SELECT COUNT(*) FROM device_keys)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .await
        .unwrap();
    assert_eq!(counts, (0, 0));
    // Lo username torna libero.
    accounts.create_user("mario", None, None).await.unwrap();
}

#[tokio::test]
async fn admin_operations() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let a = user_with_device(&ts, "mario", PASSWORD).await;

    accounts.set_max_devices(&a.user_id, Some(5)).await.unwrap();
    let users = accounts.list_users().await.unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].max_devices, Some(5));
    assert_eq!(users[0].active_devices, 1);
    assert!(!users[0].password_temporary);
    accounts.set_max_devices(&a.user_id, None).await.unwrap();
    assert_eq!(accounts.list_users().await.unwrap()[0].max_devices, None);

    let reset = accounts.reset_password(&a.user_id).await.unwrap();
    assert!(accounts.list_users().await.unwrap()[0].password_temporary);
    // La vecchia password non vale più, la provvisoria chiede il cambio.
    assert!(matches!(
        accounts.login(origin(), "mario", PASSWORD, DEVICE).await,
        Err(ServerError::InvalidCredentials)
    ));
    assert!(matches!(
        accounts
            .login(origin(), "mario", &reset.temporary_password, DEVICE)
            .await,
        Err(ServerError::PasswordChangeRequired)
    ));

    assert_eq!(
        accounts.user_id_by_username(" MARIO ").await.unwrap(),
        a.user_id
    );
    accounts.delete_user(&a.user_id).await.unwrap();
    assert!(matches!(
        accounts.delete_user(&a.user_id).await,
        Err(ServerError::NotFound(_))
    ));
    assert!(matches!(
        accounts.reset_password(&a.user_id).await,
        Err(ServerError::NotFound(_))
    ));
    assert!(matches!(
        accounts.user_id_by_username("mario").await,
        Err(ServerError::NotFound(_))
    ));
}

// --- Regressioni della revisione di sicurezza della wave 1 ------------------------

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_parallel_burst_does_not_get_past_the_origin_limit() {
    let mut config = test_config();
    config.login_max_failures_per_origin = 3;
    let ts = TestServer::new(config);
    // Un account per tentativo: il tetto per account (due in corso) non interviene, e a
    // fermare la raffica resta il limite per origine.
    for n in 0..12 {
        user_with_device(&ts, &format!("mario{n}"), PASSWORD).await;
    }
    let before = ts.accounts().argon2_verifications();
    let attacker = other_origin(42);
    let tasks: Vec<_> = (0..12)
        .map(|n| {
            let accounts = ts.accounts().clone();
            tokio::spawn(async move {
                accounts
                    .login(attacker, &format!("mario{n}"), "sbagliata!!", DEVICE)
                    .await
                    .unwrap_err()
            })
        })
        .collect();
    let mut wrong = 0;
    let mut limited = 0;
    for task in tasks {
        match task.await.unwrap() {
            ServerError::InvalidCredentials => wrong += 1,
            ServerError::RateLimited { retry_after_s } if retry_after_s > 0 => limited += 1,
            other => panic!("errore inatteso: {other:?}"),
        }
    }
    assert_eq!(wrong, 3, "al più la soglia passa alla verifica");
    assert_eq!(limited, 9);
    assert_eq!(ts.accounts().argon2_verifications() - before, 3);
    assert_eq!(ts.accounts().login_limiter().pending(attacker), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn parallel_attempts_on_one_account_are_capped_at_two() {
    let ts = TestServer::standard();
    user_with_device(&ts, "mario", PASSWORD).await;
    let before = ts.accounts().argon2_verifications();
    let tasks: Vec<_> = (0..6u8)
        .map(|n| {
            let accounts = ts.accounts().clone();
            tokio::spawn(async move {
                accounts
                    .login(other_origin(100 + n), "mario", "sbagliata!!", DEVICE)
                    .await
                    .unwrap_err()
            })
        })
        .collect();
    let mut wrong = 0;
    let mut limited = 0;
    for task in tasks {
        match task.await.unwrap() {
            ServerError::InvalidCredentials => wrong += 1,
            ServerError::RateLimited { retry_after_s } if retry_after_s > 0 => limited += 1,
            other => panic!("errore inatteso: {other:?}"),
        }
    }
    // Da molte origini sullo stesso account: due tentativi in corso alla volta (il tetto per
    // account), gli altri `rate_limited` subito, senza passare da argon2.
    assert_eq!((wrong, limited), (2, 4));
    assert_eq!(ts.accounts().argon2_verifications() - before, 2);
    let key = ts.accounts().throttle_key_for("mario").await.unwrap();
    assert_eq!(ts.accounts().throttle().pending(&key), 0);
    assert_eq!(
        ts.accounts()
            .throttle()
            .failures(&key, tokio::time::Instant::now()),
        2
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_burst_of_wrong_current_passwords_verifies_at_most_ten_times() {
    let ts = TestServer::standard();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = ts.accounts().authenticate(&a.device_key).await.unwrap();
    let before = ts.accounts().argon2_verifications();
    let tasks: Vec<_> = (0..25u8)
        .map(|n| {
            let accounts = ts.accounts().clone();
            let auth = auth.clone();
            tokio::spawn(async move {
                accounts
                    .change_password(
                        other_origin(n),
                        &auth,
                        "sbagliata!!",
                        "nuova password 9",
                        true,
                    )
                    .await
                    .unwrap_err()
            })
        })
        .collect();
    for task in tasks {
        let err = task.await.unwrap();
        assert!(
            matches!(
                err,
                ServerError::InvalidCredentials | ServerError::ServerBusy { .. }
            ),
            "{err:?}"
        );
    }
    assert!(ts.accounts().argon2_verifications() - before <= 10);
    assert!(matches!(
        ts.accounts().authenticate(&a.device_key).await,
        Err(ServerError::Unauthorized)
    ));
    // La password giusta con la chiave revocata non passa e non verifica nulla.
    let before = ts.accounts().argon2_verifications();
    let err = ts
        .accounts()
        .change_password(origin(), &auth, PASSWORD, "nuova password 9", true)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidCredentials), "{err:?}");
    assert_eq!(ts.accounts().argon2_verifications(), before);
    ts.accounts()
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_device_revoked_during_the_verification_blocks_the_change() {
    let ts = TestServer::standard();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = ts.accounts().authenticate(&a.device_key).await.unwrap();
    // Il dispositivo risulta revocato quando la transazione ricontrolla (la chiave no):
    // la password giusta non basta.
    let device_id = a.device_id.clone();
    ts.accounts()
        .db()
        .blocking(move |conn| {
            conn.execute(
                "UPDATE devices SET revoked_at = '2026-11-13T09:00:00.000Z' WHERE id = ?1",
                [&device_id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let err = ts
        .accounts()
        .change_password(origin(), &auth, PASSWORD, "nuova password 7", true)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::Unauthorized), "{err:?}");
    let err = ts
        .accounts()
        .delete_account(origin(), &auth, PASSWORD)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::Unauthorized), "{err:?}");
    // Niente è cambiato.
    ts.accounts()
        .login(origin(), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();
}

#[tokio::test]
async fn username_and_email_share_one_throttle_entry() {
    let ts = TestServer::standard();
    let created = ts
        .accounts()
        .create_user("mario", Some("Mario@Example.com"), None)
        .await
        .unwrap();
    ts.accounts()
        .change_temporary_password(
            origin(),
            "mario",
            &created.temporary_password,
            PASSWORD,
            DEVICE,
        )
        .await
        .unwrap();
    for identifier in ["mario", " MARIO ", "mario@example.com", "MARIO@EXAMPLE.COM"] {
        let err = ts
            .accounts()
            .login(other_origin(7), identifier, "sbagliata!!", DEVICE)
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::InvalidCredentials));
    }
    let by_name = ts.accounts().throttle_key_for("mario").await.unwrap();
    let by_email = ts
        .accounts()
        .throttle_key_for("mario@example.com")
        .await
        .unwrap();
    assert_eq!(by_name, by_email);
    let now = tokio::time::Instant::now();
    assert_eq!(ts.accounts().throttle().failures(&by_name, now), 4);
    // Un identificativo sconosciuto ha la sua voce, con lo stesso lavoro: una verifica.
    let before = ts.accounts().argon2_verifications();
    let _ = ts
        .accounts()
        .login(other_origin(8), "luigi", "sbagliata!!", DEVICE)
        .await;
    assert_eq!(ts.accounts().argon2_verifications() - before, 1);
    let unknown = ts.accounts().throttle_key_for("luigi").await.unwrap();
    assert_ne!(unknown, by_name);
    assert_eq!(ts.accounts().throttle().failures(&unknown, now), 1);
}

// --- Regressioni della revisione di sicurezza della wave 3 ------------------------

async fn streak_of(ts: &TestServer, device_id: &str) -> i64 {
    let device_id = device_id.to_owned();
    ts.accounts()
        .db()
        .blocking(move |conn| {
            Ok(conn.query_row(
                "SELECT wrong_password_streak FROM device_keys WHERE device_id = ?1",
                [&device_id],
                |r| r.get(0),
            )?)
        })
        .await
        .unwrap()
}

#[tokio::test(start_paused = true)]
async fn server_busy_releases_the_streak_slot() {
    let ts = TestServer::standard();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = ts.accounts().authenticate(&a.device_key).await.unwrap();
    let permits = ts
        .accounts()
        .argon2_semaphore()
        .clone()
        .acquire_many_owned(4)
        .await
        .unwrap();
    for _ in 0..12 {
        let err = ts
            .accounts()
            .change_password(origin(), &auth, PASSWORD, "nuova password 3", true)
            .await
            .unwrap_err();
        assert!(matches!(err, ServerError::ServerBusy { .. }), "{err:?}");
    }
    assert_eq!(streak_of(&ts, &a.device_id).await, 0);
    drop(permits);
    // Dodici «server occupato» non hanno revocato la chiave.
    ts.accounts()
        .change_password(origin(), &auth, PASSWORD, "nuova password 3", true)
        .await
        .unwrap();
    assert_eq!(streak_of(&ts, &a.device_id).await, 0);
}

#[tokio::test]
async fn an_interrupted_request_releases_the_streak_slot() {
    let ts = TestServer::standard();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = ts.accounts().authenticate(&a.device_key).await.unwrap();
    let permits = ts
        .accounts()
        .argon2_semaphore()
        .clone()
        .acquire_many_owned(4)
        .await
        .unwrap();
    for _ in 0..12 {
        // Il client se ne va mentre la richiesta aspetta argon2.
        let interrupted = tokio::time::timeout(
            std::time::Duration::from_millis(50),
            ts.accounts().delete_account(origin(), &auth, PASSWORD),
        )
        .await;
        assert!(interrupted.is_err());
    }
    drop(permits);
    let mut streak = -1;
    for _ in 0..100 {
        streak = streak_of(&ts, &a.device_id).await;
        if streak == 0 {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(streak, 0);
    ts.accounts().authenticate(&a.device_key).await.unwrap();
}

#[tokio::test]
async fn a_failed_operation_after_the_right_password_releases_the_slot() {
    let ts = TestServer::standard();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = ts.accounts().authenticate(&a.device_key).await.unwrap();
    // Due password sbagliate restano nella serie.
    for _ in 0..2 {
        let _ = ts
            .accounts()
            .change_password(origin(), &auth, "sbagliata!!", "nuova password 4", true)
            .await;
    }
    assert_eq!(streak_of(&ts, &a.device_id).await, 2);
    // La password giusta, ma l'operazione non si applica (dispositivo revocato nel
    // frattempo): il posto preso da questa richiesta si libera.
    let device_id = a.device_id.clone();
    ts.accounts()
        .db()
        .blocking(move |conn| {
            conn.execute(
                "UPDATE devices SET revoked_at = '2026-11-13T09:00:00.000Z' WHERE id = ?1",
                [&device_id],
            )?;
            Ok(())
        })
        .await
        .unwrap();
    let err = ts
        .accounts()
        .change_password(origin(), &auth, PASSWORD, "nuova password 4", true)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::Unauthorized), "{err:?}");
    assert_eq!(streak_of(&ts, &a.device_id).await, 2);
    // Una nuova password non valida si rifiuta prima di prendere un posto.
    let err = ts
        .accounts()
        .change_password(origin(), &auth, PASSWORD, "corta", true)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::InvalidRequest(_)), "{err:?}");
    assert_eq!(streak_of(&ts, &a.device_id).await, 2);
}

#[tokio::test]
async fn disabling_a_user_revokes_devices_and_enabling_does_not_bring_them_back() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let b = user_with_device(&ts, "anna", PASSWORD).await;
    accounts.disable_user(&a.user_id).await.unwrap();
    assert!(accounts.list_devices(&a.user_id).await.unwrap().is_empty());
    accounts.enable_user(&a.user_id).await.unwrap();
    assert!(matches!(
        accounts.authenticate(&a.device_key).await,
        Err(ServerError::Unauthorized)
    ));
    assert!(accounts.list_devices(&a.user_id).await.unwrap().is_empty());
    // Gli altri utenti non cambiano.
    accounts.authenticate(&b.device_key).await.unwrap();
    assert!(matches!(
        accounts.disable_user("inesistente").await,
        Err(ServerError::NotFound(_))
    ));
}

#[tokio::test]
async fn a_device_key_unused_for_the_inactivity_period_stops_working() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let b = user_with_device(&ts, "anna", PASSWORD).await;

    // L'uso rinnova il periodo.
    ts.clock.advance(chrono::TimeDelta::days(100));
    accounts.authenticate(&a.device_key).await.unwrap();
    ts.clock.advance(chrono::TimeDelta::days(100));
    accounts.authenticate(&a.device_key).await.unwrap();

    // Anna non l'ha usata da 200 giorni: la chiave non vale più e il dispositivo si revoca.
    assert!(matches!(
        accounts.authenticate(&b.device_key).await,
        Err(ServerError::Unauthorized)
    ));
    assert!(accounts.list_devices(&b.user_id).await.unwrap().is_empty());
    // Si accede di nuovo.
    let again = accounts.login(origin(), "anna", PASSWORD, DEVICE).await.unwrap();
    accounts.authenticate(&again.device_key).await.unwrap();
}

#[tokio::test]
async fn zero_inactive_days_means_a_key_never_expires() {
    let mut config = test_config();
    config.device_inactive_days = 0;
    let ts = TestServer::new(config);
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    ts.clock.advance(chrono::TimeDelta::days(3650));
    ts.accounts().authenticate(&a.device_key).await.unwrap();
}

#[tokio::test]
async fn attempts_on_one_account_from_many_origins_cannot_take_all_the_argon2_slots() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    user_with_device(&ts, "mario", PASSWORD).await;
    user_with_device(&ts, "anna", PASSWORD).await;
    let key = accounts.throttle_key_for("mario").await.unwrap();
    let now = tokio::time::Instant::now();
    // Two verifications of mario already in progress.
    let held = [accounts.throttle().begin(&key, now), accounts.throttle().begin(&key, now)];
    let before = accounts.argon2_verifications();
    let err = accounts
        .login(other_origin(77), "mario", PASSWORD, DEVICE)
        .await
        .unwrap_err();
    assert!(matches!(err, ServerError::RateLimited { .. }), "{err:?}");
    assert_eq!(accounts.argon2_verifications(), before, "refused before argon2");
    // The other accounts still sign in.
    accounts.login(other_origin(78), "anna", PASSWORD, DEVICE).await.unwrap();
    // When the attempts end, mario can sign in again.
    drop(held);
    accounts.login(other_origin(79), "mario", PASSWORD, DEVICE).await.unwrap();
}

#[tokio::test]
async fn a_common_password_is_refused_as_a_new_password() {
    let ts = TestServer::standard();
    let accounts = ts.accounts();
    let created = accounts.create_user("mario", None, None).await.unwrap();
    let err = accounts
        .change_temporary_password(origin(), "mario", &created.temporary_password, "Password123", DEVICE)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "password_too_common", "{err:?}");
    let a = user_with_device(&ts, "anna", PASSWORD).await;
    let auth = accounts.authenticate(&a.device_key).await.unwrap();
    let err = accounts
        .change_password(origin(), &auth, PASSWORD, "qwertyuiop", true)
        .await
        .unwrap_err();
    assert_eq!(err.code(), "password_too_common");
}
