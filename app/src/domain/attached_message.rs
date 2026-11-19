//! Messages carried inside another message (`message/rfc822` parts, `.eml`
//! files such as a PEC `postacert.eml`), read straight from their bytes.
//! A nested part is addressed by the attachment indexes walked from the outer
//! attachment, so `[]` is the attachment itself and `[1, 0]` is the first
//! attachment of the second attachment of that message.
use mail_parser::{Address, MessageParser, MimeHeaders, PartType};
use serde::Serialize;

/// Deeper chains are refused: real forwards nest two or three levels.
pub const MAX_DEPTH: usize = 8;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AttachedPart {
    pub index: u32,
    pub name: String,
    pub size: u64,
    pub mime: String,
    /// True when the part is itself a message that can be opened the same way.
    pub message: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AttachedMessage {
    pub subject: String,
    pub from_name: String,
    pub from_addr: String,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    /// Unix seconds; absent when the message has no Date header.
    pub date: Option<i64>,
    pub body: String,
    pub body_html: String,
    pub attachments: Vec<AttachedPart>,
}

/// Whether an attachment should open as a message rather than a document.
/// Keep in step with `desktop/src/lib/attached-message.ts`.
pub fn is_message(name: &str, mime: &str) -> bool {
    let mime = mime.trim();
    mime.eq_ignore_ascii_case("message/rfc822")
        || mime.eq_ignore_ascii_case("message/global")
        || name
            .rsplit_once('.')
            .is_some_and(|(_, extension)| extension.trim().eq_ignore_ascii_case("eml"))
}

fn people(address: Option<&Address>) -> Vec<String> {
    address
        .map(|list| {
            list.iter()
                .filter_map(|person| {
                    let addr = person.address()?.trim();
                    Some(match person.name().map(str::trim) {
                        Some(name) if !name.is_empty() && name != addr => format!("{name} <{addr}>"),
                        _ => addr.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

fn part_mime(part: &mail_parser::MessagePart) -> String {
    part.content_type()
        .map(|c| format!("{}/{}", c.ctype(), c.subtype().unwrap_or("octet-stream")))
        .unwrap_or_else(|| "application/octet-stream".into())
        .to_ascii_lowercase()
}

/// Extension for an unnamed part, so previews and text extraction, which pick
/// a format by extension, still recognise it.
fn extension_for(mime: &str) -> Option<&'static str> {
    Some(match mime {
        "message/rfc822" | "message/global" => "eml",
        "application/pdf" => "pdf",
        "application/vnd.openxmlformats-officedocument.wordprocessingml.document" => "docx",
        "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet" => "xlsx",
        "application/vnd.openxmlformats-officedocument.presentationml.presentation" => "pptx",
        "application/vnd.oasis.opendocument.text" => "odt",
        "application/vnd.oasis.opendocument.spreadsheet" => "ods",
        "application/vnd.oasis.opendocument.presentation" => "odp",
        "application/xml" | "text/xml" => "xml",
        "application/json" => "json",
        "text/plain" => "txt",
        "text/html" => "html",
        "text/csv" => "csv",
        "text/calendar" => "ics",
        "image/png" => "png",
        "image/jpeg" => "jpg",
        "image/gif" => "gif",
        "image/webp" => "webp",
        _ => return None,
    })
}

fn part_name(part: &mail_parser::MessagePart) -> String {
    if let Some(name) = part.attachment_name().map(str::trim).filter(|n| !n.is_empty()) {
        return name.to_string();
    }
    let mime = if matches!(part.body, PartType::Message(_)) {
        "message/rfc822".to_string()
    } else {
        part_mime(part)
    };
    match extension_for(&mime) {
        Some("eml") => "message.eml".into(),
        Some(extension) => format!("attachment.{extension}"),
        None => "attachment".into(),
    }
}

fn parsed(raw: &[u8]) -> Result<mail_parser::Message<'_>, String> {
    let message = MessageParser::default()
        .parse(raw)
        .ok_or("The attached message could not be read.")?;
    // The parser accepts any text; a message needs at least one real mail header.
    let looks_like_mail = message.from().is_some()
        || message.to().is_some()
        || message.subject().is_some()
        || message.date().is_some()
        || message.message_id().is_some();
    if !looks_like_mail {
        return Err("The attachment is not an email message.".into());
    }
    Ok(message)
}

pub fn parse(raw: &[u8]) -> Result<AttachedMessage, String> {
    let message = parsed(raw)?;
    let (from_name, from_addr) = message
        .from()
        .and_then(|from| from.first())
        .map(|person| {
            let addr = person.address().unwrap_or("").trim().to_string();
            let name = person.name().map(str::trim).filter(|n| !n.is_empty());
            (name.map(String::from).unwrap_or_else(|| addr.clone()), addr)
        })
        .unwrap_or_default();
    let body_html = match message.html_part(0).map(|part| &part.body) {
        Some(PartType::Html(html)) if !html.trim().is_empty() => html.to_string(),
        _ => String::new(),
    };
    let attachments = message
        .attachments()
        .enumerate()
        .map(|(index, part)| {
            let name = part_name(part);
            let mime = part_mime(part);
            AttachedPart {
                index: index as u32,
                size: part.contents().len() as u64,
                message: matches!(part.body, PartType::Message(_)) || is_message(&name, &mime),
                name,
                mime,
            }
        })
        .collect();
    Ok(AttachedMessage {
        subject: message
            .subject()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .unwrap_or("(no subject)")
            .to_string(),
        from_name,
        from_addr,
        to: people(message.to()),
        cc: people(message.cc()),
        date: message.date().map(|d| d.to_timestamp()),
        body: message.body_text(0).map(|t| t.to_string()).unwrap_or_default(),
        body_html,
        attachments,
    })
}

/// The name and bytes of the part reached by `path`, starting from a message's
/// raw bytes. An empty path returns the input unchanged with no name.
pub fn nested(raw: &[u8], path: &[u32]) -> Result<(Option<String>, Vec<u8>), String> {
    if path.len() > MAX_DEPTH {
        return Err("The attachment is nested too deeply to open.".into());
    }
    let mut name = None;
    let mut current = raw.to_vec();
    for &index in path {
        let (next_name, next) = {
            let message = parsed(&current)?;
            let part = message
                .attachment(index)
                .ok_or("The attachment is no longer available.")?;
            (part_name(part), part.contents().to_vec())
        };
        name = Some(next_name);
        current = next;
    }
    Ok((name, current))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pec() -> Vec<u8> {
        let inner = "From: Ufficio Tributi <protocollo@pec.comune.example>\r\n\
To: mario.rossi@pec.example\r\n\
Subject: Avviso di accertamento\r\n\
Date: Tue, 3 Nov 2026 14:20:00 +0100\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"in\"\r\n\
\r\n\
--in\r\n\
Content-Type: text/html; charset=utf-8\r\n\
\r\n\
<p>Gentile contribuente</p>\r\n\
--in\r\n\
Content-Type: application/pdf; name=\"Avviso.pdf\"\r\n\
Content-Disposition: attachment; filename=\"Avviso.pdf\"\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQK\r\n\
--in--\r\n";
        format!(
            "From: Posta Certificata <posta-certificata@pec.example>\r\n\
To: mario.rossi@example.com\r\n\
Subject: POSTA CERTIFICATA: Avviso\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"out\"\r\n\
\r\n\
--out\r\n\
Content-Type: text/plain; charset=utf-8\r\n\
\r\n\
Messaggio di posta certificata\r\n\
--out\r\n\
Content-Type: application/xml; name=\"daticert.xml\"\r\n\
Content-Disposition: attachment; filename=\"daticert.xml\"\r\n\
\r\n\
<postacert/>\r\n\
--out\r\n\
Content-Type: message/rfc822; name=\"postacert.eml\"\r\n\
Content-Disposition: attachment; filename=\"postacert.eml\"\r\n\
\r\n\
{inner}\
--out--\r\n"
        )
        .into_bytes()
    }

    #[test]
    fn names_unnamed_parts_from_their_type() {
        let raw = b"From: a@example.com\r\n\
Subject: Unnamed\r\n\
MIME-Version: 1.0\r\n\
Content-Type: multipart/mixed; boundary=\"b\"\r\n\
\r\n\
--b\r\n\
Content-Type: text/plain\r\n\
\r\n\
body\r\n\
--b\r\n\
Content-Type: application/pdf\r\n\
Content-Disposition: attachment\r\n\
Content-Transfer-Encoding: base64\r\n\
\r\n\
JVBERi0xLjQK\r\n\
--b\r\n\
Content-Type: application/x-unknown\r\n\
Content-Disposition: attachment\r\n\
\r\n\
data\r\n\
--b--\r\n";
        let names: Vec<_> = parse(raw).unwrap().attachments.into_iter().map(|a| a.name).collect();
        assert_eq!(names, ["attachment.pdf", "attachment"]);
        assert_eq!(nested(raw, &[0]).unwrap().0.as_deref(), Some("attachment.pdf"));
    }

    #[test]
    fn recognises_messages_by_extension_or_type() {
        assert!(is_message("postacert.eml", "application/octet-stream"));
        assert!(is_message("POSTACERT.EML", ""));
        assert!(is_message("attachment", "message/rfc822"));
        assert!(is_message("attachment", "Message/Global"));
        assert!(!is_message("daticert.xml", "application/xml"));
        assert!(!is_message("eml", "text/plain"));
    }

    #[test]
    fn reads_a_pec_envelope_and_lists_its_message_part() {
        let envelope = parse(&pec()).unwrap();
        assert_eq!(envelope.subject, "POSTA CERTIFICATA: Avviso");
        let names: Vec<_> = envelope.attachments.iter().map(|a| (a.name.as_str(), a.message)).collect();
        assert_eq!(names, [("daticert.xml", false), ("postacert.eml", true)]);
    }

    #[test]
    fn opens_postacert_with_headers_html_body_and_its_own_attachments() {
        let (name, bytes) = nested(&pec(), &[1]).unwrap();
        assert_eq!(name.as_deref(), Some("postacert.eml"));
        let message = parse(&bytes).unwrap();
        assert_eq!(message.subject, "Avviso di accertamento");
        assert_eq!(message.from_name, "Ufficio Tributi");
        assert_eq!(message.from_addr, "protocollo@pec.comune.example");
        assert_eq!(message.to, ["mario.rossi@pec.example"]);
        assert!(message.date.is_some());
        assert!(message.body_html.contains("Gentile contribuente"));
        assert_eq!(message.attachments.len(), 1);
        assert_eq!(message.attachments[0].name, "Avviso.pdf");
        assert!(!message.attachments[0].message);
    }

    #[test]
    fn walks_two_levels_to_a_document_inside_the_attached_message() {
        let (name, bytes) = nested(&pec(), &[1, 0]).unwrap();
        assert_eq!(name.as_deref(), Some("Avviso.pdf"));
        assert!(bytes.starts_with(b"%PDF-1.4"));
    }

    #[test]
    fn refuses_missing_parts_non_messages_and_excessive_depth() {
        assert!(nested(&pec(), &[7]).is_err());
        assert!(parse(b"just some text without headers").is_err());
        assert!(nested(&pec(), &[0; MAX_DEPTH + 1]).is_err());
        let (_, xml) = nested(&pec(), &[0]).unwrap();
        assert!(nested(&xml, &[0]).is_err());
    }
}
