//! A vault that does not open on this computer (phase 3, Task 7: a data folder copied from
//! another Windows user or computer, no transfer passphrase) leaves mail and settings usable:
//! the workspace opens, the credentials count as absent, and reading one says why.
use qamille::{
    application::{AppError, AppResult, ServiceContext, ports::*, workspace::WorkspaceService},
    infrastructure::repository::MemoryRepository,
    model::*,
};
use std::sync::Arc;

const LOCKED: &str = "The credentials key belongs to another Windows user or another computer: sign in to the accounts again in Settings.";

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}
struct LockedSecrets;
impl SecretStore for LockedSecrets {
    fn get(&self, _: &str) -> AppResult<String> {
        Err(AppError::external(LOCKED))
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Err(AppError::external(LOCKED))
    }
}
struct NoAttachments;
impl AttachmentGateway for NoAttachments {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
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

fn context(store: Store) -> Arc<ServiceContext> {
    Arc::new(ServiceContext {
        compute: Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: Arc::new(NoMail),
        secrets: Arc::new(LockedSecrets),
        clock: Arc::new(FixedClock),
        attachments: Arc::new(NoAttachments),
    })
}

#[test]
fn vault_key_locked_here_still_opens_the_workspace() {
    let mut store = sample_store();
    let mut gmail = store.accounts[0].clone();
    gmail.id = 900;
    gmail.email = "moved@example.test".into();
    gmail.kind = AccountKind::Gmail;
    gmail.provider_id.clear();
    store.accounts.push(gmail);
    let accounts = store.accounts.len();
    let emails = store.emails.len();
    let context = context(store);
    let snapshot = WorkspaceService::new(context.clone()).snapshot().unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(json["has_ai_key"], false);
    assert_eq!(json["has_google_secret"], false);
    assert_eq!(json["accounts"].as_array().unwrap().len(), accounts);
    // The mail is untouched; a credential read says why it cannot be used.
    assert_eq!(context.read().unwrap().emails.len(), emails);
    let store = context.read().unwrap();
    assert_eq!(context.account(&store, 900).unwrap_err().message, LOCKED);
}
