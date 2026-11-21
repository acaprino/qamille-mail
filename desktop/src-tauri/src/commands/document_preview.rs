use crate::runtime::{DesktopState, blocking};
use crate::saved_files::SavedFile;
use qamille::application::AppResult;
use tauri::{State, ipc::Response};

/// The document to preview, for the embedded WebAssembly engines in the WebView.
#[tauri::command]
pub async fn attachment_preview_source(
    state: State<'_, DesktopState>,
    email_id: u64,
    attachment_id: u64,
    path: Option<Vec<u32>>,
) -> AppResult<Response> {
    let previews = state.previews.clone();
    let path = path.unwrap_or_default();
    let frame = blocking(move || {
        let (kind, bytes) = previews.document_source(email_id, attachment_id, &path)?;
        Ok(encode_source_frame(kind, &bytes))
    })
    .await?;
    Ok(Response::new(frame))
}

/// Asks where to save the attachment, then writes it with the Mark-of-the-Web (`zone.rs`). `None` when
/// the dialog is cancelled; otherwise where it went and the token that opens it (`saved_files.rs`).
#[tauri::command]
pub async fn attachment_save(
    window: tauri::WebviewWindow,
    state: State<'_, DesktopState>,
    email_id: u64,
    attachment_id: u64,
    path: Option<Vec<u32>>,
    italian: Option<bool>,
) -> AppResult<Option<SavedFile>> {
    let previews = state.previews.clone();
    let path = path.unwrap_or_default();
    let (name, bytes) = blocking(move || previews.file_to_save(email_id, attachment_id, &path)).await?;
    // The dialog is owned by the calling window: modal to it, and never under it (tauri-native-desktop F1).
    let mut dialog = rfd::AsyncFileDialog::new()
        .set_parent(&window)
        .set_title(if italian.unwrap_or(false) { "Salva l'allegato" } else { "Save attachment" })
        .set_file_name(safe_file_name(&name));
    if let Some(extension) = std::path::Path::new(&name).extension().and_then(|e| e.to_str()) {
        if !extension.is_empty() && extension.len() <= 10 && extension.chars().all(|c| c.is_ascii_alphanumeric()) {
            dialog = dialog.add_filter(extension.to_ascii_uppercase(), &[extension]);
        }
    }
    let Some(file) = dialog.save_file().await else {
        return Ok(None);
    };
    let target = file.path().to_path_buf();
    blocking(move || {
        // Marked as coming from the Internet, so Windows warns before a dangerous attachment is opened.
        crate::zone::save_downloaded(&target, &bytes, None)?;
        Ok(Some(SavedFile::remember(target)))
    })
    .await
}

/// The sender chooses attachment names: keep only a plain file name.
fn safe_file_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .map(|c| if c.is_control() || "<>:\"|?*".contains(c) { '_' } else { c })
        .collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() { "attachment".to_string() } else { cleaned }
}

/// `[u32 LE metadata length][JSON {kind}][document bytes]`.
fn encode_source_frame(kind: &str, bytes: &[u8]) -> Vec<u8> {
    let metadata = serde_json::to_vec(&serde_json::json!({ "kind": kind })).unwrap_or_default();
    let mut frame = Vec::with_capacity(4 + metadata.len() + bytes.len());
    frame.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    frame.extend_from_slice(&metadata);
    frame.extend_from_slice(bytes);
    frame
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saved_file_names_never_carry_a_path_or_reserved_characters() {
        assert_eq!(safe_file_name("report.pdf"), "report.pdf");
        assert_eq!(safe_file_name("..\\..\\Windows\\evil.exe"), "evil.exe");
        assert_eq!(safe_file_name("a/b/c.docx"), "c.docx");
        assert_eq!(safe_file_name("we:ird|na?me.pdf"), "we_ird_na_me.pdf");
        assert_eq!(safe_file_name("..."), "attachment");
        assert_eq!(safe_file_name(""), "attachment");
    }

    #[test]
    fn source_frame_names_the_kind_before_the_document_bytes() {
        let frame = encode_source_frame("docx", b"PK");
        let len = u32::from_le_bytes(frame[0..4].try_into().expect("header")) as usize;
        let metadata: serde_json::Value =
            serde_json::from_slice(&frame[4..4 + len]).expect("metadata");
        assert_eq!(metadata["kind"], "docx");
        assert_eq!(&frame[4 + len..], b"PK");
    }
}
