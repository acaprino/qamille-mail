//! A vault that does not open on this computer (phase 3, Task 7: a data folder copied from
//! another Windows user or computer) leaves mail and settings usable: the workspace opens, the
//! credentials count as absent, and reading one says why. A sign-in the person starts goes on
//! when no passphrase can open the old credentials (its save sets them aside), and starts nothing
//! (no browser, no login, no device) when a transfer passphrase must be entered first.
use qamille::{
    application::{
        AppError, AppResult, ServiceContext,
        accounts::AccountService,
        error::vault_codes,
        ports::*,
        server_account::{Credentials, LoginGrant, LoginOutcome, RemoteEndpoint, ServerAccount, ServerAccountService, ServerAddress},
        workspace::WorkspaceService,
    },
    domain::server_account::{DeviceInfo, ServerInfo},
    edition::Edition,
    infrastructure::repository::MemoryRepository,
    model::*,
};
use std::sync::{Arc, Mutex};

const FOREIGN: &str = "The credentials key belongs to another Windows user or another computer: sign in to the accounts again in Settings.";
const LOCKED: &str = "The credentials key belongs to another Windows user or another computer: enter the transfer passphrase in Settings, or sign in to the accounts again.";
const SERVER: &str = "https://qamille.example";

struct FixedClock;
impl Clock for FixedClock {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}

/// The vault as `KeyringSecrets` reports it: every read refused with its code; a save goes through
/// only when no passphrase can open them, and then (the real vault set the old credentials aside)
/// the new credentials open.
struct LockedVault {
    transfer: bool,
    saved: Mutex<Vec<String>>,
    values: Mutex<std::collections::HashMap<String, String>>,
}
impl LockedVault {
    fn new(transfer: bool) -> Arc<Self> {
        Arc::new(Self { transfer, saved: Mutex::new(Vec::new()), values: Mutex::new(Default::default()) })
    }
    fn refusal(&self) -> AppError {
        if self.transfer {
            AppError::new(vault_codes::LOCKED, LOCKED)
        } else {
            AppError::new(vault_codes::FOREIGN, FOREIGN)
        }
    }
    fn saved(&self) -> Vec<String> {
        self.saved.lock().unwrap().clone()
    }
}
impl SecretStore for LockedVault {
    fn get(&self, key: &str) -> AppResult<String> {
        if self.saved.lock().unwrap().is_empty() {
            return Err(self.refusal());
        }
        Ok(self.values.lock().unwrap().get(key).cloned().unwrap_or_default())
    }
    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        if self.transfer {
            return Err(self.refusal());
        }
        self.saved.lock().unwrap().push(key.into());
        self.values.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn check_writable(&self) -> AppResult<()> {
        if self.transfer { Err(self.refusal()) } else { Ok(()) }
    }
    fn set_aside_notice(&self) -> Option<String> {
        (!self.saved.lock().unwrap().is_empty()).then(|| "credentials-set-aside-20261121-101500".into())
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

/// Google in the browser: what it was asked with.
#[derive(Default)]
struct Google {
    secrets_seen: Mutex<Vec<String>>,
}
impl GoogleIdentity for Google {
    fn client_credentials(&self, _: &str, client_secret: &str) -> AppResult<(String, String)> {
        self.secrets_seen.lock().unwrap().push(client_secret.into());
        Ok(("qamille-app-client".into(), "qamille-app-secret".into()))
    }
    fn sign_in(&self, _: &str, _: &str, _: &str) -> AppResult<(String, String)> {
        Ok(("moved@example.test".into(), "fresh-refresh-token".into()))
    }
    fn cancel_sign_in(&self) {}
}

/// A Qamille server that counts the logins, the only call that registers a device.
#[derive(Default)]
struct Server {
    logins: Mutex<u32>,
}
impl ServerAccount for Server {
    fn info(&self, _: &ServerAddress) -> AppResult<ServerInfo> {
        unreachable!()
    }
    fn login(&self, _: &ServerAddress, _: &Credentials<'_>) -> AppResult<LoginOutcome> {
        *self.logins.lock().unwrap() += 1;
        Ok(LoginOutcome::Signed(LoginGrant {
            user_id: "u1".into(),
            username: "mario".into(),
            device_id: "d1".into(),
            device_key: "device-key".into(),
        }))
    }
    fn change_temporary_password(&self, _: &ServerAddress, _: &Credentials<'_>, _: &str) -> AppResult<LoginOutcome> {
        unreachable!()
    }
    fn disable_device_with_password(&self, _: &ServerAddress, _: &str, _: &str, _: &str) -> AppResult<()> {
        unreachable!()
    }
    fn devices(&self, _: &RemoteEndpoint) -> AppResult<Vec<DeviceInfo>> {
        unreachable!()
    }
    fn revoke_device(&self, _: &RemoteEndpoint, _: &str) -> AppResult<()> {
        unreachable!()
    }
    fn change_password(&self, _: &RemoteEndpoint, _: &str, _: &str, _: bool) -> AppResult<()> {
        unreachable!()
    }
    fn sign_out(&self, _: &RemoteEndpoint) -> AppResult<()> {
        unreachable!()
    }
    fn delete_account(&self, _: &RemoteEndpoint, _: &str) -> AppResult<()> {
        unreachable!()
    }
}

fn context(store: Store, vault: Arc<LockedVault>) -> Arc<ServiceContext> {
    Arc::new(ServiceContext {
        compute: Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: Arc::new(NoMail),
        secrets: vault,
        clock: Arc::new(FixedClock),
        attachments: Arc::new(NoAttachments),
    })
}

fn moved_store() -> Store {
    let mut store = sample_store();
    let mut gmail = store.accounts[0].clone();
    gmail.id = 900;
    gmail.email = "moved@example.test".into();
    gmail.kind = AccountKind::Gmail;
    gmail.provider_id.clear();
    store.accounts.push(gmail);
    store.settings.compute_consented_urls = vec![SERVER.into()];
    store
}

#[test]
fn removing_a_mailbox_from_a_locked_vault_preserves_the_account_and_skips_post_commit_cleanup() {
    let context = context(moved_store(), LockedVault::new(true));
    let service = AccountService::new(context.clone(), Arc::new(Google::default()))
        .with_grant_cleanup(|id, detach| {
            assert_eq!(id, 900);
            detach()?;
            panic!("contact writes must survive a failed removal")
        });
    let error = service.remove_account(900).unwrap_err();
    assert_eq!(error.code, vault_codes::LOCKED);
    assert!(context.read().unwrap().account(900).is_some());
}

#[test]
fn failed_mail_commit_leaves_all_three_credentials_for_retry() {
    struct RejectFirstCommit {
        inner: Arc<dyn StoreRepository>,
        reject: std::sync::atomic::AtomicBool,
    }
    impl StoreRepository for RejectFirstCommit {
        fn read(&self) -> AppResult<Store> { self.inner.read() }
        fn transaction(&self, operation: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
            self.inner.transaction(&mut |store| {
                operation(store)?;
                if self.reject.swap(false, std::sync::atomic::Ordering::SeqCst) {
                    return Err(AppError::storage("injected mail commit failure"));
                }
                Ok(())
            })
        }
    }
    let vault = LockedVault::new(false);
    let keys = ["gmail:moved@example.test", "contacts:900", "calendar:900"];
    for key in keys { vault.set(key, "value-before-detach").unwrap(); }
    let mut context = context(moved_store(), vault.clone());
    let writable = Arc::get_mut(&mut context).unwrap();
    writable.repository = Arc::new(RejectFirstCommit {
        inner: writable.repository.clone(), reject: std::sync::atomic::AtomicBool::new(true),
    });
    let service = AccountService::new(context.clone(), Arc::new(Google::default()));
    assert!(service.remove_account(900).is_err());
    assert!(context.read().unwrap().account(900).is_some());
    assert!(context.read().unwrap().settings.pending_mailbox_cleanup.is_empty());
    for key in keys { assert_eq!(vault.get(key).unwrap(), "value-before-detach"); }
    service.remove_account(900).unwrap();
    for key in keys { assert!(vault.get(key).unwrap().is_empty()); }
    assert!(context.read().unwrap().settings.pending_mailbox_cleanup.is_empty());
}

#[test]
fn vault_key_locked_here_still_opens_the_workspace() {
    let store = moved_store();
    let accounts = store.accounts.len();
    let emails = store.emails.len();
    let context = context(store, LockedVault::new(false));
    let snapshot = WorkspaceService::new(context.clone()).snapshot().unwrap();
    let json = serde_json::to_value(&snapshot).unwrap();
    assert_eq!(json["has_ai_key"], false);
    assert_eq!(json["has_google_secret"], false);
    assert_eq!(json["credentials_set_aside"], serde_json::Value::Null);
    assert_eq!(json["accounts"].as_array().unwrap().len(), accounts);
    // The mail is untouched; a credential read says why it cannot be used.
    assert_eq!(context.read().unwrap().emails.len(), emails);
    let store = context.read().unwrap();
    let error = context.account(&store, 900).unwrap_err();
    assert_eq!((error.code, error.message.as_str()), (vault_codes::FOREIGN, FOREIGN));
}

#[test]
fn vault_key_of_another_user_gives_way_to_a_google_sign_in() {
    let vault = LockedVault::new(false);
    let context = context(moved_store(), vault.clone());
    let google = Arc::new(Google::default());
    AccountService::new(context.clone(), google.clone()).connect_google("").unwrap();
    // The saved client secret counted as none, and the new token was saved.
    assert_eq!(*google.secrets_seen.lock().unwrap(), [String::new()]);
    assert_eq!(vault.saved(), ["gmail:moved@example.test"]);
    let snapshot = serde_json::to_value(WorkspaceService::new(context).snapshot().unwrap()).unwrap();
    assert_eq!(snapshot["credentials_set_aside"], "credentials-set-aside-20261121-101500");
}

#[test]
fn vault_key_with_a_transfer_passphrase_starts_no_sign_in() {
    let vault = LockedVault::new(true);
    let context = context(moved_store(), vault.clone());
    let google = Arc::new(Google::default());
    let error = AccountService::new(context.clone(), google.clone()).connect_google("").unwrap_err();
    assert_eq!(error.code, vault_codes::LOCKED);
    assert!(google.secrets_seen.lock().unwrap().is_empty(), "the browser never opened");
    let error = AccountService::new(context.clone(), google).connect_password("Moved", "imap@example.test", "app-password").unwrap_err();
    assert_eq!(error.code, vault_codes::LOCKED);
    assert!(context.read().unwrap().accounts.iter().all(|a| a.email != "imap@example.test"));
    assert!(vault.saved().is_empty());
}

#[test]
fn vault_key_with_a_transfer_passphrase_registers_no_server_device() {
    let vault = LockedVault::new(true);
    let server = Arc::new(Server::default());
    let service = ServerAccountService::new(context(moved_store(), vault), server.clone(), Edition::Professional { server: SERVER.into() });
    let credentials = Credentials { identifier: "mario", password: "una password lunga", device_name: "PC" };
    assert_eq!(service.connect(SERVER, &credentials).unwrap_err().code, vault_codes::LOCKED);
    assert_eq!(*server.logins.lock().unwrap(), 0, "no login, so no orphan device");
    // Without a passphrase the login goes on and its session is saved over the old credentials.
    let vault = LockedVault::new(false);
    let service = ServerAccountService::new(context(moved_store(), vault.clone()), server.clone(), Edition::Professional { server: SERVER.into() });
    let connected = service.connect(SERVER, &credentials);
    assert!(connected.is_ok(), "{:?}", connected.err());
    assert_eq!(*server.logins.lock().unwrap(), 1);
    assert_eq!(vault.saved(), ["server-session"]);
}
