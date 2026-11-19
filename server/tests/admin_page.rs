mod support;

use support::{ADMIN_KEY, TestServer, bearer, loopback, remote, request, test_config};

const PAGES: [(&str, &str); 4] = [
    ("/admin", "text/html; charset=utf-8"),
    ("/admin/", "text/html; charset=utf-8"),
    ("/admin/app.css", "text/css; charset=utf-8"),
    ("/admin/app.js", "text/javascript; charset=utf-8"),
];

const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; connect-src 'self'; frame-ancestors 'none'; form-action 'none'; base-uri 'none'";

#[tokio::test]
async fn the_page_is_served_with_security_headers() {
    let ts = TestServer::standard();
    for (path, content_type) in PAGES {
        let r = request(ts.admin(), "GET", path, None, loopback(), &[]).await;
        assert_eq!(r.status, 200, "{path}");
        assert_eq!(r.headers["content-type"], content_type, "{path}");
        assert_eq!(r.headers["content-security-policy"], CSP, "{path}");
        assert_eq!(r.headers["x-content-type-options"], "nosniff");
        assert_eq!(r.headers["cache-control"], "no-store");
        assert_eq!(r.headers["x-frame-options"], "DENY");
        assert_eq!(r.headers["referrer-policy"], "no-referrer");
        assert!(!r.headers.contains_key("set-cookie"));
        assert!(!r.text.is_empty());
    }
    // Anche le risposte dell'API amministrativa portano le intestazioni.
    let r = request(
        ts.admin(),
        "GET",
        "/admin/api/users",
        None,
        loopback(),
        &[("authorization", &bearer(ADMIN_KEY))],
    )
    .await;
    assert_eq!(r.status, 200);
    assert_eq!(r.headers["content-security-policy"], CSP);
    assert_eq!(r.headers["x-content-type-options"], "nosniff");
}

#[tokio::test]
async fn the_page_does_not_exist_on_the_public_listener() {
    let ts = TestServer::standard();
    for (path, _) in PAGES {
        for origin in [loopback(), remote([203, 0, 113, 1])] {
            let r = request(&ts.public, "GET", path, None, origin, &[]).await;
            assert_eq!(r.status, 404, "{path} sul pubblico (INV-18)");
            assert_eq!(r.code(), Some("not_found"));
            assert!(!r.text.contains("<html"));
        }
    }
}

#[tokio::test]
async fn the_page_needs_a_secure_connection_or_an_allowed_plaintext_source() {
    let ts = TestServer::standard();
    let r = request(
        ts.admin(),
        "GET",
        "/admin",
        None,
        remote([172, 30, 87, 1]),
        &[],
    )
    .await;
    assert_eq!(r.status, 400);
    assert_eq!(r.code(), Some("tls_required"));

    // Il gateway della rete del compose (porta pubblicata su 127.0.0.1 dell'host).
    let mut config = test_config();
    config.admin_bind = Some("172.30.87.3:8788".parse().unwrap());
    config.admin_plaintext_from = vec!["172.30.87.1".parse().unwrap()];
    let ts = TestServer::new(config);
    let gateway = remote([172, 30, 87, 1]);
    let r = request(ts.admin(), "GET", "/admin", None, gateway, &[]).await;
    assert_eq!(r.status, 200);
    let r = request(
        ts.admin(),
        "GET",
        "/admin/api/users",
        None,
        gateway,
        &[("authorization", &bearer(ADMIN_KEY))],
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text);
    // Un altro indirizzo della stessa rete resta fuori.
    let r = request(
        ts.admin(),
        "GET",
        "/admin/api/users",
        None,
        remote([172, 30, 87, 9]),
        &[("authorization", &bearer(ADMIN_KEY))],
    )
    .await;
    assert_eq!(r.code(), Some("tls_required"));
    // E l'eccezione vale solo per l'ascolto amministrativo.
    let r = request(
        &ts.public,
        "POST",
        "/v1/login",
        Some(serde_json::json!({"identifier": "x", "password": "y", "device_name": "z"})),
        gateway,
        &[],
    )
    .await;
    assert_eq!(r.code(), Some("tls_required"));
}

#[test]
fn the_script_never_writes_html_or_stores_anything() {
    let script = include_str!("../src/http/admin/app.js");
    for forbidden in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "localStorage",
        "sessionStorage",
        "document.cookie",
        "document.write",
        "eval(",
        "new Function",
    ] {
        assert!(!script.contains(forbidden), "app.js contiene «{forbidden}»");
    }
    assert!(script.contains("Authorization"));
    assert!(script.contains("status === 401"));
}

#[test]
fn the_html_has_no_inline_code_or_style() {
    let html = include_str!("../src/http/admin/index.html");
    let lower = html.to_lowercase();
    assert!(!lower.contains("<style"));
    assert!(!lower.contains(" style="));
    assert!(!lower.contains("javascript:"));
    for (i, _) in lower.match_indices("<script") {
        let tag_end = lower[i..].find('>').unwrap() + i;
        assert!(
            lower[i..tag_end].contains("src=\"/admin/app.js\""),
            "script in linea"
        );
    }
    // Nessun attributo di evento (onclick=…).
    let mut rest = lower.as_str();
    while let Some(pos) = rest.find(" on") {
        let attr: String = rest[pos + 1..]
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .collect();
        assert!(
            !(attr.len() > 2 && rest[pos + 1 + attr.len()..].starts_with('=')),
            "attributo {attr}"
        );
        rest = &rest[pos + 3..];
    }
    // La chiave non si salva: il campo non chiede il completamento.
    assert!(html.contains("type=\"password\" autocomplete=\"off\""));
}

#[test]
fn the_plaintext_exception_needs_a_private_admin_address() {
    use qamille_server::config::ConfigError;
    let cases: [(Option<&str>, &str); 5] = [
        (None, "172.30.87.1"),
        (Some("0.0.0.0:8788"), "172.30.87.1"),
        (Some("[::]:8788"), "172.30.87.1"),
        (Some("203.0.113.5:8788"), "172.30.87.1"),
        // Una sorgente pubblica nemmeno.
        (Some("172.30.87.3:8788"), "198.51.100.1"),
    ];
    for (bind, source) in cases {
        let mut config = test_config();
        config.admin_bind = bind.map(|b| b.parse().unwrap());
        config.admin_plaintext_from = vec![source.parse().unwrap()];
        assert!(
            matches!(
                config.validate(),
                Err(ConfigError::Invalid {
                    key: "admin_plaintext_from",
                    ..
                })
            ),
            "{bind:?} {source}"
        );
    }
    for (bind, source) in [
        ("127.0.0.1:8788", "127.0.0.1"),
        ("10.1.2.3:8788", "10.1.2.1"),
        ("192.168.1.10:8788", "192.168.1.1"),
        ("[fd00:5a7c:87::3]:8788", "fd00:5a7c:87::1"),
    ] {
        let mut config = test_config();
        config.admin_bind = Some(bind.parse().unwrap());
        config.admin_plaintext_from = vec![source.parse().unwrap()];
        assert!(config.validate().is_ok(), "{bind} {source}");
    }
    // Senza eccezione al chiaro, admin_bind può essere qualunque (con TLS davanti).
    let mut config = test_config();
    config.admin_bind = Some("0.0.0.0:8788".parse().unwrap());
    assert!(config.validate().is_ok());
}

#[tokio::test]
async fn a_local_user_cannot_lock_the_admin_out_with_a_few_wrong_keys() {
    let ts = TestServer::standard();
    let gateway = loopback();
    for _ in 0..100 {
        let r = request(
            ts.admin(),
            "GET",
            "/admin/api/users",
            None,
            gateway,
            &[("authorization", "Bearer sbagliata-sbagliata-sbagliata-123")],
        )
        .await;
        assert_eq!(r.status, 401);
    }
    let r = request(
        ts.admin(),
        "GET",
        "/admin/api/users",
        None,
        gateway,
        &[("authorization", &bearer(ADMIN_KEY))],
    )
    .await;
    assert_eq!(r.status, 200, "{}", r.text);
    assert_eq!(
        qamille_server::Config::with_data_dir("x").admin_max_failures,
        300
    );
}

#[tokio::test]
async fn the_core_page_has_users_and_models_and_no_subscriptions() {
    let ts = TestServer::standard();
    let js = request(ts.admin(), "GET", "/admin/app.js", None, loopback(), &[])
        .await
        .text;
    // La pagina di oggi non ha una sezione modelli: i modelli si gestiscono via API.
    assert!(js.contains("/admin/api") && js.contains("'/users'") || js.contains("/users"));
    for word in ["subscription", "billing", "abbonament"] {
        assert!(!js.to_lowercase().contains(word), "{word}");
    }
    assert!(!js.contains("innerHTML"));
    let html = request(ts.admin(), "GET", "/admin", None, loopback(), &[])
        .await
        .text;
    for word in ["subscription", "billing", "abbonament"] {
        assert!(!html.to_lowercase().contains(word), "{word} in index.html");
    }
}

#[test]
fn the_users_table_has_a_llm_column_hidden_until_the_server_says_llm() {
    let html = include_str!("../src/http/admin/index.html");
    let script = include_str!("../src/http/admin/app.js");
    let th = html.find("id=\"llm-col\"").expect("colonna dell'assistente");
    let tag_start = html[..th].rfind("<th").unwrap();
    let tag_end = html[th..].find("</th>").unwrap() + th;
    let tag = &html[tag_start..tag_end];
    assert!(tag.contains("hidden") && tag.contains("Token assistente"), "{tag}");
    assert!(th < html.find(">Azioni<").unwrap(), "la colonna sta prima di «Azioni»");
    assert!(script.contains("byId('llm-col').hidden = !data.llm"));
    assert!(script.contains("(incoming + outgoing).toLocaleString('it-IT')"));
    assert!(script.contains("toLocaleString('it-IT')"));
}

// --- The «Modelli» section (Task 12) -----------------------------------------------------

const MODELS_IDS: [&str; 8] = [
    "models-section",
    "models-title",
    "models-refresh",
    "models-host",
    "models-status",
    "models-error",
    "models-table",
    "models-body",
];

fn page_html() -> &'static str {
    include_str!("../src/http/admin/index.html")
}

fn page_script() -> &'static str {
    include_str!("../src/http/admin/app.js")
}

/// The text of `function <name>` up to the next top-level function of the script.
fn function_text<'a>(script: &'a str, name: &str) -> &'a str {
    let start = script.find(&format!("function {name}(")).unwrap_or_else(|| panic!("function {name}"));
    let rest = &script[start + 1..];
    let end = rest.find("\n  function ").or_else(|| rest.find("\n  async function ")).map_or(script.len(), |n| start + 1 + n);
    &script[start..end]
}

#[test]
fn the_page_has_a_models_section_hidden_until_the_key_is_given() {
    let html = page_html();
    for id in MODELS_IDS {
        assert!(html.contains(&format!("id=\"{id}\"")), "manca #{id}");
    }
    let at = html.find("id=\"models-section\"").unwrap();
    let tag = &html[html[..at].rfind('<').unwrap()..at + html[at..].find('>').unwrap()];
    assert!(tag.contains("hidden"), "{tag}");
    let script = page_script();
    assert!(script.contains("byId('models-section').hidden = false"));
    assert!(function_text(script, "lock").contains("byId('models-section').hidden = true"));
    // Shown together with the users, after the key is accepted.
    assert!(function_text(script, "loadUsers").contains("models-section") || function_text(script, "loadUsers").contains("loadModels"));
}

#[test]
fn the_models_script_uses_only_the_admin_api() {
    let script = page_script();
    assert_eq!(script.matches("fetch(").count(), 1, "una sola chiamata di rete, in api()");
    assert!(script.contains("fetch('/admin/api' + path"));
    for forbidden in ["http://", "https://", "XMLHttpRequest", "WebSocket", "sendBeacon", "EventSource"] {
        assert!(!script.contains(forbidden), "app.js contiene «{forbidden}»");
    }
    for call in ["api('GET', '/models')", "'/models/' + encodeURIComponent(model.id) + '/install'", "api('DELETE', '/models/' + encodeURIComponent(model.id))"] {
        assert!(script.contains(call), "manca {call}");
    }
}

#[test]
fn the_page_shows_memory_and_disk_before_install() {
    let script = page_script();
    let actions = function_text(script, "modelActions");
    for name in ["ram_required_mb", "disk_required_bytes", "available_memory_mb", "available_disk_bytes"] {
        assert!(script.contains(name), "manca {name}");
    }
    let note = actions.find("available_memory_mb").expect("la memoria libera si legge nelle azioni");
    let button = actions.find("text: 'Installa'").expect("il pulsante Installa");
    assert!(note < button, "memoria e spazio compaiono prima del pulsante");
    assert!(actions.contains("La memoria libera ora è "), "l'avviso sulla memoria");
    assert!(actions.contains("disabled"), "senza spazio il pulsante è disattivato");
}

#[test]
fn the_page_shows_the_license_and_where_its_terms_are_before_install() {
    let script = page_script();
    let actions = function_text(script, "modelActions");
    let notice = actions.find("Condizioni: ").expect("le condizioni si leggono nelle azioni");
    let button = actions.find("text: 'Installa'").expect("il pulsante Installa");
    assert!(notice < button, "le condizioni compaiono prima del pulsante");
    assert!(actions.contains("details.license_url") && actions.contains("details.license"), "{actions}");
    // The address is text from the server, never a link or a navigation.
    assert!(!actions.contains("href") && !actions.contains("window.open"), "{actions}");
}

#[test]
fn models_are_rendered_with_text_content_only() {
    let script = page_script();
    for forbidden in ["innerHTML", "outerHTML", "insertAdjacentHTML", "document.write", "eval(", "new Function"] {
        assert!(!script.contains(forbidden), "app.js contiene «{forbidden}»");
    }
    let models = function_text(script, "renderModels");
    assert!(models.contains("el('tr'") && models.contains("text:"), "{models}");
}

#[test]
fn remove_asks_for_a_second_click_and_uses_no_browser_dialog() {
    let script = page_script();
    assert!(script.contains("Sicuro? Rimuovi"));
    for dialog in ["confirm(", "alert(", "prompt("] {
        assert!(!script.contains(dialog), "app.js contiene «{dialog}»");
    }
}

#[test]
fn the_page_has_no_inline_script_or_style() {
    let lower = page_html().to_lowercase();
    assert!(!lower.contains("<style") && !lower.contains(" style=") && !lower.contains("javascript:"));
    assert_eq!(lower.matches("<script").count(), 1, "un solo script, con src");
    let script = page_script();
    assert!(!script.contains(".style") && !script.contains("'style'") && !script.contains("setAttribute('on"));
}

#[test]
fn the_polling_stops_when_nothing_is_downloading() {
    let script = page_script();
    let load = function_text(script, "loadModels");
    assert!(load.contains("m.state === 'downloading'"), "{load}");
    assert!(load.contains("setTimeout(") && load.contains("2000"), "{load}");
    assert!(load.contains("clearTimeout"), "un solo timer alla volta");
    assert!(function_text(script, "lock").contains("clearTimeout"), "la chiave tolta ferma il timer");
}

#[tokio::test]
async fn the_page_is_still_served_with_the_security_headers_and_the_models_section() {
    let ts = TestServer::standard();
    for path in ["/admin", "/admin/app.js", "/admin/app.css"] {
        let r = request(ts.admin(), "GET", path, None, loopback(), &[]).await;
        assert_eq!(r.status, 200, "{path}");
        assert_eq!(r.headers["content-security-policy"], CSP, "{path}");
        assert_eq!(r.headers["cache-control"], "no-store");
    }
    let html = request(ts.admin(), "GET", "/admin", None, loopback(), &[]).await.text;
    assert!(html.contains("id=\"models-section\""));
}
