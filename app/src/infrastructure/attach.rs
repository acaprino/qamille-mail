//! Attachment reading: turns PDF, Word, Excel, text, CSV, HTML and calendar files into plain text
//! for preview, search, AI and the MCP server. Plain text is read here; PDF and Office files go
//! to the compute server (spec D3: the document readers live in `qamille-server`).

use crate::application::compute::codes;
use crate::application::{AppError, AppResult};
use crate::model::Attachment;
use crate::rules::ATTACHMENT_CAP_BYTES;

pub use crate::domain::text::html_to_text;

/// Formats that need a document reader. Those run only in the compute server's parser
/// process (spec D2, INV-11); the app reads plain text itself. The same list as
/// `qamille_compute::extract::needs_reader` (the app does not link that crate, INV-1).
pub fn needs_reader(name: &str) -> bool {
    matches!(extension(name).as_str(), "pdf" | "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp")
}

fn extension(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_lowercase()
}

/// Text of a file that needs no document reader, as the former in-process reader gave it;
/// `None` for the formats of [`needs_reader`].
pub fn plain_text(name: &str, bytes: &[u8]) -> Option<Result<String, String>> {
    if needs_reader(name) {
        return None;
    }
    let text = match extension(name).as_str() {
        "html" | "htm" => html_to_text(&String::from_utf8_lossy(bytes)),
        "txt" | "md" | "csv" | "tsv" | "json" | "xml" | "ics" | "log" | "eml" | "rtf" | "yaml" | "yml" => {
            String::from_utf8_lossy(bytes).to_string()
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" => return Some(Err("Image file (no text to read).".into())),
        "wav" | "mp3" | "m4a" | "ogg" => return Some(Err("Audio file: open it to listen.".into())),
        _ => {
            let s = String::from_utf8_lossy(bytes);
            let printable = s.chars().filter(|c| !c.is_control() || c.is_whitespace()).count();
            if printable * 10 > s.chars().count() * 9 { s.to_string() } else { return Some(Err("Binary file, no text preview.".into())) }
        }
    };
    Some(Ok(text.trim().to_string()))
}

/// Text of a file with the error codes of the compute port: document formats go to the
/// compute server, so a pause (`compute::is_transient`) is told apart from a bad file.
pub fn document_text(name: &str, bytes: &[u8]) -> AppResult<String> {
    if bytes.len() as u64 > ATTACHMENT_CAP_BYTES {
        return Err(AppError::new(codes::PAYLOAD_TOO_LARGE, "File is larger than 25 MB."));
    }
    match plain_text(name, bytes) {
        Some(result) => result.map_err(|message| AppError::new(codes::DOCUMENT_UNREADABLE, message)),
        None => super::compute_factory::current().extract_document(name, bytes),
    }
}

pub fn bytes_to_text(name: &str, bytes: &[u8]) -> Result<String, String> {
    document_text(name, bytes).map_err(|error| error.message)
}

pub fn read_text(a: &Attachment) -> Result<String, String> {
    if let Some(t) = &a.inline_text {
        return Ok(t.clone());
    }
    let p = a.path.as_ref().ok_or("Attachment not downloaded.")?;
    let bytes = std::fs::read(p).map_err(|e| e.to_string())?;
    bytes_to_text(&a.name, &bytes)
}

pub fn human_size(n: u64) -> String {
    if n >= 1 << 20 {
        format!("{:.1} MB", n as f64 / (1u64 << 20) as f64)
    } else if n >= 1024 {
        format!("{} KB", n / 1024)
    } else {
        format!("{n} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn plain_formats_are_read_here_and_documents_are_not() {
        assert_eq!(plain_text("note.TXT", b"  ciao  ").unwrap().unwrap(), "ciao");
        assert_eq!(plain_text("page.html", b"<p>uno</p><p>due</p>").unwrap().unwrap(), "uno\ndue");
        assert_eq!(plain_text("blob", &[0u8; 32]).unwrap().unwrap_err(), "Binary file, no text preview.");
        for name in ["a.pdf", "b.DOCX", "c.xlsx", "d.pptx", "e.odt", "f.ods", "g.odp"] {
            assert!(plain_text(name, b"x").is_none() && needs_reader(name), "{name}");
        }
        assert_eq!(document_text("big.txt", &vec![b'a'; ATTACHMENT_CAP_BYTES as usize + 1]).unwrap_err().code, codes::PAYLOAD_TOO_LARGE);
        assert_eq!(document_text("blob", &[0u8; 32]).unwrap_err().code, codes::DOCUMENT_UNREADABLE);
    }
}
