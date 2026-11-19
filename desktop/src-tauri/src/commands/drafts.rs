use crate::runtime::{DesktopState, blocking};
use qamille::{
    application::{AppResult, ports::StoreRepository},
    infrastructure::portable_files,
    model::{Draft, Store},
};
use tauri::State;

#[tauri::command]
pub fn render_body(body: String, format: qamille::model::BodyFormat) -> String {
    qamille::richtext::convert_body(&body, format, qamille::model::BodyFormat::Html)
}

#[tauri::command]
pub async fn list_drafts(state: State<'_, DesktopState>) -> AppResult<Vec<Draft>> {
    let s = state.drafts.clone();
    blocking(move || s.list()).await
}
#[tauri::command]
pub async fn create_draft(
    state: State<'_, DesktopState>,
    account_id: Option<u32>,
    reply_to: Option<u64>,
    forward: bool,
) -> AppResult<Draft> {
    let s = state.drafts.clone();
    blocking(move || s.create(account_id, reply_to, forward)).await
}
#[tauri::command]
pub async fn save_draft(
    state: State<'_, DesktopState>,
    draft: Draft,
    import_lease_ids: Option<Vec<String>>,
) -> AppResult<Draft> {
    let s = state.drafts.clone();
    // The active engine's repository (Task 4.3): never a hardcoded JSON path.
    let repository = state.repository();
    blocking(move || {
        let saved = s.save(draft)?;
        // This snapshot committed. Later picker responses have different IDs and
        // stay protected while their own asynchronous save is still pending.
        let root = Store::data_dir();
        let _ = portable_files::release_draft_attachment_imports(
            &root,
            &import_lease_ids.unwrap_or_default(),
        );
        let _ = repository.reconcile_storage();
        Ok(saved)
    })
    .await
}

#[tauri::command]
pub async fn release_attachment_imports(
    state: State<'_, DesktopState>,
    import_lease_ids: Vec<String>,
) -> AppResult<()> {
    // The active engine's repository (Task 4.3): never a hardcoded JSON path.
    let repository = state.repository();
    blocking(move || {
        let root = Store::data_dir();
        portable_files::release_draft_attachment_imports(&root, &import_lease_ids)?;
        repository.reconcile_storage()
    })
    .await
}
#[tauri::command]
pub async fn discard_draft(state: State<'_, DesktopState>, id: u64) -> AppResult<()> {
    let s = state.drafts.clone();
    blocking(move || s.discard(id)).await
}
#[tauri::command]
pub async fn send_draft(state: State<'_, DesktopState>, id: u64) -> AppResult<()> {
    let s = state.drafts.clone();
    blocking(move || s.send(id)).await
}
/// Messages dropped on the composer, each attached as its own `.eml` file
/// (the original message when the provider has it, as "Download .eml" does).
/// `existing` are the draft's current attachments: everything together must
/// stay within the size a draft can be sent with.
#[tauri::command]
pub async fn attach_mail_to_draft(
    state: State<'_, DesktopState>,
    ids: Vec<u64>,
    existing: Option<Vec<String>>,
) -> AppResult<portable_files::StagedAttachments> {
    use qamille::application::AppError;
    const MAX_MESSAGES: usize = 50;
    let mut unique = Vec::with_capacity(ids.len());
    for id in ids {
        if !unique.contains(&id) {
            unique.push(id);
        }
    }
    if unique.is_empty() {
        return Err(AppError::validation("Select at least one message"));
    }
    if unique.len() > MAX_MESSAGES {
        return Err(AppError::validation("Attach at most 50 messages at a time"));
    }
    let mail = state.mail.clone();
    blocking(move || {
        let root = Store::data_dir();
        // A missing file is reported when the draft is sent; it takes no room here.
        let used = existing
            .unwrap_or_default()
            .iter()
            .filter_map(|path| portable_files::resolve_path(path, &root).ok())
            .filter_map(|path| std::fs::metadata(path).ok())
            .filter(|metadata| metadata.is_file())
            .fold(0u64, |total, metadata| total.saturating_add(metadata.len()));
        let budget = qamille::rules::ATTACHMENT_CAP_BYTES.saturating_sub(used);
        let files = mail
            .export_eml_each(&unique, budget)?
            .into_iter()
            .map(|export| (export.filename.into(), export.bytes))
            .collect();
        portable_files::stage_draft_attachment_bytes(&root, files)
    })
    .await
}
#[tauri::command]
pub async fn pick_attachments(
    window: tauri::WebviewWindow,
    italian: Option<bool>,
) -> AppResult<portable_files::StagedAttachments> {
    // The dialog is owned by the calling window: modal to it, and never under it (tauri-native-desktop F1).
    let paths = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(if italian.unwrap_or(false) {
            "Allega file"
        } else {
            "Attach files"
        })
        .pick_files()
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|p| p.path().to_path_buf())
        .collect::<Vec<_>>();
    blocking(move || {
        portable_files::stage_draft_attachments(&qamille::model::Store::data_dir(), &paths)
    })
    .await
}
