//! Gmail drafts adapter; public application ports remain provider-independent.
use crate::{
    application::{AppError, AppResult},
    domain::{
        model::{Account, BodyFormat, Draft, Store},
        outbox::RemoteDraft,
    },
};
use serde_json::{Value, json};
fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .build()
        .into()
}
const BASE: &str = "https://gmail.googleapis.com/gmail/v1/users/me/drafts";

pub fn encode(bytes: &[u8]) -> String {
    const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";
    let mut out = String::new();
    for c in bytes.chunks(3) {
        let bits = ((c[0] as u32) << 16)
            | ((c.get(1).copied().unwrap_or(0) as u32) << 8)
            | c.get(2).copied().unwrap_or(0) as u32;
        out.push(ABC[((bits >> 18) & 63) as usize] as char);
        out.push(ABC[((bits >> 12) & 63) as usize] as char);
        if c.len() > 1 {
            out.push(ABC[((bits >> 6) & 63) as usize] as char);
        }
        if c.len() > 2 {
            out.push(ABC[(bits & 63) as usize] as char);
        }
    }
    out
}
pub fn decode(text: &str) -> AppResult<Vec<u8>> {
    let mut out = Vec::new();
    let mut bits = 0u32;
    let mut n = 0;
    for ch in text.bytes().filter(|c| *c != b'=') {
        let v = match ch {
            b'A'..=b'Z' => ch - b'A',
            b'a'..=b'z' => ch - b'a' + 26,
            b'0'..=b'9' => ch - b'0' + 52,
            b'-' | b'+' => 62,
            b'_' | b'/' => 63,
            _ => {
                return Err(AppError::external(
                    "Invalid base64 draft returned by provider",
                ));
            }
        };
        bits = (bits << 6) | u32::from(v);
        n += 6;
        if n >= 8 {
            n -= 8;
            out.push((bits >> n) as u8);
            bits &= (1 << n) - 1;
        }
    }
    Ok(out)
}
fn token(account: &Account) -> AppResult<String> {
    if account.oauth_client_id.is_empty() {
        return Err(AppError::new(
            "sign_in_required",
            "Reconnect using Google OAuth to synchronize cloud drafts",
        ));
    }
    super::oauth::access_token(account).map_err(super::oauth::provider_error)
}
fn path(id: &str) -> AppResult<String> {
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        return Err(AppError::validation("Invalid remote draft identifier"));
    }
    Ok(format!("{BASE}/{id}"))
}
fn get(url: &str, token: &str) -> AppResult<Value> {
    agent()
        .get(url)
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .map_err(|e| AppError::external(format!("Gmail draft read failed: {e}")))?
        .body_mut()
        .read_json()
        .map_err(|e| AppError::external(e.to_string()))
}
fn parse(value: &Value, account: &Account) -> AppResult<RemoteDraft> {
    let id = value["id"]
        .as_str()
        .ok_or_else(|| AppError::external("Draft id missing from Gmail response"))?
        .to_owned();
    path(&id)?;
    let raw = value["message"]["raw"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    let bytes = decode(&raw)?;
    let directory = Store::data_dir().join("draft-attachments").join(&id);
    std::fs::create_dir_all(&directory)?;
    let email = super::mail::parse_message(&bytes, 0, account.id, &directory)
        .ok_or_else(|| AppError::external("Gmail draft contains invalid MIME"))?;
    let parsed = mail_parser::MessageParser::default()
        .parse(&bytes)
        .ok_or_else(|| AppError::external("Cannot parse cloud draft"))?;
    let bcc = parsed
        .bcc()
        .map(|a| {
            a.iter()
                .filter_map(|a| a.address().map(String::from))
                .collect::<Vec<_>>()
                .join(", ")
        })
        .unwrap_or_default();
    let html = parsed.body_html(0).map(|h| h.to_string());
    let draft = Draft {
        account_id: account.id,
        to: email.to.join(", "),
        cc: email.cc.join(", "),
        bcc,
        subject: email.subject,
        body: html.clone().unwrap_or(email.body),
        format: if html.is_some() {
            BodyFormat::Html
        } else {
            BodyFormat::Plain
        },
        attachments: email
            .attachments
            .into_iter()
            .filter_map(|a| a.path)
            .collect(),
        cloud_sync_key: parsed
            .header_raw("X-Qamille-Draft-Id")
            // Drafts saved before the rebrandings carry an old header.
            .or_else(|| parsed.header_raw("X-Qemille-Draft-Id"))
            .or_else(|| parsed.header_raw("X-Sparklone-Draft-Id"))
            .unwrap_or_default()
            .trim()
            .to_string(),
        ..Default::default()
    };
    Ok(RemoteDraft {
        id,
        revision: value["message"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_string(),
        raw,
        thread_id: value["message"]["threadId"].as_str().map(str::to_owned),
        draft,
    })
}
pub fn list(account: &Account) -> AppResult<Vec<RemoteDraft>> {
    let token = token(account)?;
    let mut all = Vec::new();
    let mut page = String::new();
    loop {
        let url = if page.is_empty() {
            format!("{BASE}?maxResults=100")
        } else {
            format!("{BASE}?maxResults=100&pageToken={page}")
        };
        let result = get(&url, &token)?;
        for draft in result["drafts"].as_array().into_iter().flatten() {
            let id = draft["id"]
                .as_str()
                .ok_or_else(|| AppError::external("Missing remote draft id"))?;
            let value = get(&format!("{}?format=raw", path(id)?), &token)?;
            all.push(parse(&value, account)?);
        }
        match result["nextPageToken"].as_str().filter(|p| !p.is_empty()) {
            Some(next) => {
                if !next
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'='))
                {
                    return Err(AppError::external("Invalid Gmail pagination token"));
                }
                page = next.into();
            }
            None => break,
        }
    }
    Ok(all)
}
pub fn save(account: &Account, draft: &RemoteDraft) -> AppResult<RemoteDraft> {
    // Only this preparation phase is safe to retry: no create/update request has
    // been issued. Errors after send_json may mean Gmail accepted the write.
    let (token, payload) = prepare_save(account, draft)
        .map_err(|error| AppError::new("cloud_save_not_started", error.message))?;
    let value: Value = if draft.id.is_empty() {
        agent()
            .post(BASE)
            .header("Authorization", format!("Bearer {token}"))
            .send_json(&payload)
            .map_err(|e| {
                AppError::external(format!(
                    "Gmail draft create failed; synchronize before retrying: {e}"
                ))
            })?
            .body_mut()
            .read_json()
            .map_err(|e| AppError::external(e.to_string()))?
    } else {
        agent()
            .put(&path(&draft.id)?)
            .header("Authorization", format!("Bearer {token}"))
            .send_json(&payload)
            .map_err(|e| AppError::external(format!("Gmail draft update failed: {e}")))?
            .body_mut()
            .read_json()
            .map_err(|e| AppError::external(e.to_string()))?
    };
    let id = value["id"]
        .as_str()
        .ok_or_else(|| AppError::external("Gmail save returned no draft id"))?;
    parse(&get(&format!("{}?format=raw", path(id)?), &token)?, account)
}

fn prepare_save(account: &Account, draft: &RemoteDraft) -> AppResult<(String, Value)> {
    let token = token(account)?;
    if !draft.id.is_empty() {
        let current = get(&format!("{}?format=minimal", path(&draft.id)?), &token)?;
        if current["message"]["id"].as_str() != Some(draft.revision.as_str()) {
            return Err(AppError::new(
                "revision_conflict",
                "The Gmail draft changed remotely; synchronize before saving",
            ));
        }
    }
    let d = &draft.draft;
    let outgoing = super::mail::Outgoing {
        message_id: None,
        to: super::mail::split_addrs(&d.to),
        cc: super::mail::split_addrs(&d.cc),
        bcc: super::mail::split_addrs(&d.bcc),
        subject: d.subject.clone(),
        body: crate::richtext::convert_body(&d.body, d.format, BodyFormat::Plain),
        html: (d.format != BodyFormat::Plain)
            .then(|| crate::richtext::convert_body(&d.body, d.format, BodyFormat::Html)),
        in_reply_to: None,
        attachments: d.attachments.clone(),
    };
    let message =
        super::mail::build_message(account, "", &outgoing).map_err(AppError::validation)?;
    let mut bytes = message.formatted();
    if !d.cloud_sync_key.is_empty() {
        let mut header = format!("X-Qamille-Draft-Id: {}\r\n", d.cloud_sync_key).into_bytes();
        header.extend(bytes);
        bytes = header;
    }
    // For an unsent draft, Bcc is stored in MIME. SMTP's built message still strips it.
    if !d.bcc.is_empty() {
        let header = format!("Bcc: {}\r\n", d.bcc);
        let mut with_bcc = header.into_bytes();
        with_bcc.extend(bytes);
        bytes = with_bcc;
    }
    Ok((token, json!({"message":{"raw":encode(&bytes)}})))
}
pub fn delete(account: &Account, id: &str) -> AppResult<()> {
    let token = token(account)?;
    agent()
        .delete(&path(id)?)
        .header("Authorization", format!("Bearer {token}"))
        .call()
        .map_err(|e| AppError::external(format!("Gmail draft delete failed: {e}")))?;
    Ok(())
}

const LABELS: &str = "https://gmail.googleapis.com/gmail/v1/users/me/labels";
pub fn list_labels(account: &Account) -> AppResult<Value> {
    let auth = token(account)?;
    get(LABELS, &auth)
}
pub fn manage_label(
    account: &Account,
    action: &str,
    name: &str,
    new_name: Option<&str>,
) -> AppResult<Value> {
    if name.trim().is_empty() || name.contains(['\r', '\n']) {
        return Err(AppError::validation("A valid label name is required"));
    }
    let auth = token(account)?;
    if action == "create" {
        return agent().post(LABELS).header("Authorization",format!("Bearer {auth}")).send_json(json!({"name":name,"labelListVisibility":"labelShow","messageListVisibility":"show"})).map_err(|e|AppError::external(format!("Gmail label create failed: {e}")))?.body_mut().read_json().map_err(|e|AppError::external(e.to_string()));
    }
    let labels = get(LABELS, &auth)?;
    let label = labels["labels"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|l| l["id"] == name || l["name"] == name)
        .ok_or_else(|| AppError::missing("Provider label"))?;
    if label["type"] == "system" {
        return Err(AppError::validation(
            "System labels cannot be renamed or deleted",
        ));
    }
    let id = label["id"]
        .as_str()
        .ok_or_else(|| AppError::external("Gmail label id missing"))?;
    if !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') {
        return Err(AppError::external("Invalid label id returned by Gmail"));
    }
    let url = format!("{LABELS}/{id}");
    match action {
        "delete" => {
            agent()
                .delete(&url)
                .header("Authorization", format!("Bearer {auth}"))
                .call()
                .map_err(|e| AppError::external(format!("Gmail label delete failed: {e}")))?;
            Ok(json!({"deleted":id,"messages_deleted":false}))
        }
        "rename" => {
            let name = new_name
                .filter(|s| !s.trim().is_empty() && !s.contains(['\r', '\n']))
                .ok_or_else(|| AppError::validation("new_name is required"))?;
            agent()
                .patch(&url)
                .header("Authorization", format!("Bearer {auth}"))
                .send_json(json!({"name":name}))
                .map_err(|e| AppError::external(format!("Gmail label rename failed: {e}")))?
                .body_mut()
                .read_json()
                .map_err(|e| AppError::external(e.to_string()))
        }
        _ => Err(AppError::validation(
            "action must be create, rename or delete",
        )),
    }
}

/// Replace the cloud draft with the immutable outbox snapshot, then ask Gmail to
/// send that draft. Gmail removes the draft on acceptance; SMTP is never also used.
pub fn send_snapshot(
    account: &Account,
    name: &str,
    message: crate::application::ports::OutgoingMessage,
    id: &str,
    revision: &str,
) -> AppResult<()> {
    let auth = (|| -> AppResult<String> {
        let auth = token(account)?;
        let url = path(id)?;
        let current = get(&format!("{url}?format=minimal"), &auth)?;
        if current["message"]["id"].as_str() != Some(revision) {
            return Err(AppError::new(
                "revision_conflict",
                "The cloud draft changed after it was queued; review it before sending",
            ));
        }
        let bcc = message.bcc.join(", ");
        let outgoing = super::mail::Outgoing {
            message_id: message.message_id,
            to: message.to,
            cc: message.cc,
            bcc: message.bcc,
            subject: message.subject,
            body: message.plain,
            html: message.html,
            in_reply_to: message.in_reply_to,
            attachments: message.attachments,
        };
        let mut bytes = super::mail::build_message(account, name, &outgoing)
            .map_err(AppError::validation)?
            .formatted();
        if !bcc.is_empty() {
            let mut header = format!("Bcc: {bcc}\r\n").into_bytes();
            header.extend(bytes);
            bytes = header;
        }
        agent()
            .put(&url)
            .header("Authorization", format!("Bearer {auth}"))
            .send_json(json!({"message":{"raw":encode(&bytes)}}))
            .map_err(|e| {
                AppError::external(format!(
                    "Cloud draft preparation failed; synchronize before retrying: {e}"
                ))
            })?;
        Ok(auth)
    })().map_err(|error| AppError::delivery_not_started(error.message))?;
    agent()
        .post(format!("{BASE}/send"))
        .header("Authorization", format!("Bearer {auth}"))
        .send_json(json!({"id":id}))
        .map_err(|e| AppError::external(format!("Cloud delivery result is uncertain: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn cloud_preparation_errors_are_explicitly_safe_to_retry() {
        let account = crate::model::Account::default();
        let draft = crate::domain::outbox::RemoteDraft { id: String::new(), revision: String::new(), raw: String::new(), thread_id: None, draft: crate::model::Draft::default() };
        let error = super::save(&account, &draft).unwrap_err();
        assert_eq!(error.code, "cloud_save_not_started");
        assert!(error.message.contains("Reconnect"));
    }
    #[test]
    fn base64url_roundtrip() {
        for length in 0..256 {
            let bytes: Vec<_> = (0..length).map(|x| x as u8).collect();
            let encoded = super::encode(&bytes);
            assert!(!encoded.contains('='));
            assert_eq!(super::decode(&encoded).unwrap(), bytes);
        }
    }
}
