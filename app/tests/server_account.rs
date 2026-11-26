//! The link to a Qamille server (access wave 2, compito 2): the session in the vault, consent,
//! computation mode and the rules of the two editions, with a fake `ServerAccount`; then the
//! HTTP implementation against an in-process fake server.
use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        ports::{AttachmentGateway, Clock, MailGateway, NullSink, OutgoingMessage, RemoteAction, SecretStore, SyncBatch},
        server_account::{
            BrowserTicket, ConnectOutcome, Credentials, LoginGrant, LoginOutcome, Probe, RemoteEndpoint, SERVER_SESSION, ServerAccount,
            LlmStatus, ServerAccountService, ServerAddress, SignOut, SignUp, browser_url,
        },
    },
    domain::{
        model::{Account, AiProvider, Attachment, ComputeMode, LlmConsent, Store},
        server_account::{AccountInfo, BillingPage, DeviceInfo, LlmInfo, ServerInfo, Subscription},
    },
    edition::Edition,
    infrastructure::{repository::MemoryRepository, server_account::HttpServerAccount},
};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    sync::{Arc, Mutex},
    time::Duration,
};

#[derive(Default)]
struct Vault {
    values: Mutex<HashMap<String, String>>,
    sets: Mutex<u32>,
}
impl SecretStore for Vault {
    fn get(&self, key: &str) -> AppResult<String> {
        Ok(self.values.lock().unwrap().get(key).cloned().unwrap_or_default())
    }
    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        *self.sets.lock().unwrap() += 1;
        self.values.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
}
struct NoMail;
impl MailGateway for NoMail {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Ok(SyncBatch { emails: vec![], archive_folder: None, trash_folder: None })
    }
    fn apply(&self, _: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        Ok(())
    }
    fn send(&self, _: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        Ok(())
    }
    fn check_login(&self, _: &Account) -> AppResult<()> {
        Ok(())
    }
}
struct Fixed;
impl Clock for Fixed {
    fn now(&self) -> i64 {
        1_800_000_000
    }
}
struct NoFiles;
impl AttachmentGateway for NoFiles {
    fn total_size(&self, paths: &[String]) -> AppResult<u64> {
        Ok(paths.len() as u64)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}

/// What the fake server answers to a sign-in.
#[derive(Clone, Copy)]
enum Answer {
    Grant,
    Temporary,
    Limit,
    Wrong,
}
struct FakeServer {
    answer: Mutex<Answer>,
    api: u32,
    offline: Mutex<bool>,
    /// What `GET /v1/account` says: `None` = an older server without the route.
    account: Mutex<Option<AccountInfo>>,
    /// The page address the server gives for a checkout.
    page_url: Mutex<String>,
    /// Every call, with the address it went to.
    calls: Mutex<Vec<String>>,
    /// The `llm` block of `GET /v1/info`.
    llm: Mutex<Option<LlmInfo>>,
}
impl FakeServer {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            answer: Mutex::new(Answer::Grant),
            api: 1,
            offline: Mutex::new(false),
            account: Mutex::new(None),
            page_url: Mutex::new("https://checkout.processor.example/c/1".into()),
            calls: Mutex::new(Vec::new()),
            llm: Mutex::new(None),
        })
    }
    fn log(&self, call: &str, url: &str) -> AppResult<()> {
        self.calls.lock().unwrap().push(format!("{call} {url}"));
        if *self.offline.lock().unwrap() { Err(AppError::new("unreachable", "down")) } else { Ok(()) }
    }
    fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap().clone()
    }
    fn outcome(&self, url: &str) -> AppResult<LoginOutcome> {
        Ok(match *self.answer.lock().unwrap() {
            Answer::Grant => LoginOutcome::Signed(LoginGrant {
                user_id: "u1".into(),
                username: "mario".into(),
                device_id: format!("dev-{}", url.len()),
                device_key: format!("spk_dev_{url}"),
            }),
            Answer::Temporary => LoginOutcome::PasswordChangeRequired,
            Answer::Limit => LoginOutcome::DeviceLimit(vec![DeviceInfo { id: "old".into(), name: "Portatile".into(), created_at: "2026-01-01".into(), last_seen_at: None, current: false }]),
            Answer::Wrong => return Err(AppError::new("invalid_credentials", "Wrong username, email or password")),
        })
    }
}
impl ServerAccount for FakeServer {
    fn info(&self, address: &ServerAddress) -> AppResult<ServerInfo> {
        self.log("info", &address.url)?;
        Ok(ServerInfo {
            version: "1.0".into(),
            api: self.api,
            signup: false,
            password_reset: false,
            edition: "self-hosted".into(),
            login_providers: vec!["google".into()],
            google_signup: false,
            billing: None,
            llm: self.llm.lock().unwrap().clone(),
            sync: None,
            legal: None,
        })
    }
    fn login(&self, address: &ServerAddress, _: &Credentials<'_>) -> AppResult<LoginOutcome> {
        self.log("login", &address.url)?;
        self.outcome(&address.url)
    }
    fn change_temporary_password(&self, address: &ServerAddress, _: &Credentials<'_>, _: &str) -> AppResult<LoginOutcome> {
        self.log("change_temporary", &address.url)?;
        *self.answer.lock().unwrap() = Answer::Grant;
        self.outcome(&address.url)
    }
    fn disable_device_with_password(&self, address: &ServerAddress, _: &str, _: &str, _: &str) -> AppResult<()> {
        self.log("disable_device", &address.url)
    }
    fn devices(&self, endpoint: &RemoteEndpoint) -> AppResult<Vec<DeviceInfo>> {
        self.log("devices", &endpoint.url)?;
        Ok(vec![])
    }
    fn revoke_device(&self, endpoint: &RemoteEndpoint, _: &str) -> AppResult<()> {
        self.log("revoke", &endpoint.url)
    }
    fn change_password(&self, endpoint: &RemoteEndpoint, _: &str, _: &str, _: bool) -> AppResult<()> {
        self.log("change_password", &endpoint.url)
    }
    fn sign_out(&self, endpoint: &RemoteEndpoint) -> AppResult<()> {
        self.log(&format!("sign_out({}s)", endpoint.timeout.as_secs()), &endpoint.url)
    }
    fn delete_account(&self, endpoint: &RemoteEndpoint, _: &str) -> AppResult<()> {
        self.log("delete_account", &endpoint.url)
    }
    fn signup_start(&self, address: &ServerAddress, _: &str) -> AppResult<()> {
        self.log("signup_start", &address.url)
    }
    fn signup_confirm(&self, address: &ServerAddress, _: &SignUp<'_>) -> AppResult<LoginGrant> {
        self.log("signup_confirm", &address.url)?;
        match self.outcome(&address.url)? {
            LoginOutcome::Signed(grant) => Ok(grant),
            _ => Err(AppError::new("server_error", "x")),
        }
    }
    fn reset_start(&self, address: &ServerAddress, _: &str) -> AppResult<()> {
        self.log("reset_start", &address.url)
    }
    fn reset_confirm(&self, address: &ServerAddress, _: &str, _: &str, _: &str) -> AppResult<()> {
        self.log("reset_confirm", &address.url)
    }
    fn browser_sign_in(&self, address: &ServerAddress, provider: &str, reauth: bool, _: Option<&str>, _: &std::sync::atomic::AtomicBool) -> AppResult<BrowserTicket> {
        let kind = if reauth { "confirm" } else { "browser_sign_in" };
        self.log(&format!("{kind}({provider})"), &address.url)?;
        Ok(BrowserTicket { code: "c".repeat(64), verifier: "v".repeat(43) })
    }
    fn redeem_browser(&self, address: &ServerAddress, ticket: &BrowserTicket, _: &str) -> AppResult<LoginOutcome> {
        assert_eq!(ticket.code, "c".repeat(64));
        self.log("redeem_browser", &address.url)?;
        self.outcome(&address.url)
    }
    fn browser_disable_device(&self, address: &ServerAddress, _: &BrowserTicket, device_id: &str) -> AppResult<()> {
        self.log(&format!("browser_disable_device({device_id})"), &address.url)?;
        *self.answer.lock().unwrap() = Answer::Grant;
        Ok(())
    }
    fn account(&self, endpoint: &RemoteEndpoint) -> AppResult<AccountInfo> {
        self.log("account", &endpoint.url)?;
        self.account.lock().unwrap().clone().ok_or_else(|| AppError::new("not_available", "older server"))
    }
    fn checkout(&self, endpoint: &RemoteEndpoint, plan_id: &str) -> AppResult<BillingPage> {
        self.log(&format!("checkout({plan_id})"), &endpoint.url)?;
        Ok(BillingPage { id: "s1".into(), url: self.page_url.lock().unwrap().clone(), expires_at: "2026-11-16T10:00:00Z".into() })
    }
    fn delete_account_reauthenticated(&self, endpoint: &RemoteEndpoint, _: &BrowserTicket) -> AppResult<()> {
        self.log("delete_account_reauthenticated", &endpoint.url)
    }
}

fn account_with(billing_enabled: bool, entitled: Option<bool>) -> AccountInfo {
    AccountInfo {
        user_id: "u1".into(),
        username: "mario".into(),
        email: Some("mario@example.com".into()),
        has_password: false,
        identities: vec!["google".into()],
        billing_enabled,
        subscription: entitled.map(|entitled| Subscription {
            status: if entitled { "active".into() } else { "expired".into() },
            entitled,
            plan_id: Some("pro-monthly".into()),
            plan_name: Some("Pro".into()),
            provider: Some("fake".into()),
            current_period_end: None,
            cancel_at_period_end: false,
            manageable: entitled,
            usage: None,
        }),
    }
}

const SERVER: &str = "https://calcolo.ufficio.it";

fn setup(edition: Edition) -> (ServerAccountService, Arc<FakeServer>, Arc<Vault>, Arc<ServiceContext>) {
    let vault = Arc::new(Vault::default());
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        repository: Arc::new(MemoryRepository::new(Store::default())),
        mail: Arc::new(NoMail),
        secrets: vault.clone(),
        clock: Arc::new(Fixed),
        attachments: Arc::new(NoFiles),
        events: Arc::new(NullSink),
    });
    let server = FakeServer::new();
    (ServerAccountService::new(context.clone(), server.clone(), edition), server, vault, context)
}
fn credentials() -> Credentials<'static> {
    Credentials { identifier: "mario", password: "una password lunga", device_name: "Qamille su Windows" }
}
fn professional() -> Edition {
    Edition::Professional { server: SERVER.into() }
}

#[test]
fn a_probe_tells_what_is_at_an_address() {
    let (service, server, _, _) = setup(Edition::SelfHosted);
    assert!(matches!(service.probe("HTTPS://Calcolo.Ufficio.it/").unwrap(), Probe::Found { .. }));
    assert_eq!(service.probe("http://calcolo.ufficio.it").unwrap(), Probe::HttpsRequired);
    assert_eq!(service.probe("calcolo").unwrap(), Probe::Invalid);
    *server.offline.lock().unwrap() = true;
    assert_eq!(service.probe(SERVER).unwrap(), Probe::Unreachable);
    assert_eq!(server.calls(), [format!("info {SERVER}"), format!("info {SERVER}")]);
}

#[test]
fn without_consent_nothing_but_the_probe_reaches_the_server() {
    // INV-10
    let (service, server, vault, _) = setup(Edition::SelfHosted);
    let error = service.connect(SERVER, &credentials()).err().unwrap();
    assert_eq!(error.code, "consent_required");
    assert!(server.calls().is_empty() && *vault.sets.lock().unwrap() == 0);
}

#[test]
fn a_sign_in_keeps_one_session_and_switches_to_the_server() {
    let (service, server, vault, context) = setup(Edition::SelfHosted);
    service.record_consent(SERVER).unwrap();
    let ConnectOutcome::Connected { status } = service.connect(SERVER, &credentials()).unwrap() else { panic!("not connected") };
    assert_eq!(*vault.sets.lock().unwrap(), 1, "one set of the session");
    assert!(status.connected && status.url.as_deref() == Some(SERVER) && status.username.as_deref() == Some("mario"));
    assert_eq!(context.read().unwrap().settings.compute_mode, ComputeMode::Server);
    assert!(!serde_json::to_string(&status).unwrap().contains("spk_dev_"), "the status never carries the key");
    service.devices().unwrap();
    assert_eq!(server.calls().last().unwrap(), &format!("devices {SERVER}"), "account calls go to the session's address");
}

#[test]
fn a_temporary_password_or_the_device_limit_write_nothing() {
    let (service, server, vault, context) = setup(Edition::SelfHosted);
    service.record_consent(SERVER).unwrap();
    *server.answer.lock().unwrap() = Answer::Temporary;
    assert_eq!(service.connect(SERVER, &credentials()).unwrap(), ConnectOutcome::PasswordChangeRequired);
    *server.answer.lock().unwrap() = Answer::Limit;
    let ConnectOutcome::DeviceLimit { devices } = service.connect(SERVER, &credentials()).unwrap() else { panic!() };
    assert_eq!(devices.len(), 1);
    *server.answer.lock().unwrap() = Answer::Wrong;
    assert_eq!(service.connect(SERVER, &credentials()).err().unwrap().code, "invalid_credentials");
    assert_eq!(*vault.sets.lock().unwrap(), 0);
    assert_eq!(context.read().unwrap().settings.compute_mode, ComputeMode::Off);
    // EX-11: the temporary password is replaced and the device registered in one call.
    *server.answer.lock().unwrap() = Answer::Temporary;
    assert!(matches!(service.change_temporary_password(SERVER, &credentials(), "nuova password lunga", false).unwrap(), ConnectOutcome::Connected { .. }));
    assert_eq!(*vault.sets.lock().unwrap(), 1);
}

#[test]
fn a_temporary_password_after_a_change_of_server_follows_the_rules_of_the_change() {
    let (service, server, vault, _) = setup(Edition::SelfHosted);
    let other = "https://nuovo.ufficio.it";
    service.record_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    // Same address: refused before anything is sent.
    let before = server.calls().len();
    assert_eq!(service.change_temporary_password("https://Calcolo.Ufficio.it", &credentials(), "nuova password lunga", true).err().unwrap().code, "same_server");
    assert_eq!(service.change_temporary_password(other, &credentials(), "nuova password lunga", true).err().unwrap().code, "consent_required");
    assert_eq!(server.calls().len(), before, "nothing reached the server");
    service.record_consent(other).unwrap();
    *server.answer.lock().unwrap() = Answer::Temporary;
    service.change_temporary_password(other, &credentials(), "nuova password lunga", true).unwrap();
    assert_eq!(service.session().unwrap().unwrap().url, other);
    assert_eq!(*vault.sets.lock().unwrap(), 2);
    assert!(server.calls().contains(&format!("sign_out(30s) {SERVER}")), "the old device signed out: {:?}", server.calls());

    // Without the flag the old session is not touched.
    let (service, server, _, _) = setup(Edition::SelfHosted);
    service.record_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    service.record_consent(other).unwrap();
    service.change_temporary_password(other, &credentials(), "nuova password lunga", false).unwrap();
    assert!(!server.calls().iter().any(|c| c.starts_with("sign_out")));

    // A preset edition cannot change server at all.
    let (service, server, _, _) = setup(professional());
    service.record_consent(SERVER).unwrap();
    assert_eq!(service.change_temporary_password(SERVER, &credentials(), "nuova password lunga", true).err().unwrap().code, "not_available");
    assert!(server.calls().is_empty());
}

#[test]
fn changing_server_keeps_the_old_session_until_the_new_one_works() {
    let (service, server, vault, _) = setup(Edition::SelfHosted);
    let other = "https://nuovo.ufficio.it";
    service.record_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    assert_eq!(service.change_server("https://Calcolo.Ufficio.it", &credentials()).err().unwrap().code, "same_server");
    assert_eq!(service.change_server(other, &credentials()).err().unwrap().code, "consent_required");
    service.record_consent(other).unwrap();
    *server.answer.lock().unwrap() = Answer::Wrong;
    assert!(service.change_server(other, &credentials()).is_err());
    assert_eq!(service.session().unwrap().unwrap().url, SERVER, "the old session is intact");
    *server.answer.lock().unwrap() = Answer::Grant;
    service.change_server(other, &credentials()).unwrap();
    assert_eq!(service.session().unwrap().unwrap().url, other);
    assert_eq!(*vault.sets.lock().unwrap(), 2, "one set per session");
    assert!(server.calls().contains(&format!("sign_out(30s) {SERVER}")), "the old device signed out: {:?}", server.calls());
}

#[test]
fn signing_out_offline_removes_the_session_and_says_so() {
    let (service, server, vault, context) = setup(Edition::SelfHosted);
    service.record_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    service.set_mode(ComputeMode::Off).unwrap();
    assert!(service.session().unwrap().is_some(), "local mode keeps the session");
    service.set_mode(ComputeMode::Server).unwrap();
    *server.offline.lock().unwrap() = true;
    assert_eq!(service.sign_out().unwrap(), SignOut::SignedOutOffline);
    assert!(service.session().unwrap().is_none() && vault.get(SERVER_SESSION).unwrap().is_empty());
    assert_eq!(context.read().unwrap().settings.compute_mode, ComputeMode::Off);
    assert_eq!(service.set_mode(ComputeMode::Server).err().unwrap().code, "not_connected");
}

#[test]
fn deleting_the_account_signs_out() {
    let (service, server, _, context) = setup(Edition::SelfHosted);
    service.record_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    service.delete_account("una password lunga").unwrap();
    assert!(server.calls().contains(&format!("delete_account {SERVER}")));
    assert!(service.session().unwrap().is_none());
    assert_eq!(context.read().unwrap().settings.compute_mode, ComputeMode::Off);
}

#[test]
fn account_info_carries_the_usage() {
    let (service, server, _, _) = setup(professional());
    service.record_consent(SERVER).unwrap();
    let mut account = account_with(true, Some(true));
    let meter = |used, limit| qamille::domain::server_account::Meter { used, limit };
    account.subscription.as_mut().unwrap().usage = Some(qamille::domain::server_account::UsageInfo {
        period_start: "2026-11-01".into(),
        renews_at: "2026-12-01T00:00:00.000Z".into(),
        audio_minutes: meter(12, 600),
        embedded_tokens: meter(1_000, 2_000_000),
        llm_tokens: meter(100, 100),
    });
    *server.account.lock().unwrap() = Some(account);
    service.connect(SERVER, &credentials()).unwrap();
    let usage = service.account_info().unwrap().subscription.unwrap().usage.unwrap();
    assert_eq!(usage.renews_at, "2026-12-01T00:00:00.000Z");
    assert_eq!(usage.llm_tokens, meter(100, 100));
}

#[test]
fn deleting_an_account_with_a_paid_subscription_is_still_a_normal_delete() {
    // The app only asks the server to delete: ending the subscription at the processor is the
    // server's job (INV-22), so no other call leaves this computer.
    let (service, server, _, context) = setup(professional());
    service.record_consent(SERVER).unwrap();
    *server.account.lock().unwrap() = Some(account_with(true, Some(true)));
    service.connect(SERVER, &credentials()).unwrap();
    let before = server.calls().len();
    service.delete_account("una password lunga").unwrap();
    assert_eq!(server.calls()[before..], [format!("delete_account {SERVER}")]);
    assert!(service.session().unwrap().is_none());
    assert_eq!(context.read().unwrap().settings.compute_mode, ComputeMode::Off);
}

#[test]
fn a_professional_build_uses_only_its_server() {
    // INV-17
    let (service, server, _, _) = setup(professional());
    assert_eq!(service.probe("https://altro.example").err().unwrap().code, "preset_only");
    assert_eq!(service.record_consent("https://altro.example").err().unwrap().code, "preset_only");
    service.record_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    assert_eq!(service.change_server("https://altro.example", &credentials()).err().unwrap().code, "not_available");
    assert!(!service.startup_check().unwrap(), "its own session stays");
    assert!(server.calls().iter().all(|call| call.ends_with(SERVER)));
}

#[test]
fn a_professional_start_removes_a_session_of_another_server() {
    // EX-20: a data folder that came from a self-hosted build.
    let (service, server, vault, context) = setup(professional());
    let session = r#"{"url":"https://vecchio.example","user_id":"u","username":"mario","device_id":"d","device_key":"spk_dev_x"}"#;
    vault.set(SERVER_SESSION, session).unwrap();
    context.update(|store| { store.settings.compute_mode = ComputeMode::Server; Ok(()) }).unwrap();
    assert!(service.startup_check().unwrap());
    assert_eq!(server.calls(), ["sign_out(5s) https://vecchio.example"]);
    assert!(service.session().unwrap().is_none());
    assert_eq!(context.read().unwrap().settings.compute_mode, ComputeMode::Off);
}

/// One HTTP answer per connection; returns the address and the raw requests received.
fn fake_http(answers: Vec<(u16, &'static str)>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let seen = Arc::new(Mutex::new(Vec::new()));
    let record = seen.clone();
    std::thread::spawn(move || {
        for (status, body) in answers {
            let Ok((mut stream, _)) = listener.accept() else { return };
            stream.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
            let mut request = Vec::new();
            let mut buffer = [0u8; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap_or(0);
                request.extend_from_slice(&buffer[..n]);
                let text = String::from_utf8_lossy(&request).to_string();
                if let Some(end) = text.find("\r\n\r\n") {
                    let length = text[..end].lines().find_map(|line| {
                        let (key, value) = line.split_once(':')?;
                        key.eq_ignore_ascii_case("content-length").then(|| value.trim().parse::<usize>().unwrap())
                    }).unwrap_or(0);
                    if request.len() >= end + 4 + length { break; }
                }
                if n == 0 { break; }
            }
            record.lock().unwrap().push(String::from_utf8_lossy(&request).to_string());
            let reply = format!("HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    (url, seen)
}

#[test]
fn http_account_routes_carry_the_device_key_only_to_the_session_and_map_errors() {
    let (url, seen) = fake_http(vec![
        (200, r#"{"version":"1.0.0","api":1,"signup":false,"password_reset":false,"edition":"self-hosted"}"#),
        (409, r#"{"error":{"code":"device_limit","message":"x","retry_after_s":null},"devices":[{"id":"a","name":"Vecchio","created_at":"2026-01-01","last_seen_at":null,"current":false}]}"#),
        (401, r#"{"error":{"code":"invalid_credentials","message":"Username o password non corretti.","retry_after_s":null}}"#),
        (200, r#"{"devices":[{"id":"d1","name":"Questo","created_at":"2026-01-01","last_seen_at":null,"current":true}]}"#),
        (429, r#"{"error":{"code":"rate_limited","message":"x","retry_after_s":12}}"#),
    ]);
    let http = HttpServerAccount;
    let address = ServerAddress { url: url.clone(), ca_pem: None };
    assert_eq!(http.info(&address).unwrap().api, 1);
    let LoginOutcome::DeviceLimit(devices) = http.login(&address, &credentials()).unwrap() else { panic!("not the device limit") };
    assert_eq!(devices[0].name, "Vecchio");
    let error = http.login(&address, &credentials()).err().unwrap();
    assert_eq!(error.code, "invalid_credentials");
    assert!(!error.message.contains("una password lunga"));
    let endpoint = RemoteEndpoint { url: url.clone(), device_key: "spk_dev_abc".into(), ca_pem: None, timeout: Duration::from_secs(5) };
    assert!(http.devices(&endpoint).unwrap()[0].current);
    let limited = http.change_password(&endpoint, "a", "b", true).err().unwrap();
    assert_eq!((limited.code, limited.message.as_str()), ("rate_limited", "Too many attempts. Try again in 12 seconds"));
    let requests = seen.lock().unwrap().clone();
    assert!(requests[0].starts_with("GET /v1/info "), "{}", requests[0]);
    assert!(!requests[0].to_ascii_lowercase().contains("authorization"), "the probe carries no credential");
    assert!(requests[1].starts_with("POST /v1/login ") && requests[1].contains("\"device_name\""));
    assert!(requests[3].starts_with("GET /v1/account/devices ") && requests[3].contains("Bearer spk_dev_abc"), "{}", requests[3]);
    assert!(requests[4].starts_with("PUT /v1/account/password ") && requests[4].replace(char::is_whitespace, "").contains("\"sign_out_other_devices\":true"), "{}", requests[4]);
    let unreachable = http.info(&ServerAddress { url: "http://127.0.0.1:9".into(), ca_pem: None }).err().unwrap();
    assert_eq!(unreachable.code, "unreachable");
}

#[test]
fn a_ca_written_for_a_new_server_never_covers_the_session_in_use() {
    // Security review: «Use this certificate» during «Change server».
    let (service, _, _, context) = setup(Edition::SelfHosted);
    let old_ca = "-----BEGIN CERTIFICATE-----\nOLD\n-----END CERTIFICATE-----";
    service.set_ca_pem(Some(old_ca.into())).unwrap();
    service.record_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    assert_eq!(context.read().unwrap().settings.compute_ca_pem.as_deref(), Some(old_ca), "saved with its session");
    service.set_ca_pem(Some("-----BEGIN CERTIFICATE-----\nNEW\n-----END CERTIFICATE-----".into())).unwrap();
    service.forget_pending_ca();
    assert_eq!(service.endpoint().unwrap().unwrap().ca_pem.as_deref(), Some(old_ca), "«Cancel» changes nothing");
    service.set_ca_pem(Some("-----BEGIN CERTIFICATE-----\nNEW\n-----END CERTIFICATE-----".into())).unwrap();
    assert_eq!(service.endpoint().unwrap().unwrap().ca_pem.as_deref(), Some(old_ca), "the session keeps its own CA");
}

fn signup() -> SignUp<'static> {
    SignUp { email: "anna@example.com", code: "12345678", password: "una password lunga", username: None, device_name: "Qamille", accepted_terms: None }
}

#[test]
fn sign_up_is_professional_only_and_needs_consent() {
    // INV-17, INV-10, EX-12
    let (selfhosted, server, _, _) = setup(Edition::SelfHosted);
    selfhosted.record_consent(SERVER).unwrap();
    assert_eq!(selfhosted.signup_start(SERVER, "anna@example.com").err().unwrap().code, "not_available");
    assert_eq!(selfhosted.signup_confirm(SERVER, &signup()).err().unwrap().code, "not_available");
    assert!(server.calls().is_empty());
    let (service, server, vault, context) = setup(professional());
    assert_eq!(service.signup_start(SERVER, "anna@example.com").err().unwrap().code, "consent_required");
    service.record_consent(SERVER).unwrap();
    service.signup_start(SERVER, " anna@example.com ").unwrap();
    let status = service.signup_confirm(SERVER, &signup()).unwrap();
    assert!(status.connected);
    assert_eq!(*vault.sets.lock().unwrap(), 1, "one set of the session, like connect");
    assert_eq!(context.read().unwrap().settings.compute_mode, ComputeMode::Server);
    // After the session is kept, one look at the account decides local or remote (subscriptions).
    assert_eq!(server.calls(), [format!("signup_start {SERVER}"), format!("signup_confirm {SERVER}"), format!("account {SERVER}")]);
}

#[test]
fn a_password_reset_creates_no_session() {
    let (service, server, vault, _) = setup(Edition::SelfHosted);
    // Like every request that carries something of the person, it waits for the consent.
    assert_eq!(service.reset_start(SERVER, "anna@example.com").err().unwrap().code, "consent_required");
    assert_eq!(service.reset_confirm(SERVER, "anna@example.com", "12345678", "una password nuova").err().unwrap().code, "consent_required");
    assert!(server.calls().is_empty(), "nothing before consent (INV-10)");
    service.record_consent(SERVER).unwrap();
    service.reset_start(SERVER, "anna@example.com").unwrap();
    service.reset_confirm(SERVER, "anna@example.com", "12345678", "una password nuova").unwrap();
    assert!(service.session().unwrap().is_none() && *vault.sets.lock().unwrap() == 0);
    assert_eq!(server.calls().len(), 2);
}

/// Answers every request with a redirect to `target` and records nothing else.
fn redirecting(target: String) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            stream.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let reply = format!("HTTP/1.1 307 Moved\r\nLocation: {target}/v1/account/devices\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    url
}

#[test]
fn a_redirect_never_carries_the_device_key_or_a_password_elsewhere() {
    let (elsewhere, seen) = fake_http(vec![(200, r#"{"devices":[]}"#), (200, "{}")]);
    let url = redirecting(elsewhere);
    let http = HttpServerAccount;
    let endpoint = RemoteEndpoint { url: url.clone(), device_key: "spk_dev_abc".into(), ca_pem: None, timeout: Duration::from_secs(5) };
    assert!(http.devices(&endpoint).is_err());
    assert!(http.login(&ServerAddress { url, ca_pem: None }, &credentials()).is_err());
    std::thread::sleep(Duration::from_millis(200));
    assert!(seen.lock().unwrap().is_empty(), "nothing followed the redirect");
}

#[test]
fn the_saved_ca_belongs_to_its_address_only() {
    let (service, _, _, context) = setup(Edition::SelfHosted);
    let ca = "-----BEGIN CERTIFICATE-----\nCA\n-----END CERTIFICATE-----";
    service.set_ca_pem(Some(ca.into())).unwrap();
    service.record_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    let settings = context.read().unwrap().settings;
    assert_eq!(settings.compute_ca_url.as_deref(), Some(SERVER), "saved with its address");
    assert_eq!(qamille::application::server_account::saved_ca(&settings, SERVER).as_deref(), Some(ca));
    assert_eq!(qamille::application::server_account::saved_ca(&settings, "https://altro.example"), None);
    let mut unbound = settings.clone();
    unbound.compute_ca_url = None;
    assert_eq!(qamille::application::server_account::saved_ca(&unbound, SERVER), None, "an authority without its address covers nothing");
}

// --- Vendita Professional: accesso dal browser, abbonamento -----------------------------------

#[test]
fn a_browser_sign_in_needs_consent_and_keeps_the_session_like_a_password() {
    let (service, server, vault, context) = setup(professional());
    let refused = service.connect_with_browser(SERVER, "google", "Qamille Professional su Windows", false, None).unwrap_err();
    assert_eq!(refused.code, "consent_required");
    assert!(server.calls().is_empty(), "nothing before consent (INV-10)");
    assert_eq!(service.connect_with_browser("https://altro.example", "google", "x", false, None).unwrap_err().code, "preset_only");
    assert_eq!(service.connect_with_browser(SERVER, "github", "x", false, None).unwrap_err().code, "validation");

    service.record_consent(SERVER).unwrap();
    let outcome = service.connect_with_browser(SERVER, "google", "Qamille Professional su Windows", false, None).unwrap();
    assert!(matches!(outcome, ConnectOutcome::Connected { .. }));
    assert_eq!(server.calls()[..2], [format!("browser_sign_in(google) {SERVER}"), format!("redeem_browser {SERVER}")]);
    assert!(vault.get(SERVER_SESSION).unwrap().contains("spk_dev_"));
    // An older server without `GET /v1/account`: remote as before.
    assert_eq!(context.read_shared().unwrap().settings.compute_mode, ComputeMode::Server);
}

#[test]
fn at_the_device_limit_the_browser_code_disables_one_and_finishes() {
    let (service, server, vault, _) = setup(professional());
    service.record_consent(SERVER).unwrap();
    *server.answer.lock().unwrap() = Answer::Limit;
    let outcome = service.connect_with_browser(SERVER, "google", "PC", false, None).unwrap();
    assert!(matches!(outcome, ConnectOutcome::DeviceLimit { ref devices } if devices[0].id == "old"));
    assert!(vault.get(SERVER_SESSION).unwrap().is_empty());
    let outcome = service.browser_disable_device("old").unwrap();
    assert!(matches!(outcome, ConnectOutcome::Connected { .. }));
    assert!(server.calls().contains(&format!("browser_disable_device(old) {SERVER}")));
    // The pending code was used: a second disable has nothing to work with.
    assert_eq!(service.browser_disable_device("old").unwrap_err().code, "invalid_code");
}

#[test]
fn a_transient_error_keeps_the_pending_browser_code_and_a_refusal_drops_it() {
    let (service, server, _, _) = setup(professional());
    service.record_consent(SERVER).unwrap();
    *server.answer.lock().unwrap() = Answer::Limit;
    service.connect_with_browser(SERVER, "google", "PC", false, None).unwrap();
    // The server does not answer: the code was not used, the person can try again.
    *server.offline.lock().unwrap() = true;
    assert_eq!(service.browser_disable_device("old").unwrap_err().code, "unreachable");
    *server.offline.lock().unwrap() = false;
    *server.answer.lock().unwrap() = Answer::Grant;
    assert!(matches!(service.browser_disable_device("old").unwrap(), ConnectOutcome::Connected { .. }));
}

#[test]
fn without_a_subscription_the_computation_stays_on_this_computer() {
    let (service, server, _, context) = setup(professional());
    service.record_consent(SERVER).unwrap();
    *server.account.lock().unwrap() = Some(account_with(true, Some(false)));
    service.connect_with_browser(SERVER, "google", "PC", false, None).unwrap();
    assert_eq!(context.read_shared().unwrap().settings.compute_mode, ComputeMode::Off);
    assert_eq!(service.account_info().unwrap().subscription.unwrap().status, "expired");

    // With one, or on a server that sells nothing, remote as in A8.
    for account in [account_with(true, Some(true)), account_with(false, None)] {
        service.sign_out().unwrap();
        *server.account.lock().unwrap() = Some(account);
        service.connect(SERVER, &credentials()).unwrap();
        assert_eq!(context.read_shared().unwrap().settings.compute_mode, ComputeMode::Server);
    }
}

#[test]
fn billing_pages_open_only_on_safe_addresses() {
    let (service, server, _, _) = setup(professional());
    service.record_consent(SERVER).unwrap();
    assert_eq!(service.start_checkout("pro-monthly").unwrap_err().code, "not_connected");
    service.connect(SERVER, &credentials()).unwrap();
    assert_eq!(service.start_checkout("pro-monthly").unwrap().url, "https://checkout.processor.example/c/1");
    for bad in [
        "file:///C:/Windows/System32/calc.exe",
        "javascript:alert(1)",
        "http://processor.example/pay",
        "https://user@evil.example/",
        "ms-settings:",
        "http://127.0.0.1.evil.example/pay",
        "http://localhost.evil.example:8787/",
        "https://evil.example\\@good.example/",
    ] {
        *server.page_url.lock().unwrap() = bad.into();
        assert_eq!(service.start_checkout("pro-monthly").unwrap_err().code, "invalid_url", "{bad}");
    }
    assert!(browser_url("http://127.0.0.1:8787/billing/fake/checkout/x").is_ok());
    assert!(browser_url("http://localhost/x").is_ok());
    assert!(browser_url("https://checkout.processor.example:443/c/1?x=1").is_ok());
    assert_eq!(service.checkout_status("../admin").unwrap_err().code, "validation");
}

#[test]
fn an_account_without_password_is_deleted_by_signing_in_again() {
    let (service, server, vault, context) = setup(professional());
    service.record_consent(SERVER).unwrap();
    service.connect_with_browser(SERVER, "google", "PC", false, None).unwrap();
    service.delete_account_with_browser("google").unwrap();
    // A confirmation, not a sign-in: the server creates or links nothing, Google asks again.
    assert!(server.calls().ends_with(&[format!("confirm(google) {SERVER}"), format!("delete_account_reauthenticated {SERVER}")]));
    assert!(vault.get(SERVER_SESSION).unwrap().is_empty());
    assert_eq!(context.read_shared().unwrap().settings.compute_mode, ComputeMode::Off);
}

#[test]
fn a_browser_sign_in_can_change_server_only_in_the_self_hosted_build() {
    let (service, server, _, _) = setup(Edition::SelfHosted);
    service.record_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    let other = "https://altro.ufficio.it";
    service.record_consent(other).unwrap();
    service.connect_with_browser(other, "google", "PC", true, None).unwrap();
    assert!(server.calls().contains(&format!("sign_out(30s) {SERVER}")), "{:?}", server.calls());
    assert_eq!(service.status().unwrap().url.as_deref(), Some(other));
    let (service, _, _, _) = setup(professional());
    assert_eq!(service.connect_with_browser(SERVER, "google", "PC", true, None).unwrap_err().code, "not_available");
}

// ---- Wave 5: the assistant of the server -------------------------------------------------

fn llm(label: &str, external: bool) -> LlmInfo {
    LlmInfo { model: "m".into(), models: vec!["m".into()], external, label: label.into(), context_tokens: Some(8192) }
}

#[test]
fn record_llm_consent_stores_what_the_server_declares() {
    let (service, server, _, context) = setup(Edition::SelfHosted);
    // Without consent for the address nothing is read.
    assert_eq!(service.record_llm_consent(SERVER).err().unwrap().code, "consent_required");
    service.record_consent(SERVER).unwrap();
    service.record_llm_consent(SERVER).unwrap();
    assert_eq!(context.read().unwrap().settings.llm_consent, None, "a server without llm: no consent");
    *server.llm.lock().unwrap() = Some(llm("OpenAI", true));
    service.record_llm_consent(SERVER).unwrap();
    assert_eq!(
        context.read().unwrap().settings.llm_consent,
        Some(LlmConsent { url: SERVER.into(), label: "OpenAI".into(), external: true })
    );
}

#[test]
fn a_changed_provider_needs_a_new_consent() {
    let (service, server, _, _) = setup(Edition::SelfHosted);
    let service = service.with_llm_info_ttl(Duration::ZERO);
    *server.llm.lock().unwrap() = Some(llm("OpenAI", true));
    service.record_consent(SERVER).unwrap();
    service.record_llm_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    assert_eq!(service.llm_status().unwrap(), LlmStatus::Ready { label: "OpenAI".into(), external: true });
    *server.llm.lock().unwrap() = Some(llm("Ollama", false));
    assert_eq!(service.llm_status().unwrap(), LlmStatus::ConsentNeeded { label: "Ollama".into(), external: false });
    service.record_llm_consent(SERVER).unwrap();
    assert_eq!(service.llm_status().unwrap(), LlmStatus::Ready { label: "Ollama".into(), external: false });
    *server.llm.lock().unwrap() = None;
    assert_eq!(service.llm_status().unwrap(), LlmStatus::Unavailable);
}

#[test]
fn llm_status_rereads_info_at_most_every_ten_minutes() {
    let (service, server, _, _) = setup(Edition::SelfHosted);
    *server.llm.lock().unwrap() = Some(llm("OpenAI", true));
    service.record_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    let infos = |server: &FakeServer| server.calls().iter().filter(|c| c.starts_with("info")).count();
    service.llm_status().unwrap();
    service.llm_status().unwrap();
    assert_eq!(infos(&server), 1);
    let (service, server, _, _) = setup(Edition::SelfHosted);
    let service = service.with_llm_info_ttl(Duration::ZERO);
    service.record_consent(SERVER).unwrap();
    service.connect(SERVER, &credentials()).unwrap();
    service.llm_status().unwrap();
    service.llm_status().unwrap();
    assert_eq!(infos(&server), 2);
}

#[test]
fn connecting_to_a_server_with_llm_makes_it_the_default_assistant() {
    for (before, after) in [(AiProvider::Offline, AiProvider::QamilleServer), (AiProvider::Anthropic, AiProvider::Anthropic)] {
        let (service, server, _, context) = setup(Edition::SelfHosted);
        *server.llm.lock().unwrap() = Some(llm("Server", false));
        context.update(|store| { store.settings.ai_provider = before; Ok(()) }).unwrap();
        service.record_consent(SERVER).unwrap();
        service.record_llm_consent(SERVER).unwrap();
        service.connect(SERVER, &credentials()).unwrap();
        assert_eq!(context.read().unwrap().settings.ai_provider, after);
    }
}

#[test]
fn signing_out_removes_the_llm_consent() {
    for (provider, after) in [(AiProvider::QamilleServer, AiProvider::Offline), (AiProvider::Anthropic, AiProvider::Anthropic)] {
        let (service, server, _, context) = setup(Edition::SelfHosted);
        *server.llm.lock().unwrap() = Some(llm("Server", false));
        service.record_consent(SERVER).unwrap();
        service.record_llm_consent(SERVER).unwrap();
        service.connect(SERVER, &credentials()).unwrap();
        context.update(|store| { store.settings.ai_provider = provider; Ok(()) }).unwrap();
        service.sign_out().unwrap();
        let settings = context.read().unwrap().settings.clone();
        assert_eq!(settings.llm_consent, None);
        assert_eq!(settings.ai_provider, after);
    }
}

#[test]
fn a_settings_update_does_not_take_back_the_llm_consent() {
    let (_, _, _, context) = setup(Edition::SelfHosted);
    let consent = LlmConsent { url: SERVER.into(), label: "OpenAI".into(), external: true };
    context.update(|store| { store.settings.llm_consent = Some(consent.clone()); store.settings.user_name = "Mario".into(); Ok(()) }).unwrap();
    let mut settings = context.read().unwrap().settings.clone();
    settings.llm_consent = None;
    qamille::application::workspace::WorkspaceService::new(context.clone())
        .save_settings(qamille::application::workspace::SettingsUpdate { settings, ai_key: None, google_client_secret: None })
        .unwrap();
    assert_eq!(context.read().unwrap().settings.llm_consent, Some(consent));
}

#[test]
fn a_provider_label_with_a_line_break_is_never_recorded() {
    let (service, server, _, context) = setup(Edition::SelfHosted);
    *server.llm.lock().unwrap() = Some(llm("Evil
X-Injected: 1", true));
    service.record_consent(SERVER).unwrap();
    assert_eq!(service.record_llm_consent(SERVER).err().unwrap().code, "invalid_provider");
    assert_eq!(context.read().unwrap().settings.llm_consent, None);
}

#[test]
fn an_account_never_shows_its_refresh_token_or_app_password_in_debug() {
    let account = Account { email: "anna@example.com".into(), password: "REFRESH-TOKEN-SECRET".into(), ..Default::default() };
    let shown = format!("{account:?} {account:#?}");
    assert!(!shown.contains("SECRET"), "{shown}");
    assert!(shown.contains("anna@example.com") && shown.contains("<hidden>"));
}

#[test]
fn connection_consent_saves_only_the_disclosures_that_were_shown() {
    let (service, server, _, context) = setup(professional());
    let Probe::Found { info: mut shown } = service.probe(SERVER).unwrap() else { panic!("missing server") };
    shown.sync = Some(qamille::domain::server_account::SyncInfo { available: true });
    let changed = service.confirm_connection(SERVER, &shown).unwrap().expect("mailbox disclosure changed");
    assert!(context.read().unwrap().settings.compute_consented_urls.is_empty());

    *server.llm.lock().unwrap() = Some(llm("External provider", true));
    let latest = service.confirm_connection(SERVER, &changed).unwrap().expect("assistant disclosure changed");
    let settings = context.read().unwrap().settings;
    assert!(settings.llm_consent.is_none() && !settings.mailbox_sync);
    assert!(settings.compute_consented_urls.is_empty());

    assert!(service.confirm_connection(SERVER, &latest).unwrap().is_none());
    let settings = context.read().unwrap().settings;
    assert_eq!(settings.compute_consented_urls, vec![SERVER.to_string()]);
    assert_eq!(settings.llm_consent.unwrap().label, "External provider");
    assert!(!settings.mailbox_sync);
}
