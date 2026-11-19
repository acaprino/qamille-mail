mod support;

use serde_json::{Value, json};
use qamille_server::config::Edition;
use qamille_server::{DbError, Server, StartError};
use support::{
    ADMIN_KEY, TempDir, TestServer, bearer, keys, loopback, remote, request, request_raw, test_config,
    user_with_device,
};

const PASSWORD: &str = "una password lunga";
const DEVICE: &str = "Qamille su Windows";

fn login_body(identifier: &str, password: &str) -> Value {
    json!({ "identifier": identifier, "password": password, "device_name": DEVICE })
}

fn assert_error_shape(body: &Value, code: &str) {
    assert_eq!(keys(body), vec!["error"], "{body}");
    let error = &body["error"];
    let mut expected = vec!["code", "message", "retry_after_s"];
    if code == "device_limit" {
        expected.push("devices");
        expected.sort();
    }
    assert_eq!(keys(error), expected, "{body}");
    assert_eq!(error["code"], code, "{body}");
    assert!(error["message"].as_str().is_some_and(|m| !m.is_empty()));
}

#[tokio::test]
async fn core_health_and_info_in_both_editions() {
    for edition in [Edition::SelfHosted, Edition::Professional] {
        let mut config = test_config();
        config.edition = edition;
        let ts = TestServer::new(config);

        let health = request(&ts.public, "GET", "/health", None, loopback(), &[]).await;
        assert_eq!(health.status, 200);
        assert_eq!(keys(&health.body), vec!["status", "version"]);
        assert_eq!(health.body["status"], "ok");
        assert_eq!(health.headers["cache-control"], "no-store");

        // Anche da un'origine remota in chiaro: nessuna credenziale.
        let info = request(
            &ts.public,
            "GET",
            "/v1/info",
            None,
            remote([203, 0, 113, 1]),
            &[],
        )
        .await;
        assert_eq!(info.status, 200);
        assert_eq!(keys(&info.body), vec!["api", "edition", "version"]);
        assert_eq!(info.body["api"], 1);
        assert_eq!(info.body["edition"], edition.as_str());
        assert_eq!(info.headers["cache-control"], "no-store");
    }
}

#[tokio::test]
async fn newer_database_is_not_opened() {
    let dir = TempDir::new();
    {
        let conn = rusqlite::Connection::open(dir.path().join("server.db")).unwrap();
        conn.pragma_update(None, "user_version", 7).unwrap();
    }
    let mut config = test_config();
    config.data_dir = dir.path().to_path_buf();
    match Server::open(config) {
        Err(StartError::Db(DbError::NewerDatabase { found: 7, .. })) => {}
        Err(other) => panic!("errore inatteso: {other}"),
        Ok(_) => panic!("un database v7 non va aperto"),
    }
}

#[tokio::test]
async fn temporary_password_change_login_and_devices() {
    let ts = TestServer::standard();
    let created = ts
        .accounts()
        .create_user("mario", None, None)
        .await
        .unwrap();

    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", &created.temporary_password)),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 403);
    assert_error_shape(&r.body, "password_change_required");

    let r = request(
        &ts.public,
        "POST",
        "/v1/login/password",
        Some(json!({
            "identifier": "mario",
            "temporary_password": created.temporary_password,
            "new_password": PASSWORD,
            "device_name": DEVICE,
        })),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(
        keys(&r.body),
        vec!["device_id", "device_key", "user_id", "username"]
    );
    assert_eq!(r.headers["cache-control"], "no-store");
    let first_device = r.body["device_id"].as_str().unwrap().to_owned();

    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", PASSWORD)),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let key = r.body["device_key"].as_str().unwrap().to_owned();
    assert!(key.starts_with("spk_dev_") && key.len() == 72);
    let current_device = r.body["device_id"].as_str().unwrap().to_owned();

    let auth = bearer(&key);
    let r = request(
        &ts.public,
        "GET",
        "/v1/account/devices",
        None,
        loopback(),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(keys(&r.body), vec!["devices"]);
    let devices = r.body["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 2);
    for d in devices {
        assert_eq!(
            keys(d),
            vec!["created_at", "current", "id", "last_seen_at", "name"]
        );
        assert_eq!(d["current"], d["id"] == current_device.as_str());
        assert_eq!(d["name"], DEVICE);
    }
    assert!(devices.iter().any(|d| d["id"] == first_device.as_str()));
}

#[tokio::test]
async fn wrong_password_and_unknown_user_give_the_same_response() {
    let ts = TestServer::standard();
    user_with_device(&ts, "mario", PASSWORD).await;
    let wrong = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", "sbagliata!!")),
        loopback(),
        &[],
    )
    .await;
    let unknown = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("nessuno", "sbagliata!!")),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(wrong.status, 401);
    assert_error_shape(&wrong.body, "invalid_credentials");
    assert_eq!(wrong.status, unknown.status);
    assert_eq!(wrong.body, unknown.body);
    assert_eq!(
        wrong.body["error"]["message"],
        "Username o password non corretti."
    );
    assert_eq!(wrong.body["error"]["retry_after_s"], Value::Null);
}

#[tokio::test]
async fn device_limit_lists_devices_and_disable_device_frees_a_slot() {
    let ts = TestServer::standard();
    let created = ts
        .accounts()
        .create_user("mario", None, Some(1))
        .await
        .unwrap();
    let first = ts
        .accounts()
        .change_temporary_password(
            support::ip(loopback()),
            "mario",
            &created.temporary_password,
            PASSWORD,
            DEVICE,
        )
        .await
        .unwrap();

    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", PASSWORD)),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 409);
    assert_error_shape(&r.body, "device_limit");
    let devices = r.body["error"]["devices"].as_array().unwrap();
    assert_eq!(devices.len(), 1);
    assert_eq!(
        keys(&devices[0]),
        vec!["created_at", "id", "last_seen_at", "name"]
    );
    assert!(!r.text.contains("spk_dev_"));

    let body = json!({ "identifier": "mario", "password": PASSWORD, "device_id": first.device_id });
    let r = request(
        &ts.public,
        "POST",
        "/v1/login/disable-device",
        Some(body.clone()),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 204);
    let r = request(
        &ts.public,
        "POST",
        "/v1/login/disable-device",
        Some(body),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 404);
    assert_error_shape(&r.body, "not_found");

    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", PASSWORD)),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 200);
}

#[tokio::test]
async fn rate_limited_without_argon2() {
    let mut config = test_config();
    config.login_max_failures_per_origin = 2;
    let ts = TestServer::new(config);
    user_with_device(&ts, "mario", PASSWORD).await;
    for _ in 0..2 {
        let r = request(
            &ts.public,
            "POST",
            "/v1/login",
            Some(login_body("mario", "sbagliata!!")),
            loopback(),
            &[],
        )
        .await;
        assert_eq!(r.status, 401);
    }
    let before = ts.accounts().argon2_verifications();
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", PASSWORD)),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 429);
    assert_error_shape(&r.body, "rate_limited");
    let retry = r.body["error"]["retry_after_s"].as_u64().unwrap();
    assert!(retry > 0 && retry <= 600);
    assert_eq!(r.headers["retry-after"], retry.to_string().as_str());
    assert_eq!(ts.accounts().argon2_verifications(), before);
}

#[tokio::test]
async fn server_busy_when_argon2_is_saturated() {
    let ts = TestServer::standard();
    user_with_device(&ts, "mario", PASSWORD).await;
    let permits = ts
        .accounts()
        .argon2_semaphore()
        .clone()
        .acquire_many_owned(4)
        .await
        .unwrap();
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", PASSWORD)),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 503);
    assert_error_shape(&r.body, "server_busy");
    assert_eq!(r.body["error"]["retry_after_s"], 2);
    drop(permits);
}

#[tokio::test]
async fn plain_http_from_outside_is_refused_on_password_routes() {
    let ts = TestServer::standard();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = bearer(&a.device_key);
    let outside = remote([203, 0, 113, 7]);
    let cases: Vec<(&str, &str, Value)> = vec![
        ("POST", "/v1/login", login_body("mario", PASSWORD)),
        (
            "POST",
            "/v1/login/password",
            json!({"identifier": "mario", "temporary_password": "x", "new_password": PASSWORD, "device_name": DEVICE}),
        ),
        (
            "POST",
            "/v1/login/disable-device",
            json!({"identifier": "mario", "password": PASSWORD, "device_id": a.device_id}),
        ),
        (
            "PUT",
            "/v1/account/password",
            json!({"current_password": PASSWORD, "new_password": "nuova password 1", "sign_out_other_devices": false}),
        ),
        ("DELETE", "/v1/account", json!({"password": PASSWORD})),
    ];
    let before = ts.accounts().argon2_verifications();
    for (method, path, body) in cases {
        let r = request(
            &ts.public,
            method,
            path,
            Some(body),
            outside,
            &[("authorization", &auth)],
        )
        .await;
        assert_eq!(r.status, 400, "{method} {path}");
        assert_error_shape(&r.body, "tls_required");
    }
    assert_eq!(ts.accounts().argon2_verifications(), before);
    // Anche le rotte con la sola chiave del dispositivo: la chiave vale come una password
    // (`DeviceAuth`, come il calcolo e la sincronizzazione).
    let r = request(
        &ts.public,
        "GET",
        "/v1/account/devices",
        None,
        outside,
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 400);
    assert_error_shape(&r.body, "tls_required");
}

#[tokio::test]
async fn trusted_proxy_with_https_is_accepted() {
    let mut config = test_config();
    config.trusted_proxies = vec!["10.0.0.2".parse().unwrap()];
    config.login_max_failures_per_origin = 1;
    let ts = TestServer::new(config);
    user_with_device(&ts, "mario", PASSWORD).await;
    let proxy = remote([10, 0, 0, 2]);

    let https = [
        ("x-forwarded-for", "198.51.100.7"),
        ("x-forwarded-proto", "https"),
    ];
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", PASSWORD)),
        proxy,
        &https,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text);

    let http = [
        ("x-forwarded-for", "198.51.100.7"),
        ("x-forwarded-proto", "http"),
    ];
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", PASSWORD)),
        proxy,
        &http,
    )
    .await;
    assert_eq!(r.status, 400);
    assert_error_shape(&r.body, "tls_required");

    // Il limite per origine segue X-Forwarded-For, non il proxy.
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", "sbagliata!!")),
        proxy,
        &https,
    )
    .await;
    assert_eq!(r.status, 401);
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", PASSWORD)),
        proxy,
        &https,
    )
    .await;
    assert_eq!(r.status, 429);
    let other = [
        ("x-forwarded-for", "198.51.100.8"),
        ("x-forwarded-proto", "https"),
    ];
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", PASSWORD)),
        proxy,
        &other,
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text);

    // Un client che si finge proxy non è fidato.
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", PASSWORD)),
        remote([203, 0, 113, 9]),
        &https,
    )
    .await;
    assert_eq!(r.status, 400);
}

#[tokio::test]
async fn missing_or_invalid_key_is_unauthorized() {
    let ts = TestServer::standard();
    for headers in [
        vec![],
        vec![("authorization", "Bearer spk_dev_nonvalida")],
        vec![("authorization", "Basic abc")],
    ] {
        let r = request(
            &ts.public,
            "GET",
            "/v1/account/devices",
            None,
            loopback(),
            &headers,
        )
        .await;
        assert_eq!(r.status, 401);
        assert_error_shape(&r.body, "unauthorized");
        assert_eq!(r.headers["www-authenticate"], "Bearer");
    }
}

#[tokio::test]
async fn whoami_fixes_the_device_auth_contract() {
    let ts = TestServer::standard();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = bearer(&a.device_key);
    // In chiaro da fuori la chiave non passa (`tls_required`); dal loopback sì.
    let r = request(
        &ts.public,
        "GET",
        "/test/whoami",
        None,
        remote([203, 0, 113, 3]),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 400);
    assert_error_shape(&r.body, "tls_required");
    let r = request(
        &ts.public,
        "GET",
        "/test/whoami",
        None,
        loopback(),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(
        r.body,
        json!({ "user_id": a.user_id, "device_id": a.device_id })
    );

    let r = request(&ts.public, "GET", "/test/whoami", None, loopback(), &[]).await;
    assert_eq!(r.status, 401);

    ts.accounts().disable_user(&a.user_id).await.unwrap();
    let r = request(
        &ts.public,
        "GET",
        "/test/whoami",
        None,
        loopback(),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn account_routes_happy_paths_and_errors() {
    let ts = TestServer::standard();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let b = ts
        .accounts()
        .login(support::ip(loopback()), "mario", PASSWORD, DEVICE)
        .await
        .unwrap();
    let auth_a = bearer(&a.device_key);
    let auth_b = bearer(&b.device_key);

    // Cambio password: sbagliata, poi giusta senza scollegare gli altri.
    let r = request(
        &ts.public,
        "PUT",
        "/v1/account/password",
        Some(json!({"current_password": "sbagliata!!", "new_password": "nuova password 1"})),
        loopback(),
        &[("authorization", &auth_a)],
    )
    .await;
    assert_eq!(r.status, 401);
    assert_error_shape(&r.body, "invalid_credentials");
    let r = request(
        &ts.public,
        "PUT",
        "/v1/account/password",
        Some(json!({"current_password": PASSWORD, "new_password": "corta"})),
        loopback(),
        &[("authorization", &auth_a)],
    )
    .await;
    assert_eq!(r.status, 400);
    assert_error_shape(&r.body, "invalid_request");
    let r = request(&ts.public, "PUT", "/v1/account/password",
        Some(json!({"current_password": PASSWORD, "new_password": "nuova password 1", "sign_out_other_devices": false})),
        loopback(), &[("authorization", &auth_a)]).await;
    assert_eq!(r.status, 204);
    let r = request(
        &ts.public,
        "GET",
        "/test/whoami",
        None,
        loopback(),
        &[("authorization", &auth_b)],
    )
    .await;
    assert_eq!(r.status, 200);

    // Disattivare un altro dispositivo.
    let path = format!("/v1/account/devices/{}", b.device_id);
    let r = request(
        &ts.public,
        "DELETE",
        &path,
        None,
        loopback(),
        &[("authorization", &auth_a)],
    )
    .await;
    assert_eq!(r.status, 204);
    let r = request(
        &ts.public,
        "DELETE",
        &path,
        None,
        loopback(),
        &[("authorization", &auth_a)],
    )
    .await;
    assert_eq!(r.status, 404);
    assert_error_shape(&r.body, "not_found");
    let r = request(
        &ts.public,
        "GET",
        "/test/whoami",
        None,
        loopback(),
        &[("authorization", &auth_b)],
    )
    .await;
    assert_eq!(r.status, 401);

    // Cancellazione: password sbagliata, poi giusta.
    let r = request(
        &ts.public,
        "DELETE",
        "/v1/account",
        Some(json!({"password": PASSWORD})),
        loopback(),
        &[("authorization", &auth_a)],
    )
    .await;
    assert_eq!(r.status, 401);
    let r = request(
        &ts.public,
        "DELETE",
        "/v1/account",
        Some(json!({"password": "nuova password 1"})),
        loopback(),
        &[("authorization", &auth_a)],
    )
    .await;
    assert_eq!(r.status, 204);
    let r = request(
        &ts.public,
        "GET",
        "/test/whoami",
        None,
        loopback(),
        &[("authorization", &auth_a)],
    )
    .await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn sign_out_this_device() {
    let ts = TestServer::standard();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = bearer(&a.device_key);
    let r = request(
        &ts.public,
        "DELETE",
        "/v1/devices/self",
        None,
        loopback(),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 204);
    let r = request(
        &ts.public,
        "DELETE",
        "/v1/devices/self",
        None,
        loopback(),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn bad_bodies_are_invalid_requests_without_echo() {
    let ts = TestServer::standard();
    let r = request_raw(
        &ts.public,
        "POST",
        "/v1/login",
        Some(b"{\"identifier\": \"mario\", \"password\": 12345678901".to_vec()),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 400);
    assert_error_shape(&r.body, "invalid_request");
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(json!({"identifier": "mario", "password": 1234567890123_u64, "device_name": DEVICE})),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 400);
    assert_error_shape(&r.body, "invalid_request");
    assert!(!r.text.contains("1234567890123"));
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(json!({"identifier": "mario"})),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 400);
    let empty_device = json!({"identifier": "mario", "password": PASSWORD, "device_name": "  "});
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(empty_device),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 400);
    assert_error_shape(&r.body, "invalid_request");
}

#[tokio::test]
async fn oversized_bodies_are_payload_too_large() {
    let ts = TestServer::standard();
    let huge = "x".repeat(70 * 1024);
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(login_body("mario", &huge)),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 413);
    assert_error_shape(&r.body, "payload_too_large");
    assert_eq!(r.headers["cache-control"], "no-store");
}

#[tokio::test]
async fn admin_routes_do_not_exist_on_the_public_listener() {
    let ts = TestServer::standard();
    let admin_auth = bearer(support::ADMIN_KEY);
    for (method, path) in [
        ("GET", "/admin/api/users"),
        ("POST", "/admin/api/users"),
        ("GET", "/admin"),
    ] {
        let r = request(
            &ts.public,
            method,
            path,
            None,
            loopback(),
            &[("authorization", &admin_auth)],
        )
        .await;
        assert_eq!(r.status, 404, "{method} {path}");
        assert_error_shape(&r.body, "not_found");
    }
    let r = request(&ts.public, "POST", "/health", None, loopback(), &[]).await;
    assert_eq!(r.status, 405);
    assert_error_shape(&r.body, "method_not_allowed");
}

#[tokio::test]
async fn login_in_clear_from_a_plaintext_from_address_passes_and_from_another_answers_tls_required() {
    let mut config = test_config();
    config.plaintext_from = vec!["10.0.0.9".parse().unwrap()];
    let ts = TestServer::new(config);
    user_with_device(&ts, "mario", PASSWORD).await;
    let login = |from: [u8; 4]| {
        request(
            &ts.public,
            "POST",
            "/v1/login",
            Some(login_body("mario", PASSWORD)),
            remote(from),
            &[],
        )
    };
    assert_eq!(login([10, 0, 0, 9]).await.status, 200);
    let other = login([10, 0, 0, 10]).await;
    assert_eq!((other.status.as_u16(), other.code()), (400, Some("tls_required")));
}

#[tokio::test]
async fn the_routes_of_a_device_need_a_secure_connection_and_limit_wrong_keys_per_origin() {
    let mut config = test_config();
    config.compute_max_auth_failures_per_origin = 3;
    config.plaintext_from = vec![[10, 0, 0, 9].into()];
    let ts = TestServer::new(config);
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = bearer(&a.device_key);
    let public = remote([203, 0, 113, 5]);

    // Every route behind a device key: plain text from a public address is refused with tls_required.
    for (method, path) in [
        ("GET", "/v1/account"),
        ("GET", "/v1/account/devices"),
        ("DELETE", "/v1/account/devices/x"),
        ("DELETE", "/v1/devices/self"),
        ("GET", "/test/whoami"),
    ] {
        let r = request(&ts.public, method, path, None, public, &[("authorization", &auth)]).await;
        assert_eq!(r.status, 400, "{method} {path}");
        assert_eq!(r.code(), Some("tls_required"), "{method} {path}");
    }
    // Still alive afterwards: the refused calls revoked nothing.
    let r = request(&ts.public, "GET", "/v1/account", None, loopback(), &[("authorization", &auth)]).await;
    assert_eq!(r.status, 200);
    // `plaintext_from` is secure, like for the login.
    let r = request(&ts.public, "GET", "/v1/account", None, remote([10, 0, 0, 9]), &[("authorization", &auth)]).await;
    assert_eq!(r.status, 200);

    // Wrong keys count per origin; past the limit the origin gets 429 before any query.
    let wrong = bearer("spk_dev_0000000000000000000000000000000000000000000000000000000000000000");
    for _ in 0..3 {
        let r = request(&ts.public, "GET", "/v1/account", None, loopback(), &[("authorization", &wrong)]).await;
        assert_eq!(r.status, 401);
    }
    let r = request(&ts.public, "GET", "/v1/account", None, loopback(), &[("authorization", &auth)]).await;
    assert_eq!(r.status, 429);
    assert_eq!(r.code(), Some("rate_limited"));
    // Another origin is not affected.
    let r = request(&ts.public, "GET", "/v1/account", None, remote([10, 0, 0, 9]), &[("authorization", &auth)]).await;
    assert_eq!(r.status, 200);
}

#[tokio::test]
async fn the_admin_listener_ignores_the_plaintext_list_of_the_public_one() {
    let mut config = test_config();
    config.plaintext_from = vec![[10, 0, 0, 9].into()];
    config.admin_bind = Some("127.0.0.1:9001".parse().unwrap());
    config.admin_plaintext_from = vec![[10, 0, 0, 7].into()];
    let ts = TestServer::new(config);
    let admin = bearer(ADMIN_KEY);
    // In the public list only: plain text on the administrative listener is refused.
    let r = request(ts.admin(), "GET", "/admin/api/users", None, remote([10, 0, 0, 9]), &[("authorization", &admin)]).await;
    assert_eq!((r.status.as_u16(), r.code()), (400, Some("tls_required")));
    // In the administrative list: accepted.
    let r = request(ts.admin(), "GET", "/admin/api/users", None, remote([10, 0, 0, 7]), &[("authorization", &admin)]).await;
    assert_eq!(r.status, 200);
    let r = request(ts.admin(), "GET", "/admin/api/users", None, loopback(), &[("authorization", &admin)]).await;
    assert_eq!(r.status, 200);
}
