mod support;

use serde_json::{Value, json};
use support::{
    ADMIN_KEY, TestServer, bearer, keys, loopback, remote, request, test_config, user_with_device,
};

const PASSWORD: &str = "una password lunga";

fn admin_header() -> String {
    bearer(ADMIN_KEY)
}

#[tokio::test]
async fn users_endpoint_needs_the_admin_key() {
    let ts = TestServer::standard();
    let auth = admin_header();
    let r = request(
        ts.admin(),
        "GET",
        "/admin/api/users",
        None,
        loopback(),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(keys(&r.body), vec!["default_max_devices", "users"]);
    assert_eq!(r.headers["cache-control"], "no-store");

    for headers in [
        vec![],
        vec![("authorization", "Bearer 0123456789abcdef0123456789abcdeX")],
    ] {
        let r = request(
            ts.admin(),
            "GET",
            "/admin/api/users",
            None,
            loopback(),
            &headers,
        )
        .await;
        assert_eq!(r.status, 401);
        assert_eq!(r.body["error"]["code"], "unauthorized");
    }

    // Le rotte pubbliche non esistono sull'ascolto amministrativo.
    let r = request(
        ts.admin(),
        "POST",
        "/v1/login",
        Some(json!({})),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 404);
}

#[tokio::test]
async fn admin_requires_tls_outside_loopback() {
    let mut config = test_config();
    config.trusted_proxies = vec!["10.0.0.2".parse().unwrap()];
    let ts = TestServer::new(config);
    let auth = admin_header();
    let r = request(
        ts.admin(),
        "GET",
        "/admin/api/users",
        None,
        remote([192, 168, 1, 20]),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.body["error"]["code"], "tls_required");
    let r = request(
        ts.admin(),
        "GET",
        "/admin/api/users",
        None,
        remote([10, 0, 0, 2]),
        &[
            ("authorization", &auth),
            ("x-forwarded-for", "192.168.1.20"),
            ("x-forwarded-proto", "https"),
        ],
    )
    .await;
    assert_eq!(r.status, 200);
}

#[tokio::test]
async fn admin_failures_are_limited_per_origin() {
    let mut config = test_config();
    config.admin_max_failures = 3;
    let ts = TestServer::new(config);
    for _ in 0..3 {
        let r = request(
            ts.admin(),
            "GET",
            "/admin/api/users",
            None,
            loopback(),
            &[("authorization", "Bearer sbagliata")],
        )
        .await;
        assert_eq!(r.status, 401);
    }
    let auth = admin_header();
    let r = request(
        ts.admin(),
        "GET",
        "/admin/api/users",
        None,
        loopback(),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 429);
    assert_eq!(r.body["error"]["code"], "rate_limited");
    assert!(r.body["error"]["retry_after_s"].as_u64().unwrap() > 0);
    // Il limite amministrativo è separato da quello del login.
    let created = ts
        .accounts()
        .create_user("mario", None, None)
        .await
        .unwrap();
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(json!({"identifier": "mario", "password": created.temporary_password, "device_name": "x"})),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 403);
}

#[tokio::test]
async fn no_admin_router_without_admin_key() {
    let mut config = test_config();
    config.admin_key = None;
    let ts = TestServer::new(config);
    assert!(ts.admin.is_none());
}

#[tokio::test]
async fn user_management_round_trip() {
    let ts = TestServer::standard();
    let auth = admin_header();
    let h = [("authorization", auth.as_str())];

    let r = request(
        ts.admin(),
        "POST",
        "/admin/api/users",
        Some(json!({"username": "mario", "email": "mario@example.com", "max_devices": 2})),
        loopback(),
        &h,
    )
    .await;
    assert_eq!(r.status, 201, "{}", r.text);
    assert_eq!(
        keys(&r.body),
        vec![
            "email",
            "id",
            "temporary_expires_at",
            "temporary_password",
            "username"
        ]
    );
    let user_id = r.body["id"].as_str().unwrap().to_owned();
    let temporary = r.body["temporary_password"].as_str().unwrap().to_owned();
    assert_eq!(temporary.len(), 20);

    let r = request(
        ts.admin(),
        "POST",
        "/admin/api/users",
        Some(json!({"username": "MARIO"})),
        loopback(),
        &h,
    )
    .await;
    assert_eq!(r.status, 409);
    assert_eq!(r.body["error"]["code"], "username_taken");
    let r = request(
        ts.admin(),
        "POST",
        "/admin/api/users",
        Some(json!({"username": "luigi", "email": "MARIO@example.com"})),
        loopback(),
        &h,
    )
    .await;
    assert_eq!(r.status, 409);
    assert_eq!(r.body["error"]["code"], "email_taken");
    let r = request(
        ts.admin(),
        "POST",
        "/admin/api/users",
        Some(json!({"username": "a@b"})),
        loopback(),
        &h,
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.body["error"]["code"], "invalid_request");

    let r = request(ts.admin(), "GET", "/admin/api/users", None, loopback(), &h).await;
    let users = r.body["users"].as_array().unwrap();
    assert_eq!(users.len(), 1);
    assert_eq!(
        keys(&users[0]),
        vec![
            "active_devices",
            "created_at",
            "disabled_at",
            "email",
            "id",
            "max_devices",
            "password_temporary",
            "temporary_expires_at",
            "usage",
            "username"
        ]
    );
    // Compute counters (D5): numbers only.
    assert_eq!(
        keys(&users[0]["usage"]),
        vec!["audio_seconds", "embedded_tokens", "llm_completion_tokens", "llm_prompt_tokens", "requests"]
    );
    assert!(!r.text.contains("argon2"));

    // Reimpostazione: nuova provvisoria.
    let path = format!("/admin/api/users/{user_id}/reset-password");
    let r = request(ts.admin(), "POST", &path, None, loopback(), &h).await;
    assert_eq!(r.status, 200);
    assert_eq!(
        keys(&r.body),
        vec!["temporary_expires_at", "temporary_password"]
    );
    let new_temporary = r.body["temporary_password"].as_str().unwrap().to_owned();
    assert_ne!(new_temporary, temporary);

    // L'utente cambia la provvisoria ed entra.
    let r = request(
        &ts.public,
        "POST",
        "/v1/login/password",
        Some(json!({"identifier": "mario@example.com", "temporary_password": new_temporary, "new_password": PASSWORD, "device_name": "PC"})),
        loopback(),
        &[],
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text);
    let device_id = r.body["device_id"].as_str().unwrap().to_owned();
    let key = r.body["device_key"].as_str().unwrap().to_owned();

    let r = request(
        ts.admin(),
        "GET",
        &format!("/admin/api/users/{user_id}/devices"),
        None,
        loopback(),
        &h,
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.body["devices"][0]["id"], device_id.as_str());

    // Disattivazione e riattivazione.
    let r = request(
        ts.admin(),
        "POST",
        &format!("/admin/api/users/{user_id}/disable"),
        None,
        loopback(),
        &h,
    )
    .await;
    assert_eq!(r.status, 204);
    let user_auth = bearer(&key);
    let r = request(
        &ts.public,
        "GET",
        "/test/whoami",
        None,
        loopback(),
        &[("authorization", &user_auth)],
    )
    .await;
    assert_eq!(r.status, 401);
    let r = request(
        ts.admin(),
        "POST",
        &format!("/admin/api/users/{user_id}/enable"),
        None,
        loopback(),
        &h,
    )
    .await;
    assert_eq!(r.status, 204);
    // Riattivarlo non fa rivivere la chiave di prima: si accede di nuovo.
    let r = request(
        &ts.public,
        "GET",
        "/test/whoami",
        None,
        loopback(),
        &[("authorization", &user_auth)],
    )
    .await;
    assert_eq!(r.status, 401);
    let again = ts
        .accounts()
        .login(support::ip(loopback()), "mario@example.com", PASSWORD, "PC")
        .await
        .unwrap();
    let device_id = again.device_id.clone();
    let user_auth = bearer(&again.device_key);
    let r = request(
        &ts.public,
        "GET",
        "/test/whoami",
        None,
        loopback(),
        &[("authorization", &user_auth)],
    )
    .await;
    assert_eq!(r.status, 200);

    // Massimo di dispositivi.
    let r = request(
        ts.admin(),
        "POST",
        &format!("/admin/api/users/{user_id}/max-devices"),
        Some(json!({"max_devices": 5})),
        loopback(),
        &h,
    )
    .await;
    assert_eq!(r.status, 204);
    let r = request(ts.admin(), "GET", "/admin/api/users", None, loopback(), &h).await;
    assert_eq!(r.body["users"][0]["max_devices"], 5);
    let r = request(
        ts.admin(),
        "POST",
        &format!("/admin/api/users/{user_id}/max-devices"),
        Some(json!({"max_devices": null})),
        loopback(),
        &h,
    )
    .await;
    assert_eq!(r.status, 204);
    let r = request(ts.admin(), "GET", "/admin/api/users", None, loopback(), &h).await;
    assert_eq!(r.body["users"][0]["max_devices"], Value::Null);

    // Revoca di un dispositivo.
    let path = format!("/admin/api/users/{user_id}/devices/{device_id}");
    let r = request(ts.admin(), "DELETE", &path, None, loopback(), &h).await;
    assert_eq!(r.status, 204);
    let r = request(ts.admin(), "DELETE", &path, None, loopback(), &h).await;
    assert_eq!(r.status, 404);
    let r = request(
        &ts.public,
        "GET",
        "/test/whoami",
        None,
        loopback(),
        &[("authorization", &user_auth)],
    )
    .await;
    assert_eq!(r.status, 401);

    // Cancellazione.
    let r = request(
        ts.admin(),
        "DELETE",
        &format!("/admin/api/users/{user_id}"),
        None,
        loopback(),
        &h,
    )
    .await;
    assert_eq!(r.status, 204);
    for (method, path) in [
        ("DELETE", format!("/admin/api/users/{user_id}")),
        ("POST", format!("/admin/api/users/{user_id}/disable")),
        ("POST", format!("/admin/api/users/{user_id}/reset-password")),
        ("GET", format!("/admin/api/users/{user_id}/devices")),
    ] {
        let r = request(ts.admin(), method, &path, None, loopback(), &h).await;
        assert_eq!(r.status, 404, "{method} {path}");
        assert_eq!(r.body["error"]["code"], "not_found");
    }
}

#[tokio::test]
async fn admin_reset_revokes_devices() {
    let ts = TestServer::standard();
    let a = user_with_device(&ts, "mario", PASSWORD).await;
    let auth = admin_header();
    let r = request(
        ts.admin(),
        "POST",
        &format!("/admin/api/users/{}/reset-password", a.user_id),
        None,
        loopback(),
        &[("authorization", &auth)],
    )
    .await;
    assert_eq!(r.status, 200);
    let user_auth = bearer(&a.device_key);
    let r = request(
        &ts.public,
        "GET",
        "/test/whoami",
        None,
        loopback(),
        &[("authorization", &user_auth)],
    )
    .await;
    assert_eq!(r.status, 401);
}

#[tokio::test]
async fn the_users_list_says_llm_only_when_a_provider_is_on() {
    use support::fake_openai::FakeOpenai;
    use support::llm_env::{llm_for, ok, ts_with_llm};
    let auth = admin_header();
    let ts = TestServer::standard();
    let r = request(ts.admin(), "GET", "/admin/api/users", None, loopback(), &[("authorization", &auth)]).await;
    assert_eq!(keys(&r.body), vec!["default_max_devices", "users"]);

    let fake = FakeOpenai::start(ok()).await;
    let ts = ts_with_llm(llm_for(&fake.url));
    let r = request(ts.admin(), "GET", "/admin/api/users", None, loopback(), &[("authorization", &auth)]).await;
    assert_eq!(keys(&r.body), vec!["default_max_devices", "llm", "users"]);
    assert_eq!(r.body["llm"], true);
}
