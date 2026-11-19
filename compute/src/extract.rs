//! Document reading: turns PDF, Word, Excel, PowerPoint, OpenDocument, text, CSV, HTML and
//! calendar files into plain text. Moved from the app's `attach::bytes_to_text` (spec D3); it
//! runs only inside the sacrificial parser process (INV-11).

use std::io::Read;

/// Per-file cap, as in the app (`ATTACHMENT_CAP_BYTES`).
pub const MAX_INPUT_BYTES: usize = 25 * 1024 * 1024;
/// Extracted text cap, as the embedding worker applied it.
pub const MAX_TEXT_BYTES: usize = 4 * 1024 * 1024;

/// True for the formats that need a document reader (and so the parser process).
/// Everything else is plain text the client can read by itself.
pub fn needs_reader(name: &str) -> bool {
    matches!(extension(name).as_str(), "pdf" | "docx" | "xlsx" | "pptx" | "odt" | "ods" | "odp")
}

fn extension(name: &str) -> String {
    name.rsplit('.').next().unwrap_or("").to_lowercase()
}

fn zip_entries(bytes: &[u8], prefix: &str) -> Result<String, String> {
    const MAX_EXTRACTED: u64 = MAX_TEXT_BYTES as u64;
    let mut z = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| e.to_string())?;
    if z.len() > 10_000 { return Err("Document contains too many archive entries".into()); }
    let mut names: Vec<String> = (0..z.len())
        .filter_map(|i| z.by_index(i).ok().map(|f| f.name().to_string()))
        .filter(|n| n.starts_with(prefix) && n.ends_with(".xml"))
        .collect();
    names.sort();
    let mut out = String::new();
    let mut extracted = 0u64;
    for n in names {
        let f = z.by_name(&n).map_err(|e| e.to_string())?;
        if extracted.saturating_add(f.size()) > MAX_EXTRACTED { return Err("Expanded document exceeds the 4 MiB working budget".into()); }
        let mut s = String::new();
        f.take(MAX_EXTRACTED - extracted + 1).read_to_string(&mut s).map_err(|e| e.to_string())?;
        extracted += s.len() as u64;
        if extracted > MAX_EXTRACTED { return Err("Expanded document exceeds the 4 MiB working budget".into()); }
        out.push_str(&strip_xml(&s));
        out.push('\n');
    }
    Ok(out)
}

pub fn bytes_to_text(name: &str, bytes: &[u8]) -> Result<String, String> {
    if bytes.len() > MAX_INPUT_BYTES {
        return Err("File is larger than 25 MB.".into());
    }
    let text = match extension(name).as_str() {
        "pdf" => pdf_extract::extract_text_from_mem(bytes).map_err(|e| format!("PDF: {e}"))?,
        "docx" => zip_entries(bytes, "word/document")?,
        "xlsx" => zip_entries(bytes, "xl/sharedStrings")?,
        "pptx" => zip_entries(bytes, "ppt/slides/slide")?,
        "odt" | "ods" | "odp" => zip_entries(bytes, "content")?,
        "html" | "htm" => html_to_text(&String::from_utf8_lossy(bytes)),
        "txt" | "md" | "csv" | "tsv" | "json" | "xml" | "ics" | "log" | "eml" | "rtf" | "yaml" | "yml" => {
            String::from_utf8_lossy(bytes).to_string()
        }
        "png" | "jpg" | "jpeg" | "gif" | "webp" => return Err("Image file (no text to read).".into()),
        "wav" | "mp3" | "m4a" | "ogg" => return Err("Audio file: open it to listen.".into()),
        _ => {
            let s = String::from_utf8_lossy(bytes);
            let printable = s.chars().filter(|c| !c.is_control() || c.is_whitespace()).count();
            if printable * 10 > s.chars().count() * 9 { s.to_string() } else { return Err("Binary file, no text preview.".into()) }
        }
    };
    Ok(text.trim().to_string())
}

/// [`bytes_to_text`] with the 4 MiB output cap of the protocol (D2).
pub fn extract(name: &str, bytes: &[u8]) -> Result<String, String> {
    let text = bytes_to_text(name, bytes)?;
    if text.len() > MAX_TEXT_BYTES {
        return Err("Extracted text exceeds the 4 MiB working budget".into());
    }
    Ok(text)
}

/// Plain text of XML: the same conversion as the app's `domain::text::strip_xml`.
pub fn strip_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len() / 2);
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in s.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' => {
                in_tag = false;
                let t = tag.to_lowercase();
                if t.starts_with("/w:p") || t.starts_with("br") || t.starts_with("/p") || t.starts_with("/tr") || t.starts_with("/div") || t.starts_with("/si") || t.starts_with("/a:p") {
                    out.push('\n');
                } else if t.starts_with("w:tab") || t.starts_with("/td") {
                    out.push('\t');
                }
            }
            c if in_tag => tag.push(c),
            c => out.push(c),
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&nbsp;", " ")
}

/// Plain text of HTML: the same conversion as the app's `domain::text::html_to_text`.
pub fn html_to_text(html: &str) -> String {
    // Drop <style> and <script> blocks first.
    let mut s = html.to_string();
    for tag in ["style", "script", "head"] {
        while let Some(a) = s.to_lowercase().find(&format!("<{tag}")) {
            let lower = s.to_lowercase();
            let end = lower[a..].find(&format!("</{tag}>")).map(|i| a + i + tag.len() + 3).unwrap_or(s.len());
            s.replace_range(a..end, "");
        }
    }
    let t = strip_xml(&s);
    let mut out = String::new();
    let mut blank = 0;
    for line in t.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() {
            blank += 1;
            if blank <= 1 {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(&l);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn docx(text: &str) -> Vec<u8> {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default()).unwrap();
        zip.write_all(format!("<w:document><w:body><w:p><w:r><w:t>{text}</w:t></w:r></w:p></w:body></w:document>").as_bytes()).unwrap();
        zip.finish().unwrap().into_inner()
    }

    #[test]
    fn compressed_document_cannot_expand_past_the_text_budget() {
        let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        zip.start_file("word/document.xml", zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
        zip.write_all(&vec![b'x'; 4 * 1024 * 1024 + 1]).unwrap();
        let bytes = zip.finish().unwrap().into_inner();
        assert!(bytes.len() < 100_000);
        assert!(bytes_to_text("compressed.docx", &bytes).unwrap_err().contains("4 MiB"));
    }

    #[test]
    fn office_text_is_read() {
        assert_eq!(bytes_to_text("fattura.docx", &docx("Fattura di ottobre &amp; novembre")).unwrap(), "Fattura di ottobre & novembre");
        assert!(needs_reader("A.PDF") && needs_reader("b.docx") && !needs_reader("c.txt") && !needs_reader("noextension"));
    }

    #[test]
    fn plain_and_binary_files_keep_their_messages() {
        assert_eq!(bytes_to_text("note.txt", b"  ciao  ").unwrap(), "ciao");
        assert_eq!(bytes_to_text("blob.bin", &[0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9]).unwrap_err(), "Binary file, no text preview.");
        assert_eq!(bytes_to_text("big.txt", &vec![b'a'; MAX_INPUT_BYTES + 1]).unwrap_err(), "File is larger than 25 MB.");
    }
}
