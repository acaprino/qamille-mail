use qamille::{
    application::{
        self, AppError, AppResult, ServiceContext,
        account_worker::{CommitOutcome, WorkerRegistry},
        events::AppEvent,
        ports::{EventSink, StoreRepository},
    },
    infrastructure::{
        contact_repository::FileContactStore, gateways::*, google_people::GooglePeopleGateway,
        repository::JsonStoreRepository, sqlite_store::SqliteStoreRepository, store_migration,
    },
    model::Store,
};
use std::{
    ffi::OsStr,
    sync::{Arc, Condvar, Mutex},
};
use tauri::{Emitter, State};

pub fn demo_requested(value: Option<&OsStr>) -> bool {
    value == Some(OsStr::new("1"))
}

/// The desktop's single event outlet (ADR-0007 decision 5): every `AppEvent` a
/// writer returns from `update_with_events` becomes one `app-event` Tauri
/// emission, after the commit, serialized with its `kind` tag. Only this
/// process installs it — MCP, the native UI and the tests keep `NullSink`, so
/// no other process ever announces UI events. The taskbar sees every event
/// first; the three facts of a Windows notification (new mail, a failed
/// scheduled send, an expired sign-in) go to `notifications.rs` only, never to
/// the page (docs/aspetto-nativo.md, rules 28 and 30).
struct TauriEventSink(tauri::AppHandle);
impl EventSink for TauriEventSink {
    fn emit(&self, event: AppEvent) {
        crate::taskbar::observe(&self.0, &event);
        if crate::notifications::observe(&self.0, &event) {
            return;
        }
        // Only the main window: events carry mail subjects and notices, and the reminder chip
        // (which may listen) needs none of them (native audit T18).
        let _ = self.0.emit_to("main", "app-event", event);
    }
}

/// The rows an `email-changed` event named, read together (controller contract
/// for the merged UI of Task 2.3): `list_mail` summaries plus the three
/// membership flags — always present as keys, `null`/`false` when unset — in
/// the requested order, at most 100, with unknown or deleted ids omitted.
#[tauri::command]
pub async fn read_rows(
    state: State<'_, DesktopState>,
    ids: Vec<u64>,
) -> AppResult<Vec<application::mail::MailRow>> {
    let s = state.mail.clone();
    blocking(move || s.read_rows(ids)).await
}

#[derive(Clone)]
pub struct DesktopState {
    context: Arc<ServiceContext>,
    _workspace_lock: Arc<qamille::infrastructure::meeting_repository::WorkspaceUiLock>,
    /// What the startup migration found or did (Task 4.3): `main.rs` surfaces
    /// a damaged or freshly imported store once, as background notices.
    pub migration: store_migration::MigrationReport,
    /// Whether the shared store is the SQLite database with the change feed
    /// (Task 4.3). When true, the desktop bridges cross-process writes with
    /// the feed poller instead of the interim scheduler cap, and demo mode is
    /// off (demo always keeps its own JSON sample store).
    pub sqlite_engine: bool,
    /// Set once the deferred startup recovery has finished (success or failure).
    /// The queued-mail auto-send path waits on this so the historic
    /// "recovery before commit" ordering survives now that recovery runs off
    /// the main thread. A Condvar pair, not a polled flag: the gated thread
    /// sleeps until recovery notifies (review 3.4 minor 3, absorbed in 3.3).
    recovered: Arc<(Mutex<bool>, Condvar)>,
    pub meetings: application::meetings::MeetingService,
    pub reminders: application::meeting_reminders::MeetingReminderService,
    pub undo: application::undo::UndoService,
    /// Per-account job workers plus the dedicated drafts worker (ADR-0007
    /// decision 3): the undo deadline thread dispatches due confirmations here
    /// (Task 3.2, decision 4), and later tasks route syncs and reads through
    /// the same registry.
    pub workers: Arc<WorkerRegistry>,
    pub mail: application::mail::MailService,
    pub previews: application::document_preview::DocumentPreviewService,
    pub drafts: application::drafts::DraftService,
    pub workspace: application::workspace::WorkspaceService,
    pub assistant: application::assistant::AssistantService,
    pub accounts: application::accounts::AccountService,
    pub storage: application::storage::AccountStorageService,
    /// Storage maintenance job (ADR-0007 decision 2); Arc because the service
    /// owns its pass lock and is shared with the `qamille-maintenance` thread.
    pub maintenance: Arc<application::maintenance::MaintenanceService>,
    pub indexer: application::indexing::MailIndexService,
    pub scheduler: application::scheduler::SchedulerService,
    pub contacts: application::contacts::ContactService,
    /// The link to a Qamille server (access wave 2): session, consent, editions.
    pub server: Arc<application::server_account::ServerAccountService>,
    /// The mailboxes that follow the person between computers (wave 7).
    pub sync: Arc<application::sync::SyncService>,
    /// The notice about the old local compute mode (wave 2): stops what it left, tells once.
    pub legacy_local: Arc<application::legacy_local::LegacyLocalService>,
}
impl DesktopState {
    pub fn new(app: tauri::AppHandle) -> AppResult<Self> {
        let workspace_lock = Arc::new(qamille::infrastructure::meeting_repository::WorkspaceUiLock::acquire(&Store::data_dir())?);
        let data_dir = Store::data_dir();
        // Task 4.3: import the JSON store once (idempotent) and read the
        // engine marker the MCP process reads too, so the two processes can
        // never sit on two different stores.
        let migration = store_migration::migrate_if_needed(&data_dir)?;
        let demo = demo_requested(std::env::var_os("QAMILLE_DEMO").as_deref());
        let sqlite_engine = migration.activated && !demo;
        let repository: Arc<dyn StoreRepository> = if demo {
            Arc::new(JsonStoreRepository::demo(Store::path()))
        } else if sqlite_engine {
            Arc::new(SqliteStoreRepository::open(
                store_migration::sqlite_path(&data_dir),
            )?)
        } else {
            Arc::new(JsonStoreRepository::new(Store::path()))
        };
        // Fail before starting workers when data is unreadable.
        repository.read()?;
        // The compute factory chooses from the settings and the vault session at every step,
        // search and job (spec D3, A1).
        qamille::infrastructure::compute_factory::configure(repository.clone(), Arc::new(KeyringSecrets));
        let context = Arc::new(ServiceContext { compute: qamille::infrastructure::compute_factory::routed(),
            events: Arc::new(TauriEventSink(app.clone())),
            repository,
            mail: Arc::new(qamille::infrastructure::provider_registry::ProviderRegistry::initial(Store::data_dir().join("attachments"))),
            secrets: Arc::new(KeyringSecrets),
            clock: Arc::new(SystemClock),
            attachments: Arc::new(LocalAttachments),
        });
        let drafts = application::drafts::DraftService::new(context.clone());
        // One compute gateway for the process: the local server, found or started on first
        // use (spec D3, INV-9).
        // A compute server that cannot start at all is said once, as a notice of the window.
        qamille::infrastructure::compute_factory::set_notices(context.events.clone());
        let search: Arc<LocalSearch> = Arc::new(LocalSearch::new(context.compute.clone()));
        let mail = application::mail::MailService::new(context.clone())
            .with_search_index(search.clone());
        let undo = application::undo::UndoService::new(context.clone(), mail.clone(), drafts.clone());
        // The workers settle their undo confirmations against this same service
        // instance (`CommitOutcome`), and share the process-wide `MailService`
        // and `DraftService` so the per-account and draft operation locks stay
        // unique. Reconciled once against the loaded store so the workers exist
        // before the first deadline; the undo passes keep reconciling later.
        let workers = Arc::new(WorkerRegistry::new(
            mail.clone(),
            drafts.clone(),
            context.clone(),
            context.events.clone(),
            Arc::new(undo.clone()) as Arc<dyn CommitOutcome>,
        ));
        workers.reconcile(&context.read()?.accounts)?;
        let meetings = application::meetings::MeetingService::new(
            context.clone(),
            Arc::new(qamille::infrastructure::meeting_repository::FileMeetingRepository::new(Store::data_dir().join("meetings"))),
            Arc::new(qamille::infrastructure::meeting_audio::NativeAudioCapture::new()?),
            qamille::infrastructure::meeting_gateways::shared_processor(),
        );
        // Until `run_startup_recovery` has gone through the meetings, a window that asks about
        // one must not mark its unfinished jobs interrupted: recovery re-queues those it finds.
        meetings.set_recovering(true);
        let reminders = application::meeting_reminders::MeetingReminderService::new(
            context.clone(),
            Arc::new(qamille::mcp::calendar::ReminderCalendarReader::new(context.clone())),
            meetings.clone(),
        );
        let contacts = application::contacts::ContactService::new(
            context.clone(),
            Arc::new(FileContactStore::new(Store::data_dir())),
            Arc::new(GooglePeopleGateway::new(Arc::new(KeyringSecrets))),
        );
        let previews = application::document_preview::DocumentPreviewService::new(mail.clone());
        let server_account = Arc::new(application::server_account::ServerAccountService::new(
            context.clone(),
            Arc::new(qamille::infrastructure::server_account::HttpServerAccount),
            qamille::edition::edition(),
        ));
        // A credential that changes on this computer makes the cached access token useless.
        application::sync::set_forget_access(Box::new(|email| qamille::oauth::forget_access(email)));
        let sync = Arc::new(
            application::sync::SyncService::new(
                context.clone(),
                server_account.clone(),
                Arc::new(qamille::infrastructure::sync_http::HttpSyncServer),
                Arc::new(qamille::infrastructure::sync_state::FileSyncState::new(&data_dir)),
                Arc::new(qamille::infrastructure::sync_state::AppGoogleClients),
            )
            .with_revoker(Arc::new(qamille::infrastructure::sync_state::GoogleRevoker)),
        );
        let workspace = application::workspace::WorkspaceService::new(context.clone());
        // The threads that turn the sink's events into the taskbar's state and into Windows
        // notifications: running before any writer can announce something.
        crate::taskbar::start(&app, workspace.clone());
        crate::notifications::start(&app, context.clone());
        Ok(Self {
            context: context.clone(),
            _workspace_lock: workspace_lock,
            migration,
            sqlite_engine,
            // Startup recovery (undo + meetings) is deferred to the
            // `qamille-startup` thread so the window is not blocked; see
            // `run_startup_recovery`.
            recovered: Arc::new((Mutex::new(false), Condvar::new())),
            meetings: meetings.clone(),
            reminders,
            undo,
            workers,
            mail,
            previews,
            workspace,
            assistant: application::assistant::AssistantService::new(
                context.clone(),
                Arc::new(ConfiguredAssistant),
                search.clone(),
            ).with_meetings(meetings),
            accounts: application::accounts::AccountService::new(
                context.clone(),
                Arc::new(GoogleOAuth),
            ).with_grant_cleanup(qamille::infrastructure::mailbox_access::revoke),
            storage: application::storage::AccountStorageService::new(context.clone()),
            maintenance: Arc::new(application::maintenance::MaintenanceService::new(
                context.clone(),
            )),
            indexer: application::indexing::MailIndexService::new(
                context.clone(),
                Arc::new(qamille::infrastructure::mail_index_source::GmailIndexSource),
                search.clone(),
            ),
            scheduler: application::scheduler::SchedulerService::new(
                context.clone(),
                drafts.clone(),
            ),
            drafts,
            contacts,
            server: server_account,
            sync,
            legacy_local: Arc::new(application::legacy_local::LegacyLocalService::new(
                context.clone(),
                Arc::new(qamille::infrastructure::legacy_local::FileSystemLegacyLocal::new(data_dir.clone())),
            )),
        })
    }
    pub fn live_sync(&self) -> application::mail_sync::MailSyncService {
        // Task 3.3: the coordinator dispatches each download to the account's
        // registry worker instead of syncing inline.
        application::mail_sync::MailSyncService::new(self.context.clone(), self.workers.clone())
    }
    /// The shared store, for the change-feed poller of Task 4.3: a JSON store
    /// answers with an empty feed, the SQLite store with the rows other
    /// processes committed.
    pub fn repository(&self) -> Arc<dyn StoreRepository> {
        self.context.repository.clone()
    }
    /// The shared service context, for the commands that call an application
    /// function over it (the activity log).
    pub fn context(&self) -> Arc<ServiceContext> {
        self.context.clone()
    }
    /// Runs the interrupted-state recovery off the main thread (Task 3.4).
    /// Semantics are identical to the previous synchronous startup path:
    /// interrupted `Committing` undo actions become `Failed` with their message
    /// (INV-008: a send or move whose server result was lost during shutdown is
    /// never retried), then interrupted meeting capture/jobs are recovered. The
    /// order (undo, then meetings) is preserved. The ready flag is set on
    /// completion whether or not recovery succeeded, so gated startup paths are
    /// never stuck; the caller surfaces any error as a background notice.
    pub fn run_startup_recovery(&self) -> AppResult<()> {
        // The ready flag must flip even if a recovery step unexpectedly unwinds,
        // otherwise the gated auto-send path would wait forever.
        let _ready_on_drop = ReadyOnDrop(&self.recovered);
        // Dropped first: when the gated paths wake up, the meetings are no longer recovering.
        let _recovering_off = RecoveringOff(&self.meetings);
        if let Err(error) = self.accounts.retry_mailbox_cleanup() {
            eprintln!("Could not read pending mailbox cleanup: {error}");
        }
        self.undo
            .recover_interrupted()
            .and_then(|()| self.meetings.recover_interrupted())
    }
    /// True once `run_startup_recovery` has finished (success or failure).
    /// Non-blocking peek for commands that refuse instead of waiting.
    pub fn is_recovered(&self) -> bool {
        *self
            .recovered
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    /// Blocks until `run_startup_recovery` has finished (success or failure),
    /// without polling: recovery flips the flag and notifies, even when it
    /// unwinds (`ReadyOnDrop`).
    pub fn wait_for_recovery(&self) {
        let (flag, condvar) = &*self.recovered;
        let mut ready = flag
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while !*ready {
            ready = condvar
                .wait(ready)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}
/// Ends the meetings' recovery phase when dropped, also when recovery fails or unwinds.
struct RecoveringOff<'a>(&'a application::meetings::MeetingService);
impl Drop for RecoveringOff<'_> {
    fn drop(&mut self) {
        self.0.set_recovering(false);
    }
}
/// Flips the startup ready flag and wakes the gated thread when dropped, so an
/// unwinding recovery step cannot leave it waiting forever.
struct ReadyOnDrop<'a>(&'a (Mutex<bool>, Condvar));
impl Drop for ReadyOnDrop<'_> {
    fn drop(&mut self) {
        let (flag, condvar) = self.0;
        *flag.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = true;
        condvar.notify_all();
    }
}
pub async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> AppResult<T> + Send + 'static,
) -> AppResult<T> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|_| {
            AppError::new(
                "worker",
                "The operation stopped unexpectedly. Retry the operation.",
            )
        })?
}

#[cfg(test)]
mod tests {
    use super::demo_requested;
    use std::ffi::OsStr;

    #[test]
    fn demo_requires_an_explicit_one() {
        assert!(demo_requested(Some(OsStr::new("1"))));
        assert!(!demo_requested(None));
        for value in ["", "0", "true", "yes", "1 ", " 1"] {
            assert!(!demo_requested(Some(OsStr::new(value))), "{value:?}");
        }
    }
}
