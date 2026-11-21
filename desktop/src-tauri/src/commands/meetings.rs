use crate::runtime::{DesktopState, blocking};
use qamille::{application::{AppError, AppResult}, domain::meeting::*};
use tauri::State;

#[tauri::command]
pub async fn meeting_devices(state: State<'_, DesktopState>) -> AppResult<Vec<AudioDevice>> { let s = state.meetings.clone(); blocking(move || s.devices()).await }
#[tauri::command]
pub async fn meeting_list(state: State<'_, DesktopState>) -> AppResult<Vec<MeetingDetail>> { let s = state.meetings.clone(); blocking(move || s.list()).await }
#[tauri::command]
pub async fn meeting_list_page(offset: Option<usize>, limit: Option<usize>, from_date: Option<String>, to_date: Option<String>, state: Option<String>, app: State<'_, DesktopState>) -> AppResult<MeetingListPage> {
    let s = app.meetings.clone(); blocking(move || s.list_page(offset.unwrap_or(0), limit.unwrap_or(50), from_date.as_deref(), to_date.as_deref(), state.as_deref())).await
}
#[tauri::command]
pub async fn meeting_read(id: u64, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.read(id)).await }
#[tauri::command]
pub async fn meeting_start(request: StartMeeting, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> {
    // Recovery may still be marking Preparing/Recording meetings Interrupted; a new capture would be misread.
    if !state.is_recovered() { return Err(AppError::validation("Startup recovery is still running")); }
    let s = state.meetings.clone(); blocking(move || s.start(request)).await
}
#[tauri::command]
pub async fn meeting_capture_status(state: State<'_, DesktopState>) -> AppResult<CaptureSnapshot> { let s = state.meetings.clone(); blocking(move || { let mut snapshot = s.capture_status()?; snapshot.chunks.clear(); snapshot.gaps.clear(); snapshot.unverified_chunks.clear(); Ok(snapshot) }).await }
#[tauri::command]
pub async fn meeting_pause(state: State<'_, DesktopState>) -> AppResult<CaptureSnapshot> { let s = state.meetings.clone(); blocking(move || s.pause()).await }
#[tauri::command]
pub async fn meeting_resume(state: State<'_, DesktopState>) -> AppResult<CaptureSnapshot> { let s = state.meetings.clone(); blocking(move || s.resume()).await }
#[tauri::command]
pub async fn meeting_stop(state: State<'_, DesktopState>) -> AppResult<CaptureSnapshot> { let s = state.meetings.clone(); blocking(move || s.stop()).await }
#[tauri::command]
pub async fn meeting_recover(id: u64, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.recover(id)).await }
#[tauri::command]
pub async fn meeting_save_notes(id: u64, text: String, expected_revision: u64, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.save_notes(id, text, expected_revision)).await }
#[tauri::command]
pub async fn meeting_add_note(id: u64, text: String, at_ms: Option<u64>, expected_revision: u64, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.add_note(id, text, at_ms, expected_revision)).await }
#[tauri::command]
pub async fn meeting_update(id: u64, title: String, event_id: Option<u64>, included_in_search: bool, expected_revision: u64, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.update(id, title, event_id, included_in_search, expected_revision)).await }
#[tauri::command]
pub async fn meeting_delete(id: u64, state: State<'_, DesktopState>) -> AppResult<()> { let s = state.meetings.clone(); blocking(move || s.delete(id)).await }
#[tauri::command]
pub async fn meeting_import(window: tauri::WebviewWindow, state: State<'_, DesktopState>) -> AppResult<Option<MeetingDetail>> {
    let s = state.meetings.clone();
    blocking(move || {
        let it = crate::window_shell::italian();
        let dialog = rfd::FileDialog::new()
            .set_parent(&window)
            .set_title(if it { "Importa una registrazione" } else { "Import a recording" })
            .add_filter(if it { "File audio" } else { "Audio files" }, &["wav", "mp3", "m4a", "aac", "flac"]);
        match dialog.pick_file() { Some(path) => s.import(path).map(Some), None => Ok(None) }
    }).await
}
#[tauri::command]
pub async fn meeting_transcribe(id: u64, options: TranscribeOptions, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.transcribe(id, options)).await }
#[tauri::command]
pub async fn meeting_summarize(id: u64, language: String, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.summarize(id, language)).await }
/// What the engine chosen in Settings can do now: the page says what to fix instead of offering a dead button.
#[tauri::command]
pub async fn meeting_readiness(state: State<'_, DesktopState>) -> AppResult<MeetingReadiness> { let s = state.meetings.clone(); blocking(move || s.readiness()).await }
/// Transcribes with the engine, model and language of Settings; says why when that engine cannot work yet.
#[tauri::command]
pub async fn meeting_transcribe_default(id: u64, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.transcribe_default(id)).await }
#[tauri::command]
pub async fn meeting_summarize_default(id: u64, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.summarize_default(id)).await }
/// Deletes the audio that was kept after a transcription that did not make it useless.
#[tauri::command]
pub async fn meeting_discard_audio(id: u64, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.discard_audio(id)).await }
#[tauri::command]
pub async fn meeting_audio_cleanup_preview(state: State<'_, DesktopState>) -> AppResult<AudioCleanup> { let s = state.meetings.clone(); blocking(move || s.audio_cleanup_preview()).await }
#[tauri::command]
pub async fn meeting_audio_cleanup(state: State<'_, DesktopState>) -> AppResult<AudioCleanup> { let s = state.meetings.clone(); blocking(move || s.audio_cleanup()).await }
#[tauri::command]
pub async fn meeting_cancel_job(id: u64, job_id: String, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.cancel_job(id, &job_id)).await }
#[tauri::command]
pub async fn meeting_retry_job(id: u64, job_id: String, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.retry_job(id, &job_id)).await }
#[tauri::command]
pub async fn meeting_correct_segment(id: u64, version_id: String, segment_id: String, text: String, expected_revision: u64, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.correct_segment(id, &version_id, &segment_id, text, expected_revision)).await }
#[tauri::command]
pub async fn meeting_toggle_action(id: u64, action_id: String, done: bool, expected_revision: u64, state: State<'_, DesktopState>) -> AppResult<MeetingDetail> { let s = state.meetings.clone(); blocking(move || s.toggle_action(id, &action_id, done, expected_revision)).await }
#[tauri::command]
pub async fn meeting_search(query: String, state: State<'_, DesktopState>) -> AppResult<Vec<MeetingHit>> { let s = state.meetings.clone(); blocking(move || s.search(&query)).await }
#[tauri::command]
pub async fn meeting_audio_range(id: u64, start_ms: u64, duration_ms: u64, state: State<'_, DesktopState>) -> AppResult<Vec<u8>> { let s = state.meetings.clone(); blocking(move || s.audio_range(id, start_ms, duration_ms)).await }
#[tauri::command]
pub async fn meeting_export(id: u64, format: String, transcript_version: Option<String>, summary_version: Option<String>, sections: Option<Vec<String>>, window: tauri::WebviewWindow, state: State<'_, DesktopState>) -> AppResult<Option<String>> {
    let s = state.meetings.clone();
    blocking(move || {
        let extension = match format.as_str() { "markdown" => "md", "text" => "txt", "json" => "json", _ => return Err(AppError::validation("Formato non supportato.")) };
        let content = s.export_selected(id, &format, transcript_version.as_deref(), summary_version.as_deref(), sections.as_deref())?;
        let it = crate::window_shell::italian();
        let kind = match extension { "md" => "Markdown", "txt" => if it { "Testo" } else { "Text" }, _ => "JSON" };
        let dialog = rfd::FileDialog::new()
            .set_parent(&window)
            .set_title(if it { "Esporta la riunione" } else { "Export the meeting" })
            .add_filter(kind, &[extension])
            .set_file_name(format!("meeting-{id}.{extension}"));
        if let Some(path) = dialog.save_file() {
            std::fs::write(&path, content)?;
            return Ok(Some(path.display().to_string()));
        }
        Ok(None)
    }).await
}
#[tauri::command]
pub async fn meeting_models(state: State<'_, DesktopState>) -> AppResult<Vec<SpeechModel>> { let s = state.meetings.clone(); blocking(move || s.models()).await }

#[tauri::command]
pub async fn meeting_confirm_close(window: tauri::WebviewWindow, italian: Option<bool>) -> AppResult<bool> {
    let italian = italian.unwrap_or(true);
    // Buttons named by what they do; Esc and the X keep recording.
    let (stop, keep) = if italian { ("Termina e chiudi", "Continua a registrare") } else { ("Stop and close", "Keep recording") };
    // «Esci» from the tray: the window comes back with the question it owns.
    crate::background::restore(tauri::Manager::app_handle(&window));
    blocking(move || Ok(rfd::MessageDialog::new().set_parent(&window).set_title(if italian { "Registrazione in corso" } else { "Recording in progress" })
        .set_description(if italian {
            "Chiudendo Qamille la registrazione si ferma e viene salvata."
        } else {
            "Closing Qamille stops the recording and saves it."
        })
        .set_buttons(rfd::MessageButtons::OkCancelCustom(stop.into(), keep.into()))
        .show() == rfd::MessageDialogResult::Custom(stop.into()))).await
}
#[tauri::command]
pub async fn meeting_cancel_all_jobs(state: State<'_, DesktopState>) -> AppResult<()> { state.meetings.cancel_all_jobs(); Ok(()) }
