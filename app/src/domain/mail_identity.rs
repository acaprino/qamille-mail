//! A header Message-ID is not proof that two provider messages are identical.
use super::model::{Email, RemoteLocation};

pub fn same_folder(left: &str, right: &str) -> bool {
    left == right || (left.eq_ignore_ascii_case("INBOX") && right.eq_ignore_ascii_case("INBOX"))
}

fn metadata_matches(left: &Email, right: &Email) -> bool {
    left.from_addr == right.from_addr
        && left.subject == right.subject
        && left.date == right.date
        && left.message_id == right.message_id
}

fn compatible_hashes(left: &Email, right: &Email) -> bool {
    left.remote_content_hash.is_empty()
        || right.remote_content_hash.is_empty()
        || left.remote_content_hash == right.remote_content_hash
}

fn verified_location(location: &RemoteLocation, email: &Email) -> bool {
    Some(location.uid) == email.uid
        && same_folder(
            &location.folder,
            email.remote_folder.as_deref().unwrap_or("INBOX"),
        )
        && location.uid_validity.is_some()
        && location.uid_validity == email.remote_uid_validity
}

/// The same IMAP location may be matched while upgrading legacy records that
/// lack UIDVALIDITY. Reusing cached payload still requires the provider to verify
/// that generation and content. Across folders, only a verified alias or a full
/// content hash is accepted; identical RFC headers alone never merge messages.
pub fn same_message(left: &Email, right: &Email) -> bool {
    if left.account_id != right.account_id || !compatible_hashes(left, right) {
        return false;
    }
    if let (Some(left), Some(right)) = (left.remote_global_id, right.remote_global_id) { return left == right; }
    if (left.local_sent || right.local_sent)
        && !left.outgoing_fingerprint.is_empty()
        && left.outgoing_fingerprint == right.outgoing_fingerprint
        && !left.message_id.is_empty()
        && left.message_id.trim_matches(['<', '>']) == right.message_id.trim_matches(['<', '>'])
    { return true; }
    let same_canonical_folder = same_folder(
        left.remote_folder.as_deref().unwrap_or("INBOX"),
        right.remote_folder.as_deref().unwrap_or("INBOX"),
    );
    if same_canonical_folder {
        if let (Some(left_uid), Some(right_uid)) = (left.uid, right.uid) {
            if left_uid != right_uid {
                return false;
            }
            if matches!((left.remote_uid_validity, right.remote_uid_validity), (Some(a), Some(b)) if a != b)
            {
                return false;
            }
            return metadata_matches(left, right);
        }
        // Without a locator, matching headers or copied text are not enough to
        // collapse two same-folder messages that might be distinct deliveries.
        return false;
    }
    if metadata_matches(left, right)
        && (left
            .remote_locations
            .iter()
            .any(|location| verified_location(location, right))
            || right
                .remote_locations
                .iter()
                .any(|location| verified_location(location, left)))
    {
        return true;
    }
    !left.remote_content_hash.is_empty() && left.remote_content_hash == right.remote_content_hash
}

/// Stable across provider-added MIME headers and transfer encodings. Callers
/// supply attachment digests from actual bytes, never untrusted MIME metadata.
pub fn outgoing_fingerprint(email: &Email, attachments: &[(String, String)]) -> String {
    use sha2::{Digest, Sha256};
    // Use the same mailbox grammar as the MIME builder. Display names are not
    // envelope identity, and DNS domain names are case-insensitive; retain the
    // local part's case rather than conflating distinct case-sensitive users.
    let mailbox = |value: &str| value.parse::<lettre::message::Mailbox>()
        .map(|mailbox| format!("{}@{}", mailbox.email.user(), mailbox.email.domain().to_ascii_lowercase()))
        .unwrap_or_else(|_| value.to_string());
    let recipients = |values: &[String]| values.iter().map(|value| mailbox(value)).collect::<Vec<_>>();
    let normalize = |s: &str| s.replace("\r\n", "\n").trim_end().to_string();
    let bytes = serde_json::to_vec(&(
        mailbox(&email.from_addr), recipients(&email.to), recipients(&email.cc), &email.subject,
        normalize(&email.body), attachments,
    )).expect("Outgoing fingerprint contains only serializable strings");
    format!("{:x}", Sha256::digest(bytes))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn email() -> Email {
        Email {
            account_id: 1,
            uid: Some(7),
            remote_folder: Some("INBOX".into()),
            remote_uid_validity: Some(9),
            from_addr: "sender@example.com".into(),
            subject: "same subject".into(),
            date: 100,
            message_id: "same-header@example.com".into(),
            ..Default::default()
        }
    }
    #[test]
    fn identical_headers_in_distinct_deliveries_never_merge() {
        let left = email();
        let mut right = left.clone();
        right.uid = Some(8);
        assert!(!same_message(&left, &right));
        let mut left = left;
        left.remote_content_hash = "verified-hash".into();
        right.remote_content_hash = left.remote_content_hash.clone();
        assert!(!same_message(&left, &right));
    }
    #[test]
    fn distinct_provider_ids_override_equal_copied_mime() {
        let mut left = email();
        left.remote_global_id = Some(1);
        left.remote_content_hash = "identical-mime".into();
        let mut right = left.clone();
        right.remote_global_id = Some(2);
        right.remote_folder = Some("Archive".into());
        right.uid = Some(20);
        assert!(!same_message(&left, &right));
    }
    #[test]
    fn outgoing_fingerprint_normalizes_display_names_and_domain_case_only() {
        let mut sent = email();
        sent.to = vec!["Recipient Name <Recipient@EXAMPLE.COM>".into()];
        sent.cc = vec!["Other Name <other@EXAMPLE.COM>".into()];
        let mut echo = sent.clone();
        echo.to = vec!["Recipient@example.com".into()];
        echo.cc = vec!["other@example.com".into()];
        assert_eq!(outgoing_fingerprint(&sent, &[]), outgoing_fingerprint(&echo, &[]));
        echo.to = vec!["different@example.com".into()];
        assert_ne!(outgoing_fingerprint(&sent, &[]), outgoing_fingerprint(&echo, &[]));
    }
    #[test]
    fn uid_namespace_account_and_metadata_must_match() {
        let left = email();
        let mut right = left.clone();
        assert!(same_message(&left, &right));
        right.account_id = 2;
        assert!(!same_message(&left, &right));
        right.account_id = 1;
        right.remote_uid_validity = Some(10);
        assert!(!same_message(&left, &right));
        right.remote_uid_validity = None;
        assert!(same_message(&left, &right));
        right.subject = "changed".into();
        assert!(!same_message(&left, &right));
    }
    #[test]
    fn cross_folder_match_requires_full_hash_or_verified_alias() {
        let mut left = email();
        let mut right = left.clone();
        right.remote_folder = Some("All Mail".into());
        right.uid = Some(20);
        right.remote_uid_validity = Some(30);
        assert!(!same_message(&left, &right));
        left.remote_content_hash = "full-content".into();
        right.remote_content_hash = "full-content".into();
        assert!(same_message(&left, &right));
        right.remote_content_hash.clear();
        left.remote_locations.push(RemoteLocation {
            folder: "All Mail".into(),
            uid: 20,
            uid_validity: Some(30),
        });
        assert!(same_message(&left, &right));
        assert!(same_message(&right, &left));
        right.remote_uid_validity = Some(31);
        assert!(!same_message(&left, &right));
    }
    #[test]
    fn conflict_or_unverified_alias_is_not_identity_proof() {
        let mut left = email();
        let mut right = left.clone();
        left.remote_content_hash = "old".into();
        right.remote_content_hash = "new".into();
        assert!(!same_message(&left, &right));
        right.remote_content_hash.clear();
        right.remote_folder = Some("Archive".into());
        left.remote_locations.push(RemoteLocation {
            folder: "Archive".into(),
            uid: 7,
            uid_validity: None,
        });
        right.remote_uid_validity = None;
        assert!(!same_message(&left, &right));
        assert!(same_folder("inbox", "INBOX"));
        assert!(!same_folder("Project", "project"));
    }
}

/// Enriches a known row with what a provider read says about the same message
/// (indexing, server search): identity, location, missing labels and dates.
/// Local state (folder, read, star, labels, tasks, reminders) stays as it is.
pub fn merge_remote(existing: &mut Email, incoming: &Email) {
    if !incoming.reply_to_addresses.is_empty() {
        existing.reply_to_addresses.clone_from(&incoming.reply_to_addresses);
    }
    if incoming.remote_global_id.is_some() {
        existing.remote_global_id = incoming.remote_global_id;
    }
    if let Some(uid) = incoming.uid {
        let location = crate::model::RemoteLocation {
            folder: incoming.remote_folder.clone().unwrap_or_else(|| "INBOX".into()),
            uid,
            uid_validity: incoming.remote_uid_validity,
        };
        let primary = existing.uid == Some(uid)
            && existing.remote_folder.as_deref().unwrap_or("INBOX") == location.folder
            && existing.remote_uid_validity == location.uid_validity;
        if existing.uid.is_none() {
            existing.uid = Some(uid);
            existing.remote_folder = Some(location.folder.clone());
            existing.remote_uid_validity = location.uid_validity;
        } else if !primary && !existing.remote_locations.contains(&location) {
            existing.remote_locations.push(location);
        }
    }
    // Read state and labels changed here may still be on their way to the
    // server: a provider read only fills labels a row has never had.
    if existing.labels.is_empty() {
        existing.labels = incoming.labels.clone();
    }
    if existing.date_missing && !incoming.date_missing {
        existing.date = incoming.date;
        existing.date_missing = false;
    }
    if existing.remote_only && existing.attachments.is_empty() && !incoming.attachments.is_empty() {
        existing.attachments = incoming
            .attachments
            .iter()
            .enumerate()
            .map(|(index, part)| crate::model::Attachment {
                id: existing.id.saturating_mul(1000).saturating_add(index as u64),
                path: None,
                inline_text: None,
                ..part.clone()
            })
            .collect();
    }
}

/// Inserts or enriches metadata-only rows of one account in `store`: a known
/// message (by `same_message`) keeps its local state and gains identity and
/// location; a new one gets an id and no content. Returns (inserted, enriched).
pub fn merge_rows(store: &mut crate::model::Store, account_id: u32, rows: &[Email]) -> (Vec<u64>, Vec<u64>) {
    let (mut inserted, mut enriched) = (Vec::new(), Vec::new());
    // Rows of the account by Gmail identity: a batch of 500 does not scan the store 500 times.
    // A row without one, or not found by it, falls back to the full comparison.
    let mut by_global: std::collections::HashMap<u64, Vec<usize>> = std::collections::HashMap::new();
    let mut without_global = Vec::new();
    for (index, email) in store.emails.iter().enumerate() {
        if email.account_id == account_id {
            match email.remote_global_id {
                Some(global) => by_global.entry(global).or_default().push(index),
                None => without_global.push(index),
            }
        }
    }
    for row in rows {
        let mut row = row.clone();
        row.account_id = account_id;
        let indexed = row.remote_global_id.and_then(|global| by_global.get(&global)).and_then(|candidates| {
            candidates.iter().copied().find(|&index| same_message(&store.emails[index], &row))
        });
        // Two Gmail identities decide by themselves (`same_message`): only rows lacking one
        // on either side need the full comparison.
        let found = indexed.or_else(|| match row.remote_global_id {
            Some(_) => without_global.iter().copied().find(|&index| same_message(&store.emails[index], &row)),
            None => store.emails.iter().position(|old| old.account_id == account_id && same_message(old, &row)),
        });
        if let Some(index) = found {
            let existing = &mut store.emails[index];
            merge_remote(existing, &row);
            enriched.push(existing.id);
            continue;
        }
        row.id = store.new_id();
        for (index, attachment) in row.attachments.iter_mut().enumerate() {
            attachment.id = row.id.saturating_mul(1000).saturating_add(index as u64);
            attachment.path = None;
            attachment.inline_text = None;
        }
        row.body.clear();
        row.body_html.clear();
        row.remote_only = true;
        inserted.push(row.id);
        match row.remote_global_id {
            Some(global) => by_global.entry(global).or_default().push(store.emails.len()),
            None => without_global.push(store.emails.len()),
        }
        store.emails.push(row);
    }
    (inserted, enriched)
}
