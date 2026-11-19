use crate::runtime::{DesktopState, blocking};
use qamille::application::{AppResult, assistant::Answer, ports::SearchHit};
use tauri::State;
#[tauri::command]
pub async fn summarize_mail(state: State<'_, DesktopState>, id: u64) -> AppResult<String> {
    let s = state.assistant.clone();
    blocking(move || s.summarize(id)).await
}
#[tauri::command]
pub async fn suggest_reply(
    state: State<'_, DesktopState>,
    id: u64,
    instruction: String,
) -> AppResult<String> {
    let s = state.assistant.clone();
    blocking(move || s.reply(id, &instruction)).await
}
#[tauri::command]
pub async fn ask_assistant(state: State<'_, DesktopState>, question: String) -> AppResult<Answer> {
    let s = state.assistant.clone();
    blocking(move || s.ask(&question)).await
}
#[tauri::command]
pub async fn semantic_search(
    state: State<'_, DesktopState>,
    query: String,
) -> AppResult<Vec<SearchHit>> {
    let s = state.assistant.clone();
    blocking(move || s.search(&query)).await
}
#[tauri::command]
pub async fn index_mail(state: State<'_, DesktopState>) -> AppResult<String> {
    let s = state.indexer.clone();
    blocking(move || {
        s.configure(None, true, true)?;
        Ok("Full mailbox indexing queued. Progress is saved automatically; new mail is indexed in the background.".into())
    }).await
}

#[tauri::command]
pub async fn indexing_status(state: State<'_, DesktopState>) -> AppResult<Vec<qamille::domain::indexing::AccountIndexStatus>> {
    let service = state.indexer.clone();
    blocking(move || service.status()).await
}
#[tauri::command]
pub async fn configure_indexing(state: State<'_, DesktopState>, account_id: Option<u32>, enabled: bool, retry_partial: bool) -> AppResult<()> {
    let service = state.indexer.clone();
    blocking(move || service.configure(account_id, enabled, retry_partial)).await
}
// The model state comes from the compute server's `/health` (spec D3): off the main thread.
#[tauri::command]
pub async fn search_model_installed(state: State<'_, DesktopState>) -> AppResult<bool> {
    let s = state.assistant.clone();
    blocking(move || Ok(s.installed())).await
}
#[tauri::command]
pub async fn search_model_status(state: State<'_, DesktopState>) -> AppResult<qamille::application::ports::ModelInstallStatus> {
    let s = state.assistant.clone();
    blocking(move || Ok(s.install_status())).await
}

/// Settings → Compute (D7): the remote server's models, «Sign in again» after a `401`, the
/// warning of D4. Never a key.
#[tauri::command]
pub async fn compute_status() -> AppResult<qamille::infrastructure::compute_factory::ComputeStatus> {
    blocking(|| Ok(qamille::infrastructure::compute_factory::status())).await
}
/// D4 «Reindex»: every vector is made again with the model the server serves now.
#[tauri::command]
pub async fn search_reindex(state: State<'_, DesktopState>) -> AppResult<()> {
    let service = state.indexer.clone();
    blocking(move || service.reindex_vectors()).await
}
