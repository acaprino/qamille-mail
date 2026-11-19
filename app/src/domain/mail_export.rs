//! Portable message exports and account-scoped conversation membership.
use super::model::{Email, Store};
use lettre::{
    Message,
    message::{Attachment, Mailbox, MultiPart, SinglePart, header::ContentType},
};
use std::collections::HashMap;

pub(super) fn key(value: &str) -> &str {
    value.trim().trim_start_matches('<').trim_end_matches('>')
}

/// Resolve provider thread IDs and reply chains without joining different accounts
/// or treating missing IDs as a shared conversation.
pub fn conversation_ids(emails: &[Email]) -> HashMap<u64, u64> {
    fn root(parents: &mut [usize], mut index: usize) -> usize {
        while parents[index] != index {
            parents[index] = parents[parents[index]];
            index = parents[index];
        }
        index
    }
    let mut parents: Vec<_> = (0..emails.len()).collect();
    let mut links = HashMap::new();
    for (index, email) in emails.iter().enumerate() {
        for link in [key(&email.message_id), key(&email.thread_id)] {
            if link.is_empty() { continue; }
            if let Some(&other) = links.get(&(email.account_id, link)) {
                let a = root(&mut parents, index);
                let b = root(&mut parents, other);
                parents[a.max(b)] = a.min(b);
            } else {
                links.insert((email.account_id, link), index);
            }
        }
    }
    emails.iter().enumerate().map(|(index, email)| {
        (email.id, emails[root(&mut parents, index)].id)
    }).collect()
}

pub fn thread(store: &Store, anchor: &Email) -> Vec<Email> {
    let groups = conversation_ids(&store.emails);
    let group = groups.get(&anchor.id);
    let mut emails: Vec<_> = store
        .emails
        .iter()
        .filter(|e| e.account_id == anchor.account_id &&
            (e.id == anchor.id || group.is_some_and(|g| groups.get(&e.id) == Some(g))))
        .cloned()
        .collect();
    emails.sort_by_key(|e| (e.date, e.id));
    emails
}

pub fn filename(subject: &str, id: u64, extension: &str) -> String {
    let safe: String = subject
        .chars()
        .map(|c| {
            if c.is_control() || r#"\/:*?"<>|"#.contains(c) {
                '_'
            } else {
                c
            }
        })
        .take(80)
        .collect();
    let safe = safe.trim().trim_end_matches('.');
    format!(
        "email-{id}-{}.{}",
        if safe.is_empty() { "message" } else { safe },
        extension
    )
}

/// Rebuild local/sample mail with standards-compliant MIME encoding. Received mail
/// uses the original server bytes instead, preserving HTML, headers and attachments.
pub fn eml(email: &Email, attachments: Vec<Vec<u8>>) -> Result<Vec<u8>, String> {
    if attachments.len() != email.attachments.len() {
        return Err("Missing attachment data".into());
    }
    let mut message = Message::builder()
        .from(Mailbox::new(
            Some(email.from_name.replace(['\r', '\n'], " ")),
            email
                .from_addr
                .parse()
                .map_err(|e| format!("Invalid sender: {e}"))?,
        ))
        .subject(email.subject.replace(['\r', '\n'], " "));
    for to in &email.to {
        message = message.to(to.parse().map_err(|e| format!("Invalid recipient: {e}"))?);
    }
    for cc in &email.cc {
        message = message.cc(cc.parse().map_err(|e| format!("Invalid recipient: {e}"))?);
    }
    let date = chrono::DateTime::from_timestamp(email.date, 0).ok_or("Invalid message date")?;
    message = message.date(date.into());
    if !email.message_id.is_empty() && !email.message_id.contains(['\r', '\n']) {
        message = message.message_id(Some(format!("<{}>", key(&email.message_id))));
    }
    let plain = SinglePart::plain(email.body.clone());
    let result = if attachments.is_empty() {
        message.singlepart(plain)
    } else {
        let mut multipart = MultiPart::mixed().singlepart(plain);
        for (meta, bytes) in email.attachments.iter().zip(attachments) {
            let mime = meta
                .mime
                .parse::<ContentType>()
                .unwrap_or(ContentType::parse("application/octet-stream").unwrap());
            multipart = multipart.singlepart(
                Attachment::new(meta.name.replace(['\r', '\n'], "_")).body(bytes, mime),
            );
        }
        message.multipart(multipart)
    };
    result.map(|m| m.formatted()).map_err(|e| e.to_string())
}

/// mboxrd: LF separators and one extra '>' before every body line matching >*From.
pub fn append_mbox(output: &mut Vec<u8>, email: &Email, raw: &[u8]) {
    let sender: String = email
        .from_addr
        .chars()
        .filter(|c| !c.is_whitespace() && !c.is_control())
        .collect();
    let date = chrono::DateTime::from_timestamp(email.date, 0).unwrap_or_default();
    output.extend_from_slice(
        format!(
            "From {} {}\n",
            if sender.is_empty() {
                "MAILER-DAEMON"
            } else {
                &sender
            },
            date.format("%a %b %e %H:%M:%S %Y")
        )
        .as_bytes(),
    );
    let mut body = false;
    for line in raw.split_inclusive(|b| *b == b'\n') {
        let line = line.strip_suffix(b"\n").unwrap_or(line);
        let line = line.strip_suffix(b"\r").unwrap_or(line);
        if body
            && line
                .iter()
                .copied()
                .skip_while(|b| *b == b'>')
                .collect::<Vec<_>>()
                .starts_with(b"From ")
        {
            output.push(b'>');
        }
        output.extend_from_slice(line);
        output.push(b'\n');
        if line.is_empty() {
            body = true;
        }
    }
    output.push(b'\n');
}
