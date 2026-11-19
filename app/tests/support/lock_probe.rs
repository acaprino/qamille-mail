//! A mail gateway that records, for every call that reaches a provider, whether
//! the mail lock of that account was held at that moment.
//!
//! ADR-0007 INV-001: no network call (IMAP, SMTP, HTTP) runs while a database,
//! `MailService` or cross-account lock is held. Wrap the fake gateway of a test
//! in a `ProbeMail`, build the `MailService` on top of it, `attach` the service,
//! then inspect `observed()` / `calls_under_lock()`.
use qamille::{
    application::{
        AppResult,
        mail::MailService,
        ports::*,
    },
    domain::outbox::{ProviderCapabilities, RemoteDraft},
    model::*,
};
use std::sync::{Arc, Mutex, OnceLock};

pub struct ProbeMail {
    inner: Arc<dyn MailGateway>,
    service: OnceLock<MailService>,
    observed: Mutex<Vec<(&'static str, bool)>>,
}

impl ProbeMail {
    pub fn new(inner: Arc<dyn MailGateway>) -> Arc<Self> {
        Arc::new(Self { inner, service: OnceLock::new(), observed: Mutex::new(Vec::new()) })
    }

    /// The service and the gateway reference each other: the gateway is built
    /// first (it goes into the `ServiceContext`), the service is attached after.
    pub fn attach(&self, service: MailService) {
        assert!(self.service.set(service).is_ok(), "the probe is attached to one service only");
    }

    /// Every provider call so far as `(call name, lock held at that moment)`.
    pub fn observed(&self) -> Vec<(&'static str, bool)> {
        self.observed.lock().unwrap().clone()
    }

    /// Names of the provider calls that were made while the account lock was held.
    pub fn calls_under_lock(&self) -> Vec<&'static str> {
        self.observed().into_iter().filter(|(_, held)| *held).map(|(call, _)| call).collect()
    }

    fn record(&self, call: &'static str, account_id: u32) {
        let service = self.service.get().expect("attach the MailService before using the gateway");
        let held = service.lock_held(account_id);
        self.observed.lock().unwrap().push((call, held));
    }
}

// Every overridable method is forwarded: a default method kept here would run
// the trait's default body against `self` instead of the wrapped gateway.
impl MailGateway for ProbeMail {
    fn send_remote_draft(&self, account: &Account, sender_name: &str, message: OutgoingMessage, id: &str, revision: &str) -> AppResult<()> {
        self.record("send_remote_draft", account.id);
        self.inner.send_remote_draft(account, sender_name, message, id, revision)
    }
    fn authorize_mcp_delivery(&self, profile: Option<&str>, account_id: u32) -> AppResult<()> {
        self.inner.authorize_mcp_delivery(profile, account_id)
    }
    fn list_labels(&self, account: &Account) -> AppResult<serde_json::Value> {
        self.record("list_labels", account.id);
        self.inner.list_labels(account)
    }
    fn manage_label(&self, account: &Account, action: &str, name: &str, new_name: Option<&str>) -> AppResult<serde_json::Value> {
        self.record("manage_label", account.id);
        self.inner.manage_label(account, action, name, new_name)
    }
    fn capabilities(&self, account: &Account) -> ProviderCapabilities {
        self.inner.capabilities(account)
    }
    fn list_remote_drafts(&self, account: &Account) -> AppResult<Vec<RemoteDraft>> {
        self.record("list_remote_drafts", account.id);
        self.inner.list_remote_drafts(account)
    }
    fn save_remote_draft(&self, account: &Account, draft: &RemoteDraft) -> AppResult<RemoteDraft> {
        self.record("save_remote_draft", account.id);
        self.inner.save_remote_draft(account, draft)
    }
    fn delete_remote_draft(&self, account: &Account, id: &str) -> AppResult<()> {
        self.record("delete_remote_draft", account.id);
        self.inner.delete_remote_draft(account, id)
    }
    fn sync_with_policy(&self, account: &Account, request: &SyncRequest<'_>) -> AppResult<PolicySyncBatch> {
        self.record("sync_with_policy", account.id);
        self.inner.sync_with_policy(account, request)
    }
    fn load_body(&self, account: &Account, email: &Email) -> AppResult<Email> {
        self.record("load_body", account.id);
        self.inner.load_body(account, email)
    }
    fn load_body_preview(&self, account: &Account, email: &Email) -> AppResult<Email> {
        self.record("load_body_preview", account.id);
        self.inner.load_body_preview(account, email)
    }
    fn authorize_read(&self, account: &Account) -> AppResult<()> {
        self.inner.authorize_read(account)
    }
    fn search(&self, account: &Account, request: &qamille::application::ports::RemoteSearchRequest, cancel: &std::sync::atomic::AtomicBool) -> AppResult<qamille::application::ports::RemoteSearchPage> {
        self.record("search", account.id);
        self.inner.search(account, request, cancel)
    }
    fn apply_message(&self, account: &Account, email: &Email, action: RemoteAction) -> AppResult<()> {
        self.record("apply_message", account.id);
        self.inner.apply_message(account, email, action)
    }
    fn resolve_identity(&self, account: &Account, email: &Email) -> AppResult<Email> {
        self.record("resolve_identity", account.id);
        self.inner.resolve_identity(account, email)
    }
    fn watch(&self, account: &Account) -> AppResult<Box<dyn MailWatch>> {
        self.record("watch", account.id);
        self.inner.watch(account)
    }
    fn attachment_bytes(&self, account: &Account, email: &Email, attachment_id: u64) -> AppResult<Vec<u8>> {
        self.record("attachment_bytes", account.id);
        self.inner.attachment_bytes(account, email, attachment_id)
    }
    fn originals(&self, account: &Account, emails: &[Email]) -> AppResult<Vec<Vec<u8>>> {
        self.record("originals", account.id);
        self.inner.originals(account, emails)
    }
    fn thread(&self, account: &Account, email: &Email) -> AppResult<Vec<Email>> {
        self.record("thread", account.id);
        self.inner.thread(account, email)
    }
    fn sync(&self, account: &Account) -> AppResult<SyncBatch> {
        self.record("sync", account.id);
        self.inner.sync(account)
    }
    fn apply(&self, account: &Account, uid: u32, action: RemoteAction) -> AppResult<()> {
        self.record("apply", account.id);
        self.inner.apply(account, uid, action)
    }
    fn send(&self, account: &Account, sender_name: &str, message: OutgoingMessage) -> AppResult<()> {
        self.record("send", account.id);
        self.inner.send(account, sender_name, message)
    }
    fn check_login(&self, account: &Account) -> AppResult<()> {
        self.record("check_login", account.id);
        self.inner.check_login(account)
    }
}
