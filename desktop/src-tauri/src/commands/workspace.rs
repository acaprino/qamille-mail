use crate::runtime::{DesktopState, blocking};
use qamille::{
    application::{
        AppError, AppResult,
        workspace::{SettingsUpdate, WorkspaceSnapshot},
    },
    model::{Event, Meeting, Signature},
};
use tauri::State;

#[tauri::command]
pub async fn mcp_configuration() -> AppResult<serde_json::Value> {
    blocking(move || {
        let executable=std::env::current_exe()?;
        let config=qamille::infrastructure::mcp_config::load()?;
        let worker=qamille::model::Store::data_dir().join("mcp-worker.lock");
        let running=std::fs::OpenOptions::new().read(true).write(true).open(worker).ok().is_some_and(|file|file.try_lock().is_err());
        Ok(serde_json::json!({"config":config,"worker_running":running,"client_config":{"mcpServers":{"qamille":{"command":executable.to_string_lossy(),"args":["--mcp"]}}}}))
    }).await
}
#[tauri::command]
pub async fn save_mcp_configuration(config:qamille::infrastructure::mcp_config::Config)->AppResult<()> {
    blocking(move || {
        qamille::infrastructure::mcp_config::save_with_autostart(&config)
    }).await
}
#[tauri::command]
pub async fn start_mcp_worker()->AppResult<()> {
    blocking(move || {
        let mut command=std::process::Command::new(std::env::current_exe()?);
        command.arg("--worker").stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
        #[cfg(windows)] {use std::os::windows::process::CommandExt;command.creation_flags(0x08000000);}
        command.spawn()?;Ok(())
    }).await
}

#[tauri::command]
pub async fn workspace_snapshot(state: State<'_, DesktopState>) -> AppResult<WorkspaceSnapshot> {
    let s = state.workspace.clone();
    blocking(move || s.snapshot()).await
}
#[tauri::command]
pub async fn save_settings(
    state: State<'_, DesktopState>,
    update: SettingsUpdate,
) -> AppResult<()> {
    let s = state.workspace.clone();
    blocking(move || s.save_settings(update)).await
}
#[tauri::command]
pub async fn list_events(state: State<'_, DesktopState>) -> AppResult<Vec<Event>> {
    let s = state.workspace.clone();
    blocking(move || s.events()).await
}
#[tauri::command]
pub async fn save_event(state: State<'_, DesktopState>, event: Event) -> AppResult<Event> {
    let s = state.workspace.clone();
    blocking(move || {
        if qamille::mcp::calendar::is_remote_projection(event.id)? {return Err(AppError::new("remote_event","Edit the provider event from Calendar"));}
        s.save_event(event)
    }).await
}
#[tauri::command]
pub async fn delete_event(state: State<'_, DesktopState>, id: u64) -> AppResult<()> {
    let s = state.workspace.clone();
    blocking(move || {
        if qamille::mcp::calendar::is_remote_projection(id)? {return Err(AppError::new("remote_event","Delete the provider event from Calendar"));}
        s.delete_event(id)
    }).await
}
#[tauri::command]
pub async fn list_meetings(state: State<'_, DesktopState>) -> AppResult<Vec<Meeting>> {
    let s = state.meetings.clone();
    blocking(move || s.list().map(|items| items.into_iter().map(|d| d.meeting).collect())).await
}
#[tauri::command]
pub async fn save_meeting_notes(
    state: State<'_, DesktopState>,
    id: u64,
    notes: String,
) -> AppResult<()> {
    let s = state.meetings.clone();
    blocking(move || { let revision = s.read(id)?.revision; s.save_notes(id, notes, revision).map(|_| ()) }).await
}
#[tauri::command]
pub async fn save_signature(
    state: State<'_, DesktopState>,
    signature: Signature,
) -> AppResult<Signature> {
    let s = state.workspace.clone();
    let saved = blocking(move || s.save_signature(signature)).await;
    // The text of the signature of a mailbox travels with it.
    state.sync.nudge();
    saved
}
#[tauri::command]
pub async fn delete_signature(state: State<'_, DesktopState>, id: u64) -> AppResult<()> {
    let s = state.workspace.clone();
    blocking(move || s.delete_signature(id)).await
}
#[tauri::command]
pub async fn open_link(url: String, window: tauri::Window) -> AppResult<()> {
    // Other windows, such as the meeting chip, open only engine-chosen links.
    if window.label() != "main" {
        return Err(AppError::new("permission_denied", "This window cannot open links"));
    }
    blocking(move || {
        if !(url.starts_with("https://")
            || url.starts_with("http://")
            || url.starts_with("mailto:"))
            || url.contains(['\r', '\n', '\0'])
        {
            return Err(AppError::validation("This link type is not supported"));
        }
        open::that(url).map_err(|e| AppError::external(e.to_string()))
    })
    .await
}
