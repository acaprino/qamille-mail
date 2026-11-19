//! Doubles shared by the synchronization tests: an in-memory server for `/v1/sync/items` with
//! versions, conflicts, tombstones and epochs, and a complete «app» (repository, vault, session).
#![allow(dead_code)]
use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        accounts::AccountService,
        ports::{AttachmentGateway, Clock, MailGateway, NullSink, OutgoingMessage, RemoteAction, SecretStore, SyncBatch},
        server_account::{
            Credentials, LoginOutcome, RemoteEndpoint, SERVER_SESSION, ServerAccount, ServerAccountService, ServerAddress,
        },
        sync::{GoogleClients, Listing, PutOutcome, RemoteItem, SyncServer, SyncService, SyncState, SyncStateStore, TokenRevoker},
    },
    domain::{
        model::{Account, AccountKind, Attachment, Store},
        server_account::{DeviceInfo, ServerInfo, ServerSession, SyncInfo},
        sync::{SyncItem, item_id},
    },
    edition::Edition,
    infrastructure::repository::MemoryRepository,
};
use std::{
    collections::{BTreeMap, HashMap},
    sync::{
        Arc, Mutex,
        atomic::{AtomicI64, Ordering},
    },
};

pub const URL: &str = "http://127.0.0.1:8787";

#[derive(Default)]
pub struct Vault {
    pub values: Mutex<HashMap<String, String>>,
    /// Reading these keys fails (a vault that does not answer for one mailbox).
    pub broken: Mutex<Vec<String>>,
}
impl SecretStore for Vault {
    fn get(&self, key: &str) -> AppResult<String> {
        if self.broken.lock().unwrap().iter().any(|k| k == key) {
            return Err(AppError::storage("The vault does not answer"));
        }
        Ok(self.values.lock().unwrap().get(key).cloned().unwrap_or_default())
    }
    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        self.values.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
}
pub struct NoMail;
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
pub struct NoFiles;
impl AttachmentGateway for NoFiles {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}
pub struct TestClock(pub AtomicI64);
impl Clock for TestClock {
    fn now(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }
}
#[derive(Default)]
pub struct Events(pub Mutex<Vec<String>>);
impl qamille::application::ports::EventSink for Events {
    fn emit(&self, event: qamille::application::events::AppEvent) {
        self.0.lock().unwrap().push(format!("{event:?}"));
    }
}

/// `GET /v1/info` and nothing else: the sign-in itself is not under test here.
pub struct FakeAccountServer {
    pub sync: Mutex<bool>,
}
impl ServerAccount for FakeAccountServer {
    fn info(&self, _: &ServerAddress) -> AppResult<ServerInfo> {
        Ok(ServerInfo {
            version: "1.0".into(),
            api: 1,
            signup: false,
            password_reset: false,
            edition: "self-hosted".into(),
            login_providers: vec![],
            google_signup: false,
            billing: None,
            llm: None,
            sync: (*self.sync.lock().unwrap()).then_some(SyncInfo { available: true }),
            legal: None,
        })
    }
    fn login(&self, _: &ServerAddress, _: &Credentials<'_>) -> AppResult<LoginOutcome> {
        Err(AppError::new("unreachable", "no sign-in in this fake"))
    }
    fn change_temporary_password(&self, _: &ServerAddress, _: &Credentials<'_>, _: &str) -> AppResult<LoginOutcome> {
        Err(AppError::new("unreachable", "no sign-in in this fake"))
    }
    fn disable_device_with_password(&self, _: &ServerAddress, _: &str, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
    fn devices(&self, _: &RemoteEndpoint) -> AppResult<Vec<DeviceInfo>> {
        Ok(vec![])
    }
    fn revoke_device(&self, _: &RemoteEndpoint, _: &str) -> AppResult<()> {
        Ok(())
    }
    fn change_password(&self, _: &RemoteEndpoint, _: &str, _: &str, _: bool) -> AppResult<()> {
        Ok(())
    }
    fn sign_out(&self, _: &RemoteEndpoint) -> AppResult<()> {
        Ok(())
    }
    fn delete_account(&self, _: &RemoteEndpoint, _: &str) -> AppResult<()> {
        Ok(())
    }
}

pub struct Row {
    pub version: u64,
    pub deleted: bool,
    pub item: Option<SyncItem>,
}
#[derive(Default)]
pub struct ServerData {
    pub rows: BTreeMap<String, Row>,
    pub counter: u64,
    pub epoch: u64,
    pub calls: Vec<String>,
    /// The next call fails with this error code.
    pub fail: Option<&'static str>,
    /// Every `PUT` body, in order: what the server saw.
    pub bodies: Vec<String>,
    pub endpoints: Vec<String>,
    /// `PUT` of these ids fails with this code (a `400`, a `413`, a limit).
    pub fail_put: BTreeMap<String, &'static str>,
    /// Runs once, just before the next listing: another actor's change in the middle of a pass.
    pub before_list: Option<Box<dyn FnOnce() + Send>>,
}
/// The server of `/v1/sync/items` in memory; shared by the «apps» of one test.
#[derive(Default)]
pub struct FakeSync(pub Mutex<ServerData>);
impl FakeSync {
    pub fn calls(&self) -> Vec<String> {
        self.0.lock().unwrap().calls.clone()
    }
    pub fn clear_calls(&self) {
        self.0.lock().unwrap().calls.clear();
    }
    pub fn fail_next(&self, code: &'static str) {
        self.0.lock().unwrap().fail = Some(code);
    }
    pub fn item(&self, id: &str) -> Option<(u64, SyncItem)> {
        let data = self.0.lock().unwrap();
        data.rows.get(id).and_then(|r| r.item.clone().map(|i| (r.version, i)))
    }
    /// A write by someone else: bumps the counter and replaces the row.
    pub fn write(&self, item: SyncItem) -> u64 {
        let mut data = self.0.lock().unwrap();
        data.counter += 1;
        let version = data.counter;
        data.rows.insert(item.id(), Row { version, deleted: false, item: Some(item) });
        version
    }
    pub fn remove(&self, id: &str) {
        let mut data = self.0.lock().unwrap();
        data.counter += 1;
        let version = data.counter;
        data.rows.insert(id.into(), Row { version, deleted: true, item: None });
    }
    /// `sync reset`: everything goes, the epoch moves, the counter does not.
    pub fn reset(&self) {
        let mut data = self.0.lock().unwrap();
        data.rows.clear();
        data.epoch += 1;
    }
    fn begin(&self, call: String, endpoint: &RemoteEndpoint) -> AppResult<std::sync::MutexGuard<'_, ServerData>> {
        let mut data = self.0.lock().unwrap();
        data.calls.push(call);
        data.endpoints.push(format!("{}|{}", endpoint.url, endpoint.device_key));
        match data.fail.take() {
            Some(code) => Err(AppError::new(code, "fake failure")),
            None => Ok(data),
        }
    }
}
fn remote(id: &str, row: &Row) -> RemoteItem {
    RemoteItem { id: id.into(), version: row.version, deleted: row.deleted, item: row.item.clone() }
}
impl SyncServer for FakeSync {
    fn list(&self, endpoint: &RemoteEndpoint, since: u64) -> AppResult<Listing> {
        let hook = self.0.lock().unwrap().before_list.take();
        if let Some(hook) = hook {
            hook();
        }
        let data = self.begin(format!("list {since}"), endpoint)?;
        let mut items: Vec<RemoteItem> = data.rows.iter().filter(|(_, r)| r.version > since).map(|(id, r)| remote(id, r)).collect();
        items.sort_by_key(|i| i.version);
        Ok(Listing { version: data.counter, epoch: data.epoch, items })
    }
    fn put(&self, endpoint: &RemoteEndpoint, id: &str, base_version: u64, item: &SyncItem) -> AppResult<PutOutcome> {
        let mut data = self.begin(format!("put {id} base {base_version}"), endpoint)?;
        if let Some(code) = data.fail_put.get(id).copied() {
            return Err(AppError::new(code, "fake refusal"));
        }
        data.bodies.push(serde_json::to_string(item).unwrap());
        let current = data.rows.get(id).map(|r| r.version).unwrap_or(0);
        if current != base_version {
            return Ok(PutOutcome::Conflict(data.rows.get(id).map(|r| remote(id, r))));
        }
        data.counter += 1;
        let version = data.counter;
        data.rows.insert(id.into(), Row { version, deleted: false, item: Some(item.clone()) });
        Ok(PutOutcome::Written { version })
    }
    fn delete(&self, endpoint: &RemoteEndpoint, id: &str, base_version: Option<u64>) -> AppResult<()> {
        let mut data = self.begin(format!("delete {id} base {base_version:?}"), endpoint)?;
        if data.rows.get(id).is_some_and(|r| !r.deleted) {
            data.counter += 1;
            let version = data.counter;
            data.rows.insert(id.into(), Row { version, deleted: true, item: None });
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct MemState(pub Mutex<SyncState>, pub std::sync::atomic::AtomicBool);
impl SyncStateStore for MemState {
    fn load(&self) -> AppResult<SyncState> {
        Ok(self.0.lock().unwrap().clone())
    }
    fn save(&self, state: &SyncState) -> AppResult<()> {
        if self.1.load(Ordering::SeqCst) {
            return Err(AppError::storage("The synchronization state could not be saved"));
        }
        *self.0.lock().unwrap() = state.clone();
        Ok(())
    }
}
/// Only `app.apps.googleusercontent.com` belongs to the build.
pub struct AppClients;
impl GoogleClients for AppClients {
    fn is_app_client(&self, client_id: &str) -> bool {
        client_id == "app.apps.googleusercontent.com"
    }
}

/// Google's revocation endpoint: remembers the tokens it was asked to revoke.
#[derive(Default)]
pub struct FakeRevoker {
    /// When set, the real `oauth::revoke` talks to this address (a scripted Google).
    pub google: Mutex<Option<String>>,
    pub fail: Mutex<bool>,
    pub revoked: Mutex<Vec<String>>,
}
impl TokenRevoker for FakeRevoker {
    fn revoke(&self, token: &str) -> AppResult<()> {
        if let Some(url) = self.google.lock().unwrap().clone() {
            qamille::oauth::revoke(&url, token).map_err(|_| AppError::new("revoke_failed", "Google did not revoke the access"))?;
            self.revoked.lock().unwrap().push(token.into());
            return Ok(());
        }
        if *self.fail.lock().unwrap() {
            return Err(AppError::new("revoke_failed", "Google did not revoke the access"));
        }
        self.revoked.lock().unwrap().push(token.into());
        Ok(())
    }
}

/// One computer: its own store, vault, state file and clock; the server may be shared.
pub struct App {
    pub context: Arc<ServiceContext>,
    pub vault: Arc<Vault>,
    pub events: Arc<Events>,
    pub clock: Arc<TestClock>,
    pub state: Arc<MemState>,
    pub accounts: Arc<ServerAccountService>,
    pub account_service: AccountService,
    pub sync: SyncService,
    pub info: Arc<FakeAccountServer>,
    pub revoker: Arc<FakeRevoker>,
}
impl App {
    pub fn new(server: &Arc<FakeSync>) -> App {
        let info = Arc::new(FakeAccountServer { sync: Mutex::new(true) });
        App::build(server.clone(), info.clone(), info)
    }
    /// A computer over any `SyncServer` and `ServerAccount` (the real ones in the two-app test).
    pub fn build(server: Arc<dyn SyncServer>, account: Arc<dyn ServerAccount>, info: Arc<FakeAccountServer>) -> App {
        let vault = Arc::new(Vault::default());
        let events = Arc::new(Events::default());
        let clock = Arc::new(TestClock(AtomicI64::new(1_800_000_000)));
        let mut store = Store::default();
        store.settings.user_name = "Giulia".into();
        let context = Arc::new(ServiceContext {
            repository: Arc::new(MemoryRepository::new(store)),
            mail: Arc::new(NoMail),
            secrets: vault.clone(),
            clock: clock.clone(),
            attachments: Arc::new(NoFiles),
            events: events.clone(),
            compute: Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        });
        let accounts = Arc::new(ServerAccountService::new(context.clone(), account, Edition::SelfHosted));
        let state = Arc::new(MemState::default());
        let revoker = Arc::new(FakeRevoker::default());
        let sync = SyncService::new(context.clone(), accounts.clone(), server, state.clone(), Arc::new(AppClients)).with_revoker(revoker.clone());
        let identity: Arc<dyn qamille::application::ports::GoogleIdentity> = Arc::new(NoGoogle);
        let account_service = AccountService::new(context.clone(), identity);
        App { context, vault, events, clock, state, accounts, account_service, sync, info, revoker }
    }
    /// A session on `URL` written like the sign-in does (one `set` of the vault value).
    pub fn sign_in(&self, user: &str) {
        let session = ServerSession { url: URL.into(), user_id: user.into(), username: user.into(), device_id: "dev".into(), device_key: "spk_dev_secret-key".into() };
        self.vault.set(SERVER_SESSION, &serde_json::to_string(&session).unwrap()).unwrap();
    }
    pub fn consent(&self) {
        self.sync.record_consent(URL).unwrap();
    }
    pub fn now(&self) -> i64 {
        self.clock.0.load(Ordering::SeqCst)
    }
    pub fn advance(&self, secs: i64) {
        self.clock.0.fetch_add(secs, Ordering::SeqCst);
    }
    pub fn add_gmail(&self, email: &str, token: &str, client: &str) -> u32 {
        self.vault.set(&format!("gmail:{}", email.to_lowercase()), token).unwrap();
        self.context
            .update(|store| {
                let id = store.accounts.iter().map(|a| a.id).max().unwrap_or(0) + 1;
                store.accounts.push(Account {
                    id,
                    provider_id: "gmail".into(),
                    name: email.into(),
                    email: email.into(),
                    username: email.into(),
                    kind: AccountKind::Gmail,
                    oauth_client_id: client.into(),
                    color: [1, 2, 3],
                    imap_host: "imap.gmail.com".into(),
                    imap_port: 993,
                    smtp_host: "smtp.gmail.com".into(),
                    smtp_port: 465,
                    archive_folder: "[Gmail]/All Mail".into(),
                    trash_folder: "[Gmail]/Trash".into(),
                    ..Default::default()
                });
                Ok(id)
            })
            .unwrap()
    }
    /// A mailbox with an app password (`provider_id` other than Gmail).
    pub fn add_imap(&self, email: &str, password: &str) -> u32 {
        let id = self
            .context
            .update(|store| {
                let id = store.accounts.iter().map(|a| a.id).max().unwrap_or(0) + 1;
                store.accounts.push(Account {
                    id,
                    provider_id: "fastmail".into(),
                    name: "Ufficio".into(),
                    email: email.into(),
                    username: email.into(),
                    kind: AccountKind::Gmail,
                    color: [9, 9, 9],
                    imap_host: "imap.fastmail.com".into(),
                    imap_port: 993,
                    smtp_host: "smtp.fastmail.com".into(),
                    smtp_port: 465,
                    archive_folder: "Archive".into(),
                    trash_folder: "Trash".into(),
                    ..Default::default()
                });
                Ok(id)
            })
            .unwrap();
        self.vault.set(&format!("provider:fastmail:{id}"), password).unwrap();
        id
    }
    pub fn store(&self) -> Store {
        self.context.read().unwrap()
    }
    pub fn account(&self, email: &str) -> Option<Account> {
        self.store().accounts.into_iter().find(|a| a.email.eq_ignore_ascii_case(email))
    }
    pub fn token(&self, email: &str) -> String {
        self.vault.get(&format!("gmail:{}", email.to_lowercase())).unwrap()
    }
    pub fn id_of(provider: &str, email: &str) -> String {
        item_id(provider, email)
    }
}
struct NoGoogle;
impl qamille::application::ports::GoogleIdentity for NoGoogle {
    fn sign_in(&self, _: &str, _: &str, _: &str) -> AppResult<(String, String)> {
        Err(AppError::new("unavailable", "no Google here"))
    }
    fn cancel_sign_in(&self) {}
}
pub fn _unused(_: NullSink) {}
