#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod commands;
mod native_feel;
mod recovery;
mod reminders;
mod runtime;
mod shell_rules;
mod window_shell;
mod zone;

// phase 3, Task 1: modules

// phase 3, Task 2: modules

// phase 3, Task 3: modules

// phase 3, Task 4: modules

// phase 3, Task 5: modules
mod saved_files;

// phase 3, Task 6: modules

// phase 3, Task 8: modules

use commands::{accounts::*, activity::*, assistant::*, document_preview::*, drafts::*, mail::*, undo::*, workspace::*};
use tauri::{Emitter, Manager};
use commands::meetings::*;
use commands::calendar_mcp::calendar_mcp;
use commands::assistant_installation::*;
use commands::contacts::*;
use commands::server_account::*;
use commands::sync::*;
use commands::legacy_local::*;

fn main() {
    if matches!(std::env::args().nth(1).as_deref(), Some("--skill" | "skill")) {
        print!("{}", qamille::infrastructure::assistant_packages::skill_text());
        return;
    }
    if std::env::args().any(|a| a == "--mcp") {
        qamille::mcp::serve();
        return;
    }
    if std::env::args().any(|a| a == "--worker") {
        qamille::mcp::server::worker();
        return;
    }
    // Started by a restart after WebView2 failed (recovery.rs): the old process
    // must be gone before this one takes the workspace lock.
    recovery::wait_for_previous_instance();
    let demo = runtime::demo_requested(std::env::var_os("QAMILLE_DEMO").as_deref());
    // Before the first read of the data folder: an update that renamed the app's folder
    // from Qemille or Sparklone brings its workspace along.
    if let Some(adopted) = qamille::model::Store::adopt_previous_install() {
        eprintln!("Qamille: workspace from the previous folder moved to {}", adopted.display());
    }
    let data_dir = qamille::model::Store::data_dir();
    let webview_dir = data_dir.join("webview");
    if let Err(error) = std::fs::create_dir_all(&webview_dir) {
        rfd::MessageDialog::new()
            .set_title("Qamille")
            .set_description(format!("Qamille cannot write its portable data folder:\n{}\n\n{error}", data_dir.display()))
            .set_level(rfd::MessageLevel::Error)
            .show();
        return;
    }
    // Before any app threads start: WebView2 must use the same portable profile
    // even when the parent process has inherited a system-wide override.
    unsafe { std::env::set_var("WEBVIEW2_USER_DATA_FOLDER", &webview_dir) };
    let mut context = tauri::generate_context!();
    context.config_mut().app.app_directories_override = Some(
        tauri::utils::config::AppDirectoriesOverride::Root(data_dir.clone()),
    );
    // Each portable copy owns its own window and workspace lock.
    use std::hash::{Hash, Hasher};
    let mut identity = std::collections::hash_map::DefaultHasher::new();
    data_dir.canonicalize().unwrap_or_else(|_| data_dir.clone()).hash(&mut identity);
    context.config_mut().identifier.push_str(&format!(".workspace{:016x}", identity.finish()));
    if demo {
        context.config_mut().identifier.push_str(".preview");
    }
    let mut window_config = context.config().app.windows[0].clone();
    window_config.create = false;
    // Shown by `native_feel::show_main_window` once painted in the system theme.
    window_config.visible = false;
    // The webview is created with the first frame's colour (WebView2 takes it
    // at creation, before any page): the dark --ground when Qamille starts dark.
    if window_shell::starts_dark() {
        window_config.background_color = Some(native_feel::DARK_GROUND);
    }
    context.config_mut().app.windows[0].create = false;
    let started = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        // Tauri 2.12 turns an `Err` returned from `.setup` into a panic, and a panic there ends the
        // process (exit code 101): a release build (no console) would end without a word. Both are
        // said here instead (audit Q2).
        .setup(move |app| {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                setup_app(app, demo, window_config, webview_dir)
            }));
            let error: Box<dyn std::error::Error> = match outcome {
                Ok(Ok(())) => return Ok(()),
                Ok(Err(error)) => error,
                Err(panic) => format!("unexpected error: {}", shell_rules::panic_message(&*panic)).into(),
            };
            startup_failed(Some(app.handle()), &*error)
        })
        .on_window_event(|window, event| {
            if window.label() != "main" {
                return;
            }
            match event {
                // The chip closes with the main window, so the app can exit and reopen.
                tauri::WindowEvent::Destroyed => reminders::close_with_main(window.app_handle()),
                tauri::WindowEvent::CloseRequested { .. } => recovery::close_requested(window.app_handle()),
                _ => {}
            }
        })
        .invoke_handler(tauri::generate_handler![
            reminders::event_join_record, reminders::reminder_view, reminders::reminder_join, reminders::reminder_dismiss,
            recovery::reload_ready, recovery::close_seen, recovery::close_settled, recovery::frontend_ready, recovery::frontend_failed,
            window_shell::session_end_ready, window_shell::show_system_menu, window_shell::window_snap_button,
            window_shell::window_appearance, window_shell::window_zoom,
            meeting_devices, meeting_list, meeting_list_page, meeting_read, meeting_start, meeting_capture_status,
            meeting_pause, meeting_resume, meeting_stop, meeting_recover, meeting_save_notes,
            meeting_add_note, meeting_update, meeting_delete, meeting_import, meeting_transcribe,
            meeting_summarize, meeting_cancel_job, meeting_retry_job, meeting_correct_segment,
            meeting_toggle_action, meeting_search, meeting_audio_range, meeting_export,
            meeting_models,
            meeting_confirm_close, meeting_cancel_all_jobs,
            meeting_readiness, meeting_transcribe_default, meeting_summarize_default, meeting_discard_audio,
            meeting_audio_cleanup_preview, meeting_audio_cleanup,
            workspace_snapshot,
            list_mail,
            runtime::read_rows,
            search_suggestions,
            search_remote_mail,
            history_fetch_folder,
            set_include_spam_trash,
            mark_old_unread_read,
            app_edition,
            server_status,
            legacy_local_status,
            legacy_local_free_space,
            legacy_local_dismiss,
            server_probe,
            server_record_consent,
            server_record_llm_consent,
            server_llm_status,
            server_connect,
            server_change_temporary_password,
            server_disable_device_with_password,
            server_devices,
            server_revoke_device,
            server_change_password,
            server_change_server,
            server_sign_out,
            server_delete_account,
            server_startup_check,
            server_finish_onboarding,
            sync_status,
            sync_set_enabled,
            sync_set_mailbox,
            sync_now,
            sync_resolve_removal,
            sync_revoke_mailbox,
            sync_revoke_all,
            account_remove,
            compute_set_mode,
            compute_status,
            search_reindex,
            compute_set_ca_pem,
            compute_forget_pending_ca,
            server_signup_start,
            server_signup_confirm,
            server_reset_start,
            server_reset_confirm,
            server_connect_browser,
            server_cancel_browser_sign_in,
            server_browser_disable_device,
            server_account_info,
            server_start_checkout,
            server_checkout_status,
            server_open_billing_portal,
            server_delete_account_browser,
            cancel_remote_search,
            read_mail,
            prefetch_mail,
            read_mail_thread,
            read_replies,
            prepare_print,
            export_mail,
            print_mail,
            mail_action,
            queue_mail_action,
            queue_draft_action,
            undo_action,
            dismiss_action,
            activity_available,
            activity_recent,
            activity_page,
            activity_unseen_errors,
            activity_mark_seen,
            sync_mail,
            mail_sync_status,
            read_attachment,
            read_attached_message,
            attachment_preview_source,
            attachment_save,
            saved_files::saved_file_open, saved_files::saved_file_reveal,
            list_drafts,
            create_draft,
            save_draft,
            discard_draft,
            send_draft,
            pick_attachments,
            attach_mail_to_draft,
            release_attachment_imports,
            render_body,
            save_settings,
            list_events,
            save_event,
            delete_event,
            list_meetings,
            save_meeting_notes,
            save_signature,
            delete_signature,
            open_link,
            summarize_mail,
            suggest_reply,
            ask_assistant,
            semantic_search,
            index_mail,
            indexing_status,
            configure_indexing,
            search_model_installed,
            search_model_status,
            connect_google,
            cancel_google_sign_in,
            connect_password,
            calendar_mcp,
            mcp_configuration,
            save_mcp_configuration,
            start_mcp_worker,
            assistant_installation_status,
            assistant_installation_preview,
            assistant_installation_apply,
            assistant_installation_open,
            get_account_storage,
            update_account_storage,
            contacts_list, contacts_get, contacts_books, contacts_similar, contacts_create,
            contacts_prefill, contacts_update_targets, contacts_update, contacts_delete,
            contacts_hide, contacts_suggest, contacts_duplicates, contacts_same_person,
            contacts_unlink, contacts_different_people, contacts_merge_preview,
            contacts_merge_apply, contacts_merge_undo, contacts_import_vcf,
            contacts_export_vcf, contacts_save_vcf, contacts_sync, contacts_sync_book, contacts_set_sync,
            contacts_authorize, contacts_resolve_conflict, contacts_promote_other,
            contacts_flush, contacts_settings, contacts_save_settings
        ])
        .run(context);
    // Only `Builder::build` fails here (a plugin or the runtime could not start). A failure inside
    // `.setup` (the workspace already open in another Qamille, also after a restart whose old
    // process did not exit in time; a damaged store; no WebView2 Runtime) never returns here:
    // `startup_failed` has already said it and exited.
    if let Err(error) = started {
        startup_failed(None, &error);
    }
}

/// A start that failed: the reason in a Windows message and in `native.log`, then exit code 1,
/// never the silent end of a process without a window (audit Q2, T21). Whatever failed, a
/// missing WebView2 Runtime is said as such: no window can open without it. `app`: when the
/// failure came from `setup_app`, which may already have built and shown the main window.
fn startup_failed(app: Option<&tauri::AppHandle>, error: &dyn std::error::Error) -> ! {
    native_feel::report(&format!("could not start: {error}"));
    // A failure after `build()`: the window is on screen, and its page would stay usable
    // behind a message that has no owner, and could still open a recovery dialog over it.
    recovery::stop_asking();
    if let Some(window) = app.and_then(|app| app.get_webview_window("main")) {
        let _ = window.hide();
    }
    let webview_missing = tauri::webview_version().is_err();
    rfd::MessageDialog::new()
        .set_title("Qamille")
        .set_description(shell_rules::startup_failure_text(&error.to_string(), window_shell::italian(), webview_missing))
        .set_level(rfd::MessageLevel::Error)
        .show();
    // As every exit without the page: what the state may have started (a meeting job) is
    // stopped first. Without the state this does nothing.
    if let Some(app) = app {
        recovery::finish_meetings_before_exit(app);
    }
    std::process::exit(1)
}

/// Everything Qamille starts before its first frame: the state (it takes the workspace lock and
/// opens the store), the main window and the background threads. An `Err` is a start that
/// failed, which the `.setup` closure hands to `startup_failed`.
fn setup_app(
    app: &mut tauri::App,
    demo: bool,
    window_config: tauri::utils::config::WindowConfig,
    webview_dir: std::path::PathBuf,
) -> Result<(), Box<dyn std::error::Error>> {
    // First, before the store is opened and long before `build()`: without the WebView2 Runtime no
    // window can open, and in a release build `build()` would show Tauri's own English message.
    if let Err(error) = tauri::webview_version() {
        return Err(format!("WebView2 Runtime not found: {error}").into());
    }
    // The state owns the real event sink: every writer announces its
    // change as one typed `app-event` after the commit (ADR-0007
    // decision 5). No thread below re-announces what a writer announced.
    let state = runtime::DesktopState::new(app.handle().clone())?;
    app.manage(state.clone());
    // The old local compute mode: stop a server left running, clean up, work out the notice.
    // After `compute_factory::configure` (in `DesktopState::new`); never in the demo.
    if !demo {
        let _ = state.legacy_local.startup();
        // The uninstaller an upgrade runs first removes the worker's startup file, and a moved app
        // folder leaves it naming the old one: with automatic startup on, write it again. Off the
        // main thread (it waits for a file lock) and never a reason not to start.
        let _ = std::thread::Builder::new()
            .name("qamille-autostart".into())
            .spawn(|| {
                if let Err(error) = qamille::infrastructure::mcp_config::refresh_autostart() {
                    // Into native.log too: a release build has no console for stderr.
                    native_feel::report(&format!("the worker's startup file was not refreshed: {}", error.message));
                }
            });
    }
    let main_window = tauri::WebviewWindowBuilder::from_config(app, &window_config)?
        .data_directory(webview_dir)
        // The app suggests recipients itself; browser "Suggestions" would cover them.
        .general_autofill_enabled(false)
        .on_navigation(native_feel::navigation_guard(app.handle(), true))
        // A link that asks for a window of its own opens in the system browser; the app never gets one.
        .on_new_window(native_feel::new_window_guard(app.handle()))
        // Every page that starts loading must draw within 15 s, a reload the page starts too.
        .on_page_load(recovery::page_load)
        .build()?;
    native_feel::harden_webview(&main_window, "index.html");
    window_shell::attach_main(&main_window);
    native_feel::show_main_window(&main_window);
    recovery::watch_startup(&main_window);
    reminders::spawn(app.handle().clone(), state.clone())?;
    if demo
        && let Some(window) = app.get_webview_window("main")
    {
        window.set_title("Qamille — Sample workspace")?;
    }
    let handle = app.handle().clone();
    // Move the heavy interrupted-state recovery off the main thread so the
    // window appears immediately regardless of store size or meeting count.
    // What recovery changed announces itself: `recover_interrupted` emits
    // the typed events for the actions it marks, and the UI reloads once
    // on `startup-recovery-done`. An error stays a background notice, so
    // the toast appears without the removed file watcher.
    let startup_state = state.clone();
    let startup_handle = handle.clone();
    std::thread::Builder::new()
        .name("qamille-startup".into())
        .spawn(move || {
            match startup_state.run_startup_recovery() {
                Ok(()) => {
                    let _ = startup_handle.emit("startup-recovery-done", ());
                }
                Err(error) => { let _ = startup_handle.emit_to("main", "background-notice", error.message); }
            }
            // The one-time migration report (Task 4.3) is announced
            // after recovery, when the window's listeners are up: a
            // damaged store must never pass silently, and a fresh
            // import says that the JSON original was preserved.
            if startup_state.migration.damaged {
                let _ = startup_handle.emit_to(
                    "main", "background-notice",
                    "The mail data file store.json was unreadable; it was preserved as store.damaged.json and the workspace started empty.",
                );
            } else if startup_state.migration.migrated {
                let _ = startup_handle.emit_to(
                    "main", "background-notice",
                    format!(
                        "Mail data imported into the SQLite store ({} emails, {} accounts); store.json is preserved as a backup.",
                        startup_state.migration.emails, startup_state.migration.accounts
                    ),
                );
            }
        })?;
    // Storage maintenance: deadline sleep + one pass, never inside a
    // user write (ADR-0007 decision 2). Errors surface like every other
    // background thread's, via `background-notice`.
    let maintenance = state.maintenance.clone();
    let maintenance_handle = handle.clone();
    std::thread::Builder::new()
        .name("qamille-maintenance".into())
        .spawn(move || loop {
            maintenance.wait_until_due();
            if let Err(error) = maintenance.run_once() {
                let _ = maintenance_handle.emit_to("main", "background-notice", error.message);
            }
        })?;
    let indexer = state.indexer.clone();
    let index_handle = handle.clone();
    std::thread::Builder::new().name("qamille-mail-index".into()).spawn(move || {
        // Once at startup; afterwards the maintenance thread keeps storage reconciled.
        if let Err(error) = indexer.reconcile_storage() {
            let _ = index_handle.emit_to("main", "background-notice", error.message);
        }
        // `indexing-changed` at most once a second while batches follow each other.
        let mut announced: Option<std::time::Instant> = None;
        let mut unannounced = false;
        loop {
            // Last resort: whatever happens inside one pass, the thread survives.
            let worked = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| indexer.step())) {
                Ok(Ok(true)) => true,
                Ok(Ok(false)) => false,
                Ok(Err(error)) => { let _ = index_handle.emit_to("main", "background-notice", error.message); false }
                Err(_) => { let _ = index_handle.emit_to("main", "background-notice", "Indexing hit an unexpected error and will continue."); false }
            };
            unannounced |= worked;
            if unannounced && (!worked || announced.is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1))) {
                let _ = index_handle.emit("indexing-changed", ());
                announced = Some(std::time::Instant::now());
                unannounced = false;
            }
            // After work, only a short pause so user writes go first (INV-5);
            // with nothing to do, the old two seconds.
            std::thread::sleep(if worked { std::time::Duration::from_millis(50) } else { std::time::Duration::from_secs(2) });
        }
    })?;
    if !demo {
        // Recordings stopped with no server wait flagged (`transcription_waiting`): once a
        // server is connected they are transcribed. Only flagged ones, never the old ones.
        let waiting = state.meetings.clone();
        std::thread::Builder::new().name("qamille-waiting-work".into()).spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_secs(30));
                if matches!(qamille::infrastructure::compute_factory::global().route(), qamille::infrastructure::compute_factory::Route::Server(_)) {
                    let _ = waiting.resume_waiting();
                }
            }
        })?;
    }
    let meetings = state.meetings.clone();
    let meeting_handle = handle.clone();
    std::thread::Builder::new().name("qamille-meeting-state".into()).spawn(move || {
        let mut last_error = String::new();
        loop {
            std::thread::sleep(std::time::Duration::from_millis(500));
            match meetings.capture_status() {
                Ok(_) => last_error.clear(),
                Err(error) if error.message != last_error => {
                    last_error = error.message.clone();
                    let _ = meeting_handle.emit_to("main", "background-notice", error.message);
                }
                Err(_) => {}
            }
        }
    })?;
    let undo = state.undo.clone();
    let workers = state.workers.clone();
    let undo_gate = state.clone();
    let undo_handle = handle.clone();
    std::thread::Builder::new().name("qamille-undo".into()).spawn(move || {
        // Preserve the startup ordering now that recovery is deferred: never
        // auto-commit a queued send until interrupted recovery has marked any
        // surviving `Committing` action `Failed` (INV-008: a send whose server
        // result was lost during shutdown is never retried).
        // The gate sleeps on recovery's Condvar instead of polling the
        // flag every 50 ms (review 3.4 minor 3, absorbed here).
        undo_gate.wait_for_recovery();
        // Deadline-driven, not polling (ADR-0007 decision 4, INV-005): the
        // thread sleeps until the next confirmation's deadline, a queue
        // writer's signal, or a connectivity wake. Due confirmations are
        // dispatched to the per-account workers and settled there, so a
        // slow commit never delays another account's. The queue writers
        // announce every transition themselves (`undo-changed`,
        // `undo-removed`, and the mail or draft events of the confirmed
        // action): the thread only reports errors.
        loop {
            let deadline = match undo.next_deadline() {
                Ok(deadline) => deadline,
                Err(error) => {
                    let _ = undo_handle.emit_to("main", "background-notice", error.message);
                    std::thread::sleep(std::time::Duration::from_secs(5));
                    continue;
                }
            };
            undo.wait_for_work(deadline);
            if let Err(error) = undo.commit_due(&workers) {
                let _ = undo_handle.emit_to("main", "background-notice", error.message);
                std::thread::sleep(std::time::Duration::from_secs(5));
            }
        }
    })?;
    // The mailboxes that follow the person: a pass at startup, every five minutes and two
    // seconds after a local change. A failure reaches the main window once, not every
    // five minutes (the person already knows).
    {
        let sync = state.sync.clone();
        let sync_handle = handle.clone();
        std::thread::Builder::new().name("qamille-mailbox-sync".into()).spawn(move || {
            // Let the main window attach its listeners, so the first notice is not lost.
            std::thread::sleep(std::time::Duration::from_secs(8));
            let mut last = String::new();
            loop {
                match sync.tick(std::time::Instant::now()) {
                    // Only a pass that ran says the failure is over: `Ok(None)` (nothing was
                    // due) must not make the same error come back every five minutes.
                    Ok(Some(_)) => last.clear(),
                    Ok(None) => {}
                    Err(error) => {
                        if error.message != last {
                            let _ = sync_handle.emit_to("main", "background-notice", error.message.clone());
                            last = error.message;
                        }
                    }
                }
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        })?;
    }
    let contacts = state.contacts.clone();
    let contacts_handle = handle.clone();
    std::thread::Builder::new().name("qamille-contacts-sync".into()).spawn(move || {
        // Every enabled Google book is read at startup and then every 10 minutes;
        // each distinct failure reaches the main window once.
        let mut last = String::new();
        // Let the main window attach its listeners, so the first notice is not lost.
        std::thread::sleep(std::time::Duration::from_secs(5));
        loop {
            let message = match contacts.sync_all() {
                Ok(report) => report
                    .books
                    .iter()
                    .filter(|b| !b.consent_required)
                    .find_map(|b| b.error.clone())
                    .unwrap_or_default(),
                Err(error) => error.message,
            };
            if message != last {
                if !message.is_empty() {
                    let _ = contacts_handle.emit_to("main", "background-notice", message.clone());
                }
                last = message;
            }
            let _ = contacts_handle.emit("contacts-changed", ());
            std::thread::sleep(std::time::Duration::from_secs(600));
        }
    })?;
    let sync_state = state.clone();
    let sync_handle = handle.clone();
    std::thread::Builder::new().name("qamille-gmail-sync".into()).spawn(move || {
        let mut sync = sync_state.live_sync();
        loop {
            // A merge announces itself from the writer (`sync-progress`,
            // one `email-changed` per account, `counts-changed`): the
            // thread only surfaces the errors as notices.
            match sync.step(std::time::Duration::from_secs(1)) {
                Ok(report) => {
                    for error in report.errors { let _ = sync_handle.emit_to("main", "background-notice", error); }
                }
                Err(error) => {
                    let _ = sync_handle.emit_to("main", "background-notice", error.message);
                    std::thread::sleep(std::time::Duration::from_secs(5));
                }
            }
        }
    })?;
    // The activity history (centro attività) lives in its own file whatever engine the mail
    // store runs on, and the MCP process writes there too: this thread announces the rows
    // the OTHER process wrote or changed (`activity-changed`). Rows of this process are
    // skipped at the source, its services announced them themselves. A demo has no history.
    {
        let repository = state.repository();
        if repository.activity_supported() {
            let activity_handle = handle.clone();
            std::thread::Builder::new()
                .name("qamille-activity-feed".into())
                .spawn(move || {
                    let mut last = repository.activity_feed_head().unwrap_or(0);
                    loop {
                        std::thread::sleep(std::time::Duration::from_secs(2));
                        if let Ok((next, ids)) = repository.poll_activity_feed(last) {
                            last = next;
                            if !ids.is_empty() {
                                let _ = activity_handle.emit_to(
                                    "main",
                                    "app-event",
                                    qamille::application::events::AppEvent::ActivityChanged { ids },
                                );
                            }
                        }
                    }
                })?;
        }
    }
    if state.sqlite_engine {
        // Cross-process reactivity (Task 4.3, EX-005/D-crossproc): the
        // MCP process commits into the shared SQLite store and appends
        // to its change feed; this thread reads the new rows every 2 s
        // (a cheap WAL read, not an mtime watcher), synthesizes the
        // local events and wakes the scheduler for new deadlines.
        // Rows of this process are skipped at the source (pid): its
        // writers already announced them.
        let feed_state = state.clone();
        let feed_handle = handle.clone();
        std::thread::Builder::new()
            .name("qamille-change-feed".into())
            .spawn(move || {
                let repository = feed_state.repository();
                let mut last = repository.change_feed_head().unwrap_or(0);
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(2));
                    let rows = match repository.poll_change_feed(last) {
                        Ok(rows) => rows,
                        Err(_) => continue,
                    };
                    if rows.is_empty() {
                        continue;
                    }
                    last = rows.last().map_or(last, |row| row.seq);
                    qamille::application::scheduler::note_schedule_change();
                    for row in rows {
                        for event in row.events() {
                            let _ = feed_handle.emit_to("main", "app-event", event);
                        }
                    }
                }
            })?;
    }
    std::thread::Builder::new()
        .name("qamille-scheduler".into())
        .spawn(move || {
            // Deadlines, not polling (INV-005): the thread sleeps until
            // the earliest snooze/remind/send/outbox deadline or until
            // a writer notes a schedule change. Wakes announce their
            // messages from the writer, so nothing is re-emitted here
            // and no file date is watched (INV-004).
            // Cross-process bridge (EX-005): on the JSON store the MCP
            // process writes deadlines into the shared file without
            // noting this process's waker, so the wait is capped to
            // keep its queued sends as timely as the old 1 s tick. On
            // the SQLite engine the change-feed poller notes the waker
            // within its own 2 s pass and the deadlines stay exact.
            // In-process writers always note the waker directly.
            const CROSS_PROCESS_CAP_MS: i64 = 2_000;
            loop {
                let deadline = match state.scheduler.next_deadline() {
                    Ok(deadline) => deadline,
                    Err(e) => {
                        let _ = handle.emit_to("main", "background-notice", e.message);
                        std::thread::sleep(std::time::Duration::from_secs(5));
                        continue;
                    }
                };
                let wait = if state.sqlite_engine {
                    deadline
                } else {
                    let cap = state.scheduler.now_ms() + CROSS_PROCESS_CAP_MS;
                    Some(deadline.map_or(cap, |due| due.min(cap)))
                };
                state.scheduler.wait_for_deadline(wait);
                match state.scheduler.tick() {
                    Ok(notices) => {
                        for note in notices {
                            let _ = handle.emit_to("main", "background-notice", note);
                        }
                    }
                    Err(e) => {
                        let _ = handle.emit_to("main", "background-notice", e.message);
                    }
                }
            }
        })?;
    Ok(())
}
