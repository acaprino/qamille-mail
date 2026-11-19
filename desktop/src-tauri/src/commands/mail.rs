use crate::runtime::{DesktopState, blocking};
use qamille::{
    application::{
        AppResult,
        mail::{ConversationReply, ExportFormat, MailPage, MailQuery, RemoteSearchCursor, RemoteSearchReport, SyncReport},
    },
    model::Email,
};
use tauri::State;

#[tauri::command]
pub async fn list_mail(state: State<'_, DesktopState>, query: MailQuery) -> AppResult<MailPage> {
    let s = state.mail.clone();
    blocking(move || s.list(query)).await
}
#[tauri::command]
pub async fn search_suggestions(state: State<'_, DesktopState>, account_id: Option<u32>) -> AppResult<Vec<qamille::application::mail::SearchPerson>> {
    let s = state.mail.clone();
    blocking(move || s.search_suggestions(account_id)).await
}
#[tauri::command]
pub async fn search_remote_mail(state: State<'_, DesktopState>, query: String, account_id: Option<u32>, cursor: Option<RemoteSearchCursor>) -> AppResult<RemoteSearchReport> {
    let s = state.mail.clone();
    blocking(move || s.search_remote(&query, account_id, cursor)).await
}
#[tauri::command]
pub fn cancel_remote_search(state: State<'_, DesktopState>) { state.mail.cancel_remote_search(); }
#[tauri::command]
pub async fn read_mail(state: State<'_, DesktopState>, id: u64) -> AppResult<Email> {
    let s = state.mail.clone();
    blocking(move || s.read(id)).await
}
#[tauri::command]
pub fn prefetch_mail(state: State<'_, DesktopState>, ids: Vec<u64>) -> AppResult<()> {
    state.mail.prefetch(ids)
}
#[tauri::command]
pub async fn read_mail_thread(state: State<'_, DesktopState>, id: u64) -> AppResult<Vec<Email>> {
    let s = state.mail.clone();
    blocking(move || s.conversation(id)).await
}
/// The replies of a conversation row beyond the ones `list_mail` carried (ADR-0010). `folder` is
/// the view the row came from: the Trash and Spam views hold their own members.
#[tauri::command]
pub async fn read_replies(
    state: State<'_, DesktopState>,
    main_id: u64,
    folder: String,
    offset: usize,
    limit: usize,
) -> AppResult<Vec<ConversationReply>> {
    let s = state.mail.clone();
    blocking(move || s.replies(main_id, &folder, offset, limit)).await
}
#[tauri::command]
pub async fn prepare_print(
    state: State<'_, DesktopState>,
    ids: Vec<u64>,
    thread: bool,
) -> AppResult<Vec<Email>> {
    let s = state.mail.clone();
    blocking(move || {
        if thread {
            if ids.len() != 1 {
                return Err(qamille::application::AppError::validation(
                    "Select one conversation",
                ));
            }
            s.thread(ids[0])
        } else {
            s.selection(&ids)
        }
    })
    .await
}

#[derive(serde::Serialize)]
pub struct ExportSaved {
    path: String,
    count: usize,
}

#[tauri::command]
pub async fn export_mail(
    window: tauri::WebviewWindow,
    state: State<'_, DesktopState>,
    ids: Vec<u64>,
    format: ExportFormat,
    italian: Option<bool>,
) -> AppResult<Option<ExportSaved>> {
    let s = state.mail.clone();
    // Prepare everything before opening the dialog; an incomplete export is never saved.
    let export = blocking(move || s.export(&ids, format)).await?;
    let extension = match format {
        ExportFormat::Eml => "eml",
        ExportFormat::Mbox => "mbox",
    };
    // The dialog is owned by the calling window: modal to it, and never under it (tauri-native-desktop F1).
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(if italian.unwrap_or(false) { "Salva l'esportazione delle email" } else { "Save email export" })
        .add_filter(
            match (format, italian.unwrap_or(false)) {
                (ExportFormat::Eml, true) => "Messaggio email",
                (ExportFormat::Eml, false) => "Email message",
                (ExportFormat::Mbox, true) => "Archivio email",
                (ExportFormat::Mbox, false) => "Mailbox",
            },
            &[extension],
        )
        .set_file_name(&export.filename)
        .save_file()
        .await
    else {
        return Ok(None);
    };
    let path = file.path().to_path_buf();
    blocking(move || {
        std::fs::write(&path, export.bytes)?;
        Ok(Some(ExportSaved {
            path: path.to_string_lossy().into_owned(),
            count: export.count,
        }))
    })
    .await
}

#[tauri::command]
pub fn print_mail(window: tauri::WebviewWindow) -> AppResult<()> {
    // The Windows print dialog after the app's own preview, not Edge's preview on top (F8).
    crate::window_shell::print_with_system_dialog(&window)
        .map_err(|e| qamille::application::AppError::external(e.to_string()))
}
#[tauri::command]
pub async fn mail_action(
    state: State<'_, DesktopState>,
    id: u64,
    action: String,
    value: Option<String>,
) -> AppResult<()> {
    let s = state.mail.clone();
    blocking(move || s.act(id, &action, value.as_deref())).await
}
#[tauri::command]
pub fn mail_sync_status(state: State<'_, DesktopState>) -> qamille::application::mail_sync_status::MailSyncStatus {
    state.mail.sync_status()
}

#[tauri::command]
pub async fn sync_mail(state: State<'_, DesktopState>) -> AppResult<SyncReport> {
    let s = state.mail.clone();
    blocking(move || s.sync()).await
}
#[tauri::command]
pub async fn read_attachment(
    state: State<'_, DesktopState>,
    email_id: u64,
    attachment_id: u64,
    path: Option<Vec<u32>>,
) -> AppResult<String> {
    let s = state.mail.clone();
    let path = path.unwrap_or_default();
    blocking(move || s.attached_text(email_id, attachment_id, &path)).await
}
/// An attachment that is an email (`.eml`, `message/rfc822`), parsed for
/// reading; `path` reaches a message nested inside it.
#[tauri::command]
pub async fn read_attached_message(
    state: State<'_, DesktopState>,
    email_id: u64,
    attachment_id: u64,
    path: Option<Vec<u32>>,
) -> AppResult<qamille::domain::attached_message::AttachedMessage> {
    let s = state.mail.clone();
    let path = path.unwrap_or_default();
    blocking(move || s.attached_message(email_id, attachment_id, &path)).await
}

/// Queues one history batch for the account: it runs after the account's other work.
fn queue_history(state: &DesktopState, account_id: u32) {
    use qamille::application::account_worker::{Job, Priority};
    let _ = state.workers.submit(account_id, Priority::History, Job::Backfill);
}
/// Opening Spam or Trash of a Gmail account: its newest messages come with the next batch.
#[tauri::command]
pub async fn history_fetch_folder(state: State<'_, DesktopState>, account_id: u32, folder: String) -> AppResult<()> {
    let context = state.context();
    blocking(move || qamille::application::history::request_folder(&context, account_id, &folder)).await?;
    queue_history(&state, account_id);
    Ok(())
}
#[tauri::command]
pub async fn set_include_spam_trash(state: State<'_, DesktopState>, account_id: u32, enabled: bool) -> AppResult<()> {
    let context = state.context();
    blocking(move || qamille::application::history::set_include_spam_trash(&context, account_id, enabled)).await?;
    if enabled {
        queue_history(&state, account_id);
    }
    Ok(())
}
/// The choice of the unread notice: old unread Inbox messages marked read, here and on Gmail.
#[tauri::command]
pub async fn mark_old_unread_read(state: State<'_, DesktopState>, account_id: u32) -> AppResult<usize> {
    let context = state.context();
    let changed = blocking(move || qamille::application::history::mark_old_unread_read(&context, account_id, 30)).await?;
    queue_history(&state, account_id);
    Ok(changed)
}
