//! Which attachment types have a visual preview. The pages are rendered by embedded
//! WebAssembly engines in the WebView, so no engine port is needed.

/// Extensions that can have a visual preview. The list is intentionally narrow:
/// legacy Office formats, macro-enabled files and other binaries keep the text extraction.
pub fn visual_extension(name: &str) -> Option<&'static str> {
    match name.rsplit('.').next().unwrap_or("").to_ascii_lowercase().as_str() {
        "pdf" => Some("pdf"),
        "docx" => Some("docx"),
        "xlsx" => Some("xlsx"),
        "pptx" => Some("pptx"),
        _ => None,
    }
}
