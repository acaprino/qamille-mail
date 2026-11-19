use crate::runtime::{DesktopState, blocking};
use qamille::{
    application::{
        AppResult,
        contact_merge::{DuplicateView, MergeOutcome, MergePreview},
        contact_sync::{BookSync, Keep, SyncReport},
        contacts::{BookView, ContactPage, ContactQuery, ContactView, ImportReport, Suggestion},
    },
    domain::contacts::{BookOutcome, ContactFields, ContactSettings},
};
use tauri::{AppHandle, Emitter, State};

/// Sends waiting Google writes in the background, then tells the interface to reload.
fn flush_in_background(state: &DesktopState, app: &AppHandle) {
    let (service, app) = (state.contacts.clone(), app.clone());
    std::thread::spawn(move || {
        let _ = service.flush_queue();
        let _ = app.emit("contacts-changed", ());
    });
}

#[tauri::command]
pub async fn contacts_list(state: State<'_, DesktopState>, query: ContactQuery) -> AppResult<ContactPage> {
    let s = state.contacts.clone();
    blocking(move || s.list(query)).await
}
#[tauri::command]
pub async fn contacts_get(state: State<'_, DesktopState>, id: String) -> AppResult<ContactView> {
    let s = state.contacts.clone();
    blocking(move || s.get(&id)).await
}
#[tauri::command]
pub async fn contacts_books(state: State<'_, DesktopState>) -> AppResult<Vec<BookView>> {
    let s = state.contacts.clone();
    blocking(move || s.books()).await
}
#[tauri::command]
pub async fn contacts_similar(state: State<'_, DesktopState>, fields: ContactFields) -> AppResult<Vec<ContactView>> {
    let s = state.contacts.clone();
    blocking(move || s.similar_to(&fields)).await
}
#[tauri::command]
pub async fn contacts_create(state: State<'_, DesktopState>, app: AppHandle, book: String, fields: ContactFields) -> AppResult<ContactView> {
    let s = state.contacts.clone();
    let created = blocking(move || s.create(&book, fields)).await?;
    flush_in_background(&state, &app);
    Ok(created)
}
#[tauri::command]
pub async fn contacts_prefill(state: State<'_, DesktopState>, id: String) -> AppResult<ContactFields> {
    let s = state.contacts.clone();
    blocking(move || s.prefill(&id)).await
}
#[tauri::command]
pub async fn contacts_update_targets(state: State<'_, DesktopState>, id: String) -> AppResult<Vec<BookView>> {
    let s = state.contacts.clone();
    blocking(move || s.update_targets(&id)).await
}
#[tauri::command]
pub async fn contacts_update(state: State<'_, DesktopState>, app: AppHandle, id: String, fields: ContactFields) -> AppResult<ContactView> {
    let s = state.contacts.clone();
    let updated = blocking(move || s.update(&id, fields)).await?;
    flush_in_background(&state, &app);
    Ok(updated)
}
#[tauri::command]
pub async fn contacts_delete(state: State<'_, DesktopState>, app: AppHandle, id: String, books: Vec<String>) -> AppResult<()> {
    let s = state.contacts.clone();
    blocking(move || s.delete(&id, books)).await?;
    flush_in_background(&state, &app);
    Ok(())
}
#[tauri::command]
pub async fn contacts_hide(state: State<'_, DesktopState>, id: String, hidden: bool) -> AppResult<()> {
    let s = state.contacts.clone();
    blocking(move || s.hide(&id, hidden)).await
}
#[tauri::command]
pub async fn contacts_suggest(state: State<'_, DesktopState>, text: String, limit: usize) -> AppResult<Vec<Suggestion>> {
    let s = state.contacts.clone();
    blocking(move || s.suggest(&text, limit)).await
}
#[tauri::command]
pub async fn contacts_duplicates(state: State<'_, DesktopState>, include_possible: bool) -> AppResult<Vec<DuplicateView>> {
    let s = state.contacts.clone();
    blocking(move || s.duplicates(include_possible)).await
}
#[tauri::command]
pub async fn contacts_same_person(state: State<'_, DesktopState>, keys: Vec<String>) -> AppResult<()> {
    let s = state.contacts.clone();
    blocking(move || s.same_person(keys)).await
}
#[tauri::command]
pub async fn contacts_unlink(state: State<'_, DesktopState>, keys: Vec<String>) -> AppResult<()> {
    let s = state.contacts.clone();
    blocking(move || s.unlink(keys)).await
}
#[tauri::command]
pub async fn contacts_different_people(state: State<'_, DesktopState>, keys: Vec<String>) -> AppResult<()> {
    let s = state.contacts.clone();
    blocking(move || s.different_people(keys)).await
}
#[tauri::command]
pub async fn contacts_merge_preview(state: State<'_, DesktopState>, keys: Vec<String>, keep_books: Vec<String>) -> AppResult<MergePreview> {
    let s = state.contacts.clone();
    blocking(move || s.merge_preview(keys, keep_books)).await
}
#[tauri::command]
pub async fn contacts_merge_apply(state: State<'_, DesktopState>, app: AppHandle, keys: Vec<String>, keep_books: Vec<String>, fields: ContactFields) -> AppResult<MergeOutcome> {
    let s = state.contacts.clone();
    let outcome = blocking(move || s.merge_apply(keys, keep_books, fields)).await?;
    let _ = app.emit("contacts-changed", ());
    Ok(outcome)
}
#[tauri::command]
pub async fn contacts_merge_undo(state: State<'_, DesktopState>, app: AppHandle, journal_id: u64) -> AppResult<MergeOutcome> {
    let s = state.contacts.clone();
    let outcome = blocking(move || s.merge_undo(journal_id)).await?;
    flush_in_background(&state, &app);
    Ok(outcome)
}
#[tauri::command]
pub async fn contacts_import_vcf(state: State<'_, DesktopState>, text: String) -> AppResult<ImportReport> {
    let s = state.contacts.clone();
    blocking(move || s.import_vcf(&text)).await
}
#[tauri::command]
pub async fn contacts_export_vcf(state: State<'_, DesktopState>, book: String) -> AppResult<String> {
    let s = state.contacts.clone();
    blocking(move || s.export_vcf(&book)).await
}
/// The result of "Esporta .vcf" on the desktop.
#[derive(serde::Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum VcfExport {
    Empty,
    Cancelled,
    Saved { path: String, count: usize },
}

/// Saves the book as a .vcf where the user picks in the Windows "Save as"
/// dialog, owned by the calling window, instead of a WebView2 download (F3).
#[tauri::command]
pub async fn contacts_save_vcf(
    window: tauri::WebviewWindow,
    state: State<'_, DesktopState>,
    book: String,
    italian: Option<bool>,
) -> AppResult<VcfExport> {
    let s = state.contacts.clone();
    let vcf = blocking(move || s.export_vcf(&book)).await?;
    if vcf.trim().is_empty() {
        return Ok(VcfExport::Empty);
    }
    let italian = italian.unwrap_or(false);
    let Some(file) = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(if italian { "Esporta i contatti" } else { "Export contacts" })
        .add_filter(if italian { "Biglietto da visita (vCard)" } else { "Contact card (vCard)" }, &["vcf"])
        .set_file_name("qamille-contacts.vcf")
        .save_file()
        .await
    else {
        return Ok(VcfExport::Cancelled);
    };
    let path = file.path().to_path_buf();
    let count = vcf.matches("BEGIN:VCARD").count();
    blocking(move || {
        std::fs::write(&path, vcf)?;
        Ok(VcfExport::Saved { path: path.to_string_lossy().into_owned(), count })
    })
    .await
}
#[tauri::command]
pub async fn contacts_sync(state: State<'_, DesktopState>, app: AppHandle) -> AppResult<SyncReport> {
    let s = state.contacts.clone();
    let report = blocking(move || s.sync_all()).await?;
    let _ = app.emit("contacts-changed", ());
    Ok(report)
}
#[tauri::command]
pub async fn contacts_sync_book(state: State<'_, DesktopState>, app: AppHandle, book: String) -> AppResult<BookSync> {
    let s = state.contacts.clone();
    let report = blocking(move || s.sync_book(&book)).await?;
    let _ = app.emit("contacts-changed", ());
    Ok(report)
}
#[tauri::command]
pub async fn contacts_set_sync(state: State<'_, DesktopState>, book: String, enabled: bool) -> AppResult<()> {
    let s = state.contacts.clone();
    blocking(move || s.set_sync(&book, enabled)).await
}
#[tauri::command]
pub async fn contacts_authorize(state: State<'_, DesktopState>, book: String, write: bool) -> AppResult<()> {
    let s = state.contacts.clone();
    blocking(move || s.authorize(&book, write)).await
}
#[tauri::command]
pub async fn contacts_resolve_conflict(state: State<'_, DesktopState>, app: AppHandle, key: String, keep: Keep) -> AppResult<()> {
    let s = state.contacts.clone();
    blocking(move || s.resolve_conflict(&key, keep)).await?;
    flush_in_background(&state, &app);
    Ok(())
}
#[tauri::command]
pub async fn contacts_promote_other(state: State<'_, DesktopState>, id: String) -> AppResult<ContactView> {
    let s = state.contacts.clone();
    blocking(move || s.promote_other(&id)).await
}
#[tauri::command]
pub async fn contacts_flush(state: State<'_, DesktopState>) -> AppResult<Vec<BookOutcome>> {
    let s = state.contacts.clone();
    blocking(move || s.flush_queue()).await
}
#[tauri::command]
pub async fn contacts_settings(state: State<'_, DesktopState>) -> AppResult<ContactSettings> {
    let s = state.contacts.clone();
    blocking(move || s.settings()).await
}
#[tauri::command]
pub async fn contacts_save_settings(state: State<'_, DesktopState>, priority: Vec<String>, roles: Vec<String>) -> AppResult<()> {
    let s = state.contacts.clone();
    blocking(move || s.save_settings(priority, roles)).await
}
