//! Shared doubles and builders for the contacts integration tests.
#![allow(dead_code)]
use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        contact_ports::{ContactGateway, ContactStore},
        contacts::*,
        ports::*,
    },
    domain::contacts::*,
    infrastructure::{contact_repository::MemoryContactStore, repository::MemoryRepository},
    model::*,
};
use std::{collections::HashMap, sync::{Arc, Mutex}};

pub struct TestClock;
impl Clock for TestClock {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}
#[derive(Default)]
pub struct TestSecrets(Mutex<HashMap<String, String>>);
impl SecretStore for TestSecrets {
    fn get(&self, key: &str) -> AppResult<String> {
        Ok(self.0.lock().unwrap().get(key).cloned().unwrap_or_default())
    }
    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
}
pub struct TestAttachments;
impl AttachmentGateway for TestAttachments {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, a: &Attachment) -> AppResult<String> {
        Ok(a.inline_text.clone().unwrap_or_default())
    }
}
pub struct NoMail;
impl MailGateway for NoMail {
    fn sync(&self, _: &Account) -> AppResult<SyncBatch> {
        Err(AppError::external("offline"))
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

pub fn account(id: u32, email: &str, kind: AccountKind) -> Account {
    Account { id, name: format!("Account {id}"), email: email.into(), kind, oauth_client_id: "client".into(), ..Default::default() }
}
pub fn mail_from(account_id: u32, name: &str, address: &str) -> Email {
    Email { id: account_id as u64 * 1000 + address.len() as u64, account_id, from_name: name.into(), from_addr: address.into(), ..Default::default() }
}
pub fn base_store() -> Store {
    let mut store = Store::default();
    store.accounts.push(account(1, "alex@work.test", AccountKind::Gmail));
    store.accounts.push(account(2, "alex@home.test", AccountKind::Gmail));
    store.accounts.push(account(3, "demo@sample.test", AccountKind::Sample));
    store
}

pub struct Fixture {
    pub service: ContactService,
    pub contacts: Arc<MemoryContactStore>,
    pub context: Arc<ServiceContext>,
}
pub fn fixture(store: Store) -> Fixture {
    fixture_with(store, Arc::new(qamille::application::contact_ports::NullContactGateway))
}
pub fn fixture_with(store: Store, gateway: Arc<dyn ContactGateway>) -> Fixture {
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: Arc::new(NoMail),
        secrets: Arc::new(TestSecrets::default()),
        clock: Arc::new(TestClock),
        attachments: Arc::new(TestAttachments),
    });
    let contacts = Arc::new(MemoryContactStore::default());
    let service = ContactService::new(context.clone(), contacts.clone(), gateway);
    Fixture { service, contacts, context }
}
pub fn fields(name: &str, emails: &[&str]) -> ContactFields {
    ContactFields {
        name: name.into(),
        emails: emails.iter().map(|e| Labeled { label: String::new(), value: e.to_string() }).collect(),
        ..Default::default()
    }
}
pub fn source(key: &str, book: BookId, remote: Option<&str>, f: ContactFields, modified: i64) -> SourceContact {
    SourceContact { key: key.into(), book, remote_id: remote.map(String::from), etag: remote.map(|_| "e1".into()), fields: f, modified, ..Default::default() }
}
pub fn seed(fx: &Fixture, sources: Vec<SourceContact>) {
    fx.contacts
        .transaction(&mut |s| {
            s.sources.extend(sources.clone());
            Ok(())
        })
        .unwrap();
}
pub fn all(fx: &Fixture) -> Vec<ContactView> {
    fx.service.list(ContactQuery { limit: 500, ..Default::default() }).unwrap().items
}
/// Gives write permission to the named Google books, as a finished consent flow would.
pub fn grant(fx: &Fixture, books: &[&str]) {
    fx.contacts
        .transaction(&mut |s| {
            s.settings.write_granted = books.iter().map(|b| b.to_string()).collect();
            Ok(())
        })
        .unwrap();
}

/// A scripted Google People server: pages, tokens, etags, failures and concurrency.
#[derive(Default)]
pub struct FakeContactGateway {
    pub server: Mutex<std::collections::BTreeMap<u32, Vec<RemoteContact>>>,
    pub other: Mutex<std::collections::BTreeMap<u32, Vec<RemoteContact>>>,
    pub deltas: Mutex<std::collections::BTreeMap<u32, Vec<RemoteContact>>>,
    pub log: Mutex<Vec<String>>,
    pub fail_writes: Mutex<std::collections::BTreeMap<u32, String>>,
    pub fail_reads: Mutex<std::collections::BTreeMap<u32, String>>,
    /// Accounts whose read consent was never given: listing answers `consent_required`.
    pub no_read_consent: Mutex<std::collections::BTreeSet<u32>>,
    /// Accounts whose write permission was revoked on Google's side: every write answers `consent_required`.
    pub write_without_consent: Mutex<std::collections::BTreeSet<u32>>,
    pub page_size: std::sync::atomic::AtomicUsize,
    pub expire: std::sync::atomic::AtomicBool,
    pub refuse_consent: Mutex<Option<String>>,
    pub active: std::sync::atomic::AtomicUsize,
    pub max_active: std::sync::atomic::AtomicUsize,
    pub last_update_unmanaged: Mutex<Option<serde_json::Value>>,
    /// Runs once, inside the next write, as if the user kept editing while the request was in flight.
    pub on_write: Mutex<Option<Box<dyn Fn() + Send + Sync>>>,
    counter: std::sync::atomic::AtomicU64,
}
use qamille::application::contact_ports::{RemoteContact, RemotePage};
use std::sync::atomic::Ordering;

impl FakeContactGateway {
    pub fn remote(id: &str, etag: &str, f: ContactFields) -> RemoteContact {
        RemoteContact { remote_id: id.into(), etag: etag.into(), fields: f, unmanaged: serde_json::Value::Null, deleted: false }
    }
    pub fn put(&self, account: u32, contacts: Vec<RemoteContact>) {
        self.server.lock().unwrap().insert(account, contacts);
    }
    pub fn calls(&self, prefix: &str) -> usize {
        self.log.lock().unwrap().iter().filter(|l| l.starts_with(prefix)).count()
    }
    fn next(&self) -> u64 {
        self.counter.fetch_add(1, Ordering::SeqCst) + 100
    }
    fn write<T>(&self, account: &Account, work: impl FnOnce() -> AppResult<T>) -> AppResult<T> {
        let now = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active.fetch_max(now, Ordering::SeqCst);
        if let Some(hook) = self.on_write.lock().unwrap().take() {
            hook();
        }
        std::thread::sleep(std::time::Duration::from_millis(3));
        let failure = self.fail_writes.lock().unwrap().get(&account.id).cloned();
        let revoked = self.write_without_consent.lock().unwrap().contains(&account.id);
        let result = match failure {
            _ if revoked => Err(AppError::new("consent_required", "Allow writing to this address book first")),
            Some(message) => Err(AppError::external(message)),
            None => work(),
        };
        self.active.fetch_sub(1, Ordering::SeqCst);
        result
    }
}
impl ContactGateway for FakeContactGateway {
    fn list(&self, account: &Account, other: bool, page: Option<&str>, sync_token: Option<&str>) -> AppResult<RemotePage> {
        self.log.lock().unwrap().push(format!("list {} other={other} token={sync_token:?} page={page:?}", account.id));
        if self.no_read_consent.lock().unwrap().contains(&account.id) {
            return Err(AppError::new("consent_required", "Allow access to Google Contacts for this account first"));
        }
        if let Some(message) = self.fail_reads.lock().unwrap().get(&account.id).cloned() {
            return Err(AppError::external(message));
        }
        if sync_token.is_some() && self.expire.load(Ordering::SeqCst) {
            return Err(AppError::new("sync_token_expired", "expired"));
        }
        if sync_token.is_some() {
            let items = self.deltas.lock().unwrap().remove(&account.id).unwrap_or_default();
            return Ok(RemotePage { items, next_page: None, next_sync_token: Some(format!("T{}", self.next())) });
        }
        let table = if other { &self.other } else { &self.server };
        let items = table.lock().unwrap().get(&account.id).cloned().unwrap_or_default();
        let size = match self.page_size.load(Ordering::SeqCst) {
            0 => items.len().max(1),
            n => n,
        };
        let start: usize = page.and_then(|p| p.parse().ok()).unwrap_or(0);
        let end = (start + size).min(items.len());
        Ok(RemotePage {
            items: items[start..end].to_vec(),
            next_page: (end < items.len()).then(|| end.to_string()),
            next_sync_token: (end >= items.len()).then(|| "T1".to_string()),
        })
    }
    fn create(&self, account: &Account, fields: &ContactFields, request_id: &str) -> AppResult<RemoteContact> {
        self.write(account, || {
            let n = self.next();
            let mut created = Self::remote(&format!("people/{n}"), &format!("e{n}"), fields.clone());
            created.unmanaged = serde_json::json!({"clientData": [{"key": qamille::application::contact_ports::CREATE_REQUEST_KEY, "value": request_id}]});
            self.log.lock().unwrap().push(format!("create {}", account.id));
            self.server.lock().unwrap().entry(account.id).or_default().push(created.clone());
            Ok(created)
        })
    }
    fn update(&self, account: &Account, remote_id: &str, etag: &str, fields: &ContactFields, unmanaged: &serde_json::Value) -> AppResult<RemoteContact> {
        self.write(account, || {
            self.log.lock().unwrap().push(format!("update {} {remote_id} etag={etag}", account.id));
            *self.last_update_unmanaged.lock().unwrap() = Some(unmanaged.clone());
            let mut server = self.server.lock().unwrap();
            let card = server.entry(account.id).or_default().iter_mut().find(|c| c.remote_id == remote_id).ok_or_else(|| AppError::new("not_found", "gone"))?;
            if card.etag != etag {
                return Err(AppError::new("etag_conflict", "etag differs"));
            }
            card.fields = fields.clone();
            card.etag = format!("e{}", self.next());
            Ok(card.clone())
        })
    }
    fn delete(&self, account: &Account, remote_id: &str) -> AppResult<()> {
        self.write(account, || {
            self.log.lock().unwrap().push(format!("delete {} {remote_id}", account.id));
            let mut server = self.server.lock().unwrap();
            let list = server.entry(account.id).or_default();
            let before = list.len();
            list.retain(|c| c.remote_id != remote_id);
            if list.len() == before { Err(AppError::new("not_found", "gone")) } else { Ok(()) }
        })
    }
    fn get(&self, account: &Account, remote_id: &str) -> AppResult<RemoteContact> {
        self.log.lock().unwrap().push(format!("get {} {remote_id}", account.id));
        self.server.lock().unwrap().get(&account.id).and_then(|l| l.iter().find(|c| c.remote_id == remote_id).cloned()).ok_or_else(|| AppError::new("not_found", "gone"))
    }
    fn promote_other(&self, account: &Account, remote_id: &str) -> AppResult<RemoteContact> {
        self.write(account, || {
            let mut other = self.other.lock().unwrap();
            let list = other.entry(account.id).or_default();
            let position = list.iter().position(|c| c.remote_id == remote_id).ok_or_else(|| AppError::new("not_found", "gone"))?;
            let n = self.next();
            let mut promoted = list.remove(position);
            promoted.remote_id = format!("people/{n}");
            promoted.etag = format!("e{n}");
            self.server.lock().unwrap().entry(account.id).or_default().push(promoted.clone());
            self.log.lock().unwrap().push(format!("promote {}", account.id));
            Ok(promoted)
        })
    }
    fn authorize(&self, account: &Account, write: bool) -> AppResult<()> {
        self.log.lock().unwrap().push(format!("authorize {} write={write}", account.id));
        match self.refuse_consent.lock().unwrap().clone() {
            Some(message) => Err(AppError::new("consent_required", message)),
            None => Ok(()),
        }
    }
}
