use super::mail;
use super::mail::extension::GmailExtensions;
use crate::application::ports::{PolicySyncBatch, SyncBatch, SyncRequest};
use crate::model::{Account, Email};
use std::{
    collections::HashSet,
    io::{Read, Write},
    path::Path,
};

// Bound each background pass independently from the account's disk quota.
// Later passes reuse confirmed content and continue with the missing messages.
const OFFLINE_PASS_BYTES: u64 = 64 * 1024 * 1024;

pub fn sync_account_policy(
    account: &Account,
    directory: &Path,
    request: &SyncRequest<'_>,
) -> Result<PolicySyncBatch, String> {
    let mut session = mail::connect(account)?;
    let extensions = session.extensions.clone();
    let gmail = session.capabilities().map_err(|e| e.to_string())?.has_str("X-GM-EXT-1");
    let result = fetch_policy_extended(&mut session, account, directory, request, gmail.then_some(&extensions as &dyn GmailExtensions));
    let _ = session.logout();
    result
}

#[derive(Clone)]
struct Folder {
    remote: String,
    logical: String,
}

fn folder_plan<T: Read + Write>(
    session: &mut imap::Session<T>,
    account: &Account,
) -> Result<(Vec<Folder>, Option<String>, Option<String>, Option<String>), String> {
    let mut folders = vec![Folder {
        remote: "INBOX".into(),
        logical: "Inbox".into(),
    }];
    let mut archive = None;
    let mut trash = None;
    let names = session
        .list(Some(""), Some("*"))
        .map_err(|e| e.to_string())?;
    let mut roles = HashSet::new();
    for name in names.iter() {
        for attr in name.attributes() {
            let imap::types::NameAttribute::Custom(role) = attr else {
                continue;
            };
            let logical = match role.as_ref() {
                "\\All" => {
                    archive = Some(name.name().to_owned());
                    "Archive"
                }
                "\\Sent" => "Sent",
                "\\Trash" => {
                    trash = Some(name.name().to_owned());
                    "Trash"
                }
                "\\Junk" => "Spam",
                "\\Drafts" => "Drafts",
                _ => continue,
            };
            roles.insert(role.as_ref().to_owned());
            if account.storage_policy.keep_all_offline
                && !folders.iter().any(|folder| folder.remote == name.name())
            {
                folders.push(Folder {
                    remote: name.name().into(),
                    logical: logical.into(),
                });
            }
        }
    }
    // Keep Inbox/Sent membership when the same message also occurs in All Mail.
    folders.sort_by_key(|folder| folder.logical == "Archive");
    let missing: Vec<_> = ["\\All", "\\Sent", "\\Trash", "\\Junk", "\\Drafts"]
        .into_iter()
        .filter(|role| !roles.contains(*role))
        .collect();
    let notice = (account.storage_policy.keep_all_offline && !missing.is_empty()).then(|| format!(
        "Offline copy is partial: Gmail folders {} are not exposed over IMAP. Enable them in Gmail and sync again.",
        missing.join(", ")
    ));
    Ok((folders, archive, trash, notice))
}

fn cached_content_ready(email: &Email) -> bool {
    !email.remote_only
        && email.attachments.iter().all(|attachment| {
            attachment.path.as_ref().is_some_and(|path| {
                std::fs::metadata(path)
                    .is_ok_and(|metadata| metadata.is_file() && metadata.len() == attachment.size)
            })
        })
}

pub(super) fn attachment_metadata(
    structure: &imap_proto::BodyStructure<'_>,
    output: &mut Vec<crate::model::Attachment>,
) {
    use imap_proto::BodyStructure;
    let (common, other) = match structure {
        BodyStructure::Multipart { bodies, .. } => {
            for body in bodies {
                attachment_metadata(body, output);
            }
            return;
        }
        BodyStructure::Basic { common, other, .. }
        | BodyStructure::Text { common, other, .. }
        | BodyStructure::Message { common, other, .. } => (common, other),
    };
    let filename = common
        .disposition
        .as_ref()
        .and_then(|disposition| disposition.params.as_ref())
        .and_then(|parameters| {
            parameters
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case("filename"))
        })
        .map(|(_, value)| *value)
        .or_else(|| {
            common.ty.params.as_ref().and_then(|parameters| {
                parameters
                    .iter()
                    .find(|(name, _)| name.eq_ignore_ascii_case("name"))
                    .map(|(_, value)| *value)
            })
        });
    let attached = filename.is_some()
        || common
            .disposition
            .as_ref()
            .is_some_and(|disposition| disposition.ty.eq_ignore_ascii_case("attachment"));
    if attached {
        output.push(crate::model::Attachment {
            id: output.len() as u64,
            name: filename.unwrap_or("attachment").into(),
            mime: format!(
                "{}/{}",
                common.ty.ty.to_lowercase(),
                common.ty.subtype.to_lowercase()
            ),
            size: u64::from(other.octets),
            path: None,
            inline_text: None,
        });
    }
}

pub(super) fn assign_attachment_ids(email: &Email, attachments: &mut [crate::model::Attachment]) {
    let mut used = std::collections::HashSet::new();
    let mut reserved: std::collections::HashSet<_> = email.attachments.iter().map(|a| a.id).collect();
    let mut next = email.id.saturating_mul(1000);
    for attachment in attachments {
        if let Some((index, known)) = email.attachments.iter().enumerate().find(|(index, known)| {
            !used.contains(index) && known.name == attachment.name && known.mime.eq_ignore_ascii_case(&attachment.mime)
        }) {
            used.insert(index);
            attachment.id = known.id;
        } else {
            while reserved.contains(&next) { next = next.saturating_add(1); }
            attachment.id = next;
            reserved.insert(next);
            next = next.saturating_add(1);
        }
    }
}

fn same_location(left: &Email, right: &Email) -> bool {
    left.account_id == right.account_id
        && left.uid.is_some()
        && left.uid == right.uid
        && crate::domain::mail_identity::same_folder(
            left.remote_folder.as_deref().unwrap_or("INBOX"),
            right.remote_folder.as_deref().unwrap_or("INBOX"),
        )
        && left.from_addr == right.from_addr
        && left.subject == right.subject
        && left.date == right.date
}

fn confirmed_location(known: &Email, incoming: &Email) -> bool {
    if known.remote_content_hash.is_empty() || incoming.remote_uid_validity.is_none() {
        return false;
    }
    (same_location(known, incoming) && known.remote_uid_validity == incoming.remote_uid_validity)
        || known.remote_locations.iter().any(|location| {
            Some(location.uid) == incoming.uid
                && location.uid_validity == incoming.remote_uid_validity
                && crate::domain::mail_identity::same_folder(
                    &location.folder,
                    incoming.remote_folder.as_deref().unwrap_or("INBOX"),
                )
        })
}

fn add_location(target: &mut Email, source: &Email) {
    let Some(uid) = source.uid else { return };
    let folder = source.remote_folder.as_deref().unwrap_or("INBOX");
    if !target.remote_locations.iter().any(|location| {
        location.uid == uid
            && location.uid_validity == source.remote_uid_validity
            && crate::domain::mail_identity::same_folder(&location.folder, folder)
    }) {
        target.remote_locations.push(crate::model::RemoteLocation {
            folder: folder.into(),
            uid,
            uid_validity: source.remote_uid_validity,
        });
    }
    for location in &source.remote_locations {
        if !target.remote_locations.contains(location) {
            target.remote_locations.push(location.clone());
        }
    }
}

#[cfg(test)]
fn fetch_policy<T: Read + Write>(
    session: &mut imap::Session<T>,
    account: &Account,
    directory: &Path,
    request: &SyncRequest<'_>,
) -> Result<PolicySyncBatch, String> {
    fetch_policy_extended(session, account, directory, request, None)
}

fn fetch_policy_extended<T: Read + Write>(
    session: &mut imap::Session<T>, account: &Account, directory: &Path,
    request: &SyncRequest<'_>, extensions: Option<&dyn GmailExtensions>,
) -> Result<PolicySyncBatch, String> {
    let offline = account.storage_policy.keep_all_offline;
    let (folders, archive_folder, trash_folder, mut notice) = folder_plan(session, account)?;
    let mut complete = offline && notice.is_none();
    let mut remaining = request.max_bytes.unwrap_or(u64::MAX);
    let mut pass_remaining = OFFLINE_PASS_BYTES;
    let mut output = Vec::new();
    let mut seen = HashSet::new();
    let mut inbox_membership = None;
    let progress = (|| -> Result<(), String> {
        for folder in folders {
            let mailbox = session.examine(&folder.remote).map_err(|e| e.to_string())?;
            if offline && mailbox.uid_validity.is_none() {
                complete = false;
                notice.get_or_insert_with(|| "Offline copy could not verify the mailbox identity. Sync again before relying on complete offline coverage.".into());
            }
            let mut ids: Vec<_> = session
                .uid_search("ALL")
                .map_err(|e| e.to_string())?
                .into_iter()
                .collect();
            ids.sort_unstable_by_key(|uid| std::cmp::Reverse(*uid));
            if folder.logical == "Inbox" { inbox_membership = Some((mailbox.uid_validity, ids.iter().copied().collect::<HashSet<_>>())); }
            if !offline {
                ids.truncate(account.fetch_count.clamp(10, 500) as usize);
            }
            for batch in ids.chunks(100) {
                let set = batch
                    .iter()
                    .map(u32::to_string)
                    .collect::<Vec<_>>()
                    .join(",");
                let global_ids = extensions.map(|extension| extension.ids(&set)).transpose()?.unwrap_or_default();
                let metadata = session
                    .uid_fetch(
                        set,
                        "(UID FLAGS RFC822.SIZE INTERNALDATE BODYSTRUCTURE BODY.PEEK[HEADER])",
                    )
                    .map_err(|e| e.to_string())?;
                if metadata.len() != batch.len() {
                    complete = false;
                    notice.get_or_insert_with(|| "The mailbox changed during sync. Sync again to finish the offline copy.".into());
                }
                let mut rows: Vec<_> = metadata.iter().collect();
                rows.sort_unstable_by_key(|row| std::cmp::Reverse(row.uid));
                for row in rows {
                    let parsed = row.uid.zip(row.header()).and_then(|(uid, header)| {
                        mail::parse_message_inner(header, 0, account.id, None).map(|email| (uid, email))
                    });
                    let Some((uid, mut email)) = parsed else {
                        complete = false;
                        notice.get_or_insert_with(|| "Gmail did not provide readable headers for some messages. Sync again to finish the offline copy.".into());
                        continue;
                    };
                    email.remote_only = true;
                    email.uid = Some(uid);
                    email.remote_folder = Some(folder.remote.clone());
                    email.remote_uid_validity = mailbox.uid_validity;
                    email.remote_global_id = global_ids.get(&uid).copied();
                    if extensions.is_some() && email.remote_global_id.is_none() { return Err("Mailbox changed while verifying Gmail identities; sync again".into()); }
                    email.folder = folder.logical.clone();
                    if let Some(structure) = row.bodystructure() {
                        attachment_metadata(structure, &mut email.attachments);
                    }
                    if email.date_missing {
                        if let Some(date) = row.internal_date() {
                            email.date = date.timestamp();
                            email.date_missing = false;
                        }
                    }
                    // When Gmail received it, whatever its Date header says: whether it is new mail.
                    email.received_at = row.internal_date().map(|date| date.timestamp());
                    let key = format!(
                        "location:{}:{:?}:{uid}",
                        folder.remote, mailbox.uid_validity
                    );
                    if !seen.insert(key) {
                        continue;
                    }
                    let known = request
                        .known
                        .iter()
                        .find(|old| crate::domain::mail_identity::same_message(old, &email));
                    let possible_label_copy = !email.message_id.is_empty()
                        && request.known.iter().chain(output.iter()).any(|old| {
                            old.account_id == account.id
                                && old.message_id == email.message_id
                                && !old.remote_content_hash.is_empty()
                        });
                    if let Some(old) = known {
                        // An online account may keep an existing local copy until its
                        // own quota needs space; a metadata sync does not discard it.
                        email.attachments = old.attachments.clone();
                        email.remote_content_hash = old.remote_content_hash.clone();
                        email.remote_locations = old.remote_locations.clone();
                        if !old.remote_only {
                            email.body = old.body.clone();
                            email.remote_only = false;
                        }
                    }
                    let mut missing_content = offline
                        && known.is_none_or(|old| {
                            !cached_content_ready(old) || !confirmed_location(old, &email)
                        });
                    let new_metadata = if known.is_none() {
                        serde_json::to_vec_pretty(&email)
                            .map_err(|e| e.to_string())?
                            .len() as u64
                            + 64
                    } else {
                        0
                    };
                    if new_metadata > remaining {
                        complete = false;
                        notice.get_or_insert_with(|| "Offline copy is partial: the account's local storage limit has been reached.".into());
                        // A full account must not grow an unbounded metadata list while
                        // its protected contents already occupy all available space.
                        if offline && !possible_label_copy {
                            continue;
                        }
                    } else {
                        remaining = remaining.saturating_sub(new_metadata);
                    }
                    if missing_content {
                        let estimated = u64::from(row.size.unwrap_or(u32::MAX));
                        if (estimated <= remaining || possible_label_copy)
                            && estimated <= pass_remaining
                        {
                            // A label copy can be verified in RAM even when its bytes
                            // already fill the account quota; no second file is needed.
                            match super::storage_fs::reserve(
                                directory,
                                if possible_label_copy { 0 } else { estimated },
                            ) {
                                Err(error) => {
                                    notice.get_or_insert(error.message);
                                }
                                Ok(_reservation) => {
                                    let fetched = session
                                        .uid_fetch(uid.to_string(), "BODY.PEEK[]")
                                        .map_err(|e| e.to_string())?;
                                    let raw = fetched
                                        .iter()
                                        .find(|message| message.uid == Some(uid))
                                        .and_then(|message| message.body())
                                        .ok_or("Gmail did not return a complete message")?;
                                    // Validate actual size before any managed file write.
                                    let mut preview =
                                        mail::parse_message_inner(raw, 0, account.id, None)
                                            .ok_or("Gmail message could not be read")?;
                                    use sha2::{Digest, Sha256};
                                    preview.remote_content_hash =
                                        format!("{:x}", Sha256::digest(raw));
                                    preview.uid = email.uid;
                                    preview.remote_folder = email.remote_folder.clone();
                                    preview.remote_uid_validity = email.remote_uid_validity;
                                    preview.remote_global_id = email.remote_global_id;
                                    preview.folder = email.folder.clone();
                                    if preview.date_missing {
                                        preview.date = email.date;
                                        preview.date_missing = email.date_missing;
                                    }
                                    if let Some(previous) = output.iter_mut().find(|old| {
                                        old.remote_content_hash == preview.remote_content_hash
                                            && crate::domain::mail_identity::same_message(
                                                old, &preview,
                                            )
                                    }) {
                                        add_location(previous, &preview);
                                        pass_remaining =
                                            pass_remaining.saturating_sub(raw.len() as u64);
                                        continue;
                                    }
                                    let verified_copy = request.known.iter().find(|old| {
                                        old.remote_content_hash == preview.remote_content_hash
                                            && crate::domain::mail_identity::same_message(
                                                old, &preview,
                                            )
                                            && cached_content_ready(old)
                                    });
                                    if let Some(previous) = verified_copy {
                                        preview.body = previous.body.clone();
                                        preview.attachments = previous.attachments.clone();
                                        preview.remote_locations =
                                            previous.remote_locations.clone();
                                        add_location(&mut preview, previous);
                                        email = preview;
                                        missing_content = false;
                                        pass_remaining =
                                            pass_remaining.saturating_sub(raw.len() as u64);
                                    } else {
                                        let serialized = serde_json::to_vec_pretty(&preview)
                                            .map_err(|e| e.to_string())?
                                            .len()
                                            as u64;
                                        let path_allowance =
                                            directory.to_string_lossy().len() as u64 + 160;
                                        let growth = preview.attachments.iter().fold(
                                            serialized + 64,
                                            |size, attachment| {
                                                size.saturating_add(attachment.size).saturating_add(
                                                    path_allowance
                                                        + attachment.name.len() as u64 * 6,
                                                )
                                            },
                                        );
                                        if growth <= remaining && raw.len() as u64 <= pass_remaining
                                        {
                                            let mut loaded =
                                                mail::parse_message(raw, 0, account.id, directory)
                                                    .ok_or("Gmail message could not be read")?;
                                            loaded.uid = email.uid;
                                            loaded.remote_folder = email.remote_folder.clone();
                                            loaded.remote_uid_validity = email.remote_uid_validity;
                                            loaded.remote_global_id = email.remote_global_id;
                                            loaded.remote_content_hash =
                                                preview.remote_content_hash;
                                            loaded.folder = email.folder.clone();
                                            if loaded.date_missing {
                                                loaded.date = email.date;
                                                loaded.date_missing = email.date_missing;
                                            }
                                            missing_content = !cached_content_ready(&loaded);
                                            remaining = remaining.saturating_sub(growth);
                                            pass_remaining =
                                                pass_remaining.saturating_sub(raw.len() as u64);
                                            email = loaded;
                                        }
                                    }
                                }
                            }
                        }
                        if missing_content {
                            complete = false;
                            notice.get_or_insert_with(|| if estimated > OFFLINE_PASS_BYTES {
                            "Offline copy is partial: a message exceeds the 64 MiB download safety limit. Its content remains available online.".into()
                        } else if estimated > remaining {
                            "Offline copy is partial: increase this account's storage limit to continue.".into()
                        } else {
                            "Offline copy is still being prepared. The next sync continues downloading missing content.".into()
                        });
                        }
                    }
                    email.read = row
                        .flags()
                        .iter()
                        .any(|flag| matches!(flag, imap::types::Flag::Seen));
                    email.starred = row
                        .flags()
                        .iter()
                        .any(|flag| matches!(flag, imap::types::Flag::Flagged));
                    email.replied = row
                        .flags()
                        .iter()
                        .any(|flag| matches!(flag, imap::types::Flag::Answered));
                    if let Some(previous) = output
                        .iter_mut()
                        .find(|old| crate::domain::mail_identity::same_message(old, &email))
                    {
                        add_location(previous, &email);
                        continue;
                    }
                    output.push(email);
                }
            }
        }
        if let Some(extensions) = extensions {
            reconcile_known(session, extensions, account, request.known, inbox_membership.as_ref(), &mut output)?;
        }
        Ok(())
    })();
    if let Err(error) = progress {
        if output.is_empty() {
            return Err(error);
        }
        complete = false;
        notice = Some(format!(
            "Sync stopped before completion: {error}. Confirmed downloads are retained; sync again to continue."
        ));
    }
    Ok(PolicySyncBatch {
        batch: SyncBatch {
            emails: output,
            archive_folder,
            trash_folder,
        },
        offline_complete: complete,
        notice,
    })
}

/// Resolve only known records absent from the bounded response. Full INBOX
/// membership distinguishes an old message outside the window from a move.
fn reconcile_known<T: Read + Write>(
    session: &mut imap::Session<T>, extensions: &dyn GmailExtensions,
    account: &Account, known: &[Email], inbox: Option<&(Option<u32>, HashSet<u32>)>,
    output: &mut Vec<Email>,
) -> Result<(), String> {
    let candidates: Vec<_> = known.iter().filter(|old| old.account_id == account.id).filter(|old| {
        if old.local_sent { return true; }
        if old.folder != "Inbox" || old.remote_global_id.is_none()
            || output.iter().any(|incoming| crate::domain::mail_identity::same_message(old, incoming)) { return false; }
        !inbox.is_some_and(|(generation, ids)| generation.is_some() && *generation == old.remote_uid_validity
            && old.remote_folder.as_deref().is_some_and(|folder| folder.eq_ignore_ascii_case("INBOX"))
            && old.uid.is_some_and(|uid| ids.contains(&uid)))
    }).collect();
    if candidates.is_empty() { return Ok(()); }
    let mut all = account.clone();
    all.storage_policy.keep_all_offline = true;
    let (mut folders, _, _, _) = folder_plan(session, &all)?;
    folders.sort_by_key(|folder| match folder.logical.as_str() {
        "Inbox" => 0, "Trash" => 1, "Spam" => 2, "Sent" => 3, "Archive" => 5, _ => 4,
    });
    let complete_membership = ["Archive", "Trash", "Spam"].iter()
        .all(|role| folders.iter().any(|folder| folder.logical == *role));
    for old in candidates {
        if old.local_sent {
            let Some(folder) = folders.iter().find(|folder| folder.logical == "Sent") else { continue };
            let mailbox = session.examine(&folder.remote).map_err(|e| e.to_string())?;
            let quoted = mail::imap_quote(&format!("<{}>", old.message_id.trim_matches(['<', '>'])))?;
            let ids = session.uid_search(format!("HEADER Message-ID {quoted}")).map_err(|e| e.to_string())?;
            // Header equality is only a candidate lookup. Multiple physical
            // messages are ambiguous even if their contents happen to match.
            if ids.len() != 1 { continue; }
            let uid = *ids.iter().next().unwrap();
            let size = session.uid_fetch(uid.to_string(), "(UID RFC822.SIZE)").map_err(|e| e.to_string())?;
            if size.iter().find(|row| row.uid == Some(uid)).and_then(|row| row.size)
                .is_none_or(|size| u64::from(size) > OFFLINE_PASS_BYTES) { continue; }
            let fetched = session.uid_fetch(uid.to_string(), "(UID FLAGS BODY.PEEK[])").map_err(|e| e.to_string())?;
            let Some(row) = fetched.iter().find(|row| row.uid == Some(uid)) else { continue };
            let Some(raw) = row.body().filter(|raw| raw.len() as u64 <= OFFLINE_PASS_BYTES) else { continue };
            let Some(mut incoming) = verified_sent_echo(old, raw) else { continue };
            let Some(global) = extensions.ids(&uid.to_string())?.remove(&uid) else { continue };
            incoming.remote_global_id = Some(global);
            incoming.uid = Some(uid);
            incoming.remote_uid_validity = mailbox.uid_validity;
            incoming.remote_folder = Some(folder.remote.clone());
            incoming.folder = "Sent".into();
            incoming.local_sent = false;
            incoming.keep_parent_of(old);
            incoming.read = row.flags().iter().any(|f| matches!(f, imap::types::Flag::Seen));
            incoming.starred = row.flags().iter().any(|f| matches!(f, imap::types::Flag::Flagged));
            incoming.replied = row.flags().iter().any(|f| matches!(f, imap::types::Flag::Answered));
            // Preserve the immutable local snapshot, avoiding a second disk copy.
            if cached_content_ready(old) {
                incoming.attachments = old.attachments.clone();
            } else {
                incoming.remote_only = true;
                incoming.body.clear();
            }
            if let Some(existing) = output.iter_mut().find(|item| item.remote_global_id == Some(global)) {
                *existing = incoming;
            } else { output.push(incoming); }
            continue;
        }
        let global = old.remote_global_id.expect("candidate has provider identity");
        let mut found = false;
        for folder in &folders {
            let mailbox = session.examine(&folder.remote).map_err(|e| e.to_string())?;
            let ids = session.uid_search(format!("X-GM-MSGID {global}")).map_err(|e| e.to_string())?;
            if ids.len() > 1 { return Err("Provider returned ambiguous message locations".into()); }
            let Some(uid) = ids.into_iter().next() else { continue };
            let fetched = session.uid_fetch(uid.to_string(), "(UID FLAGS)").map_err(|e| e.to_string())?;
            let Some(row) = fetched.iter().find(|row| row.uid == Some(uid)) else { return Err("Mailbox changed during membership reconciliation".into()); };
            let mut incoming = old.clone();
            incoming.folder = folder.logical.clone();
            incoming.uid = Some(uid);
            incoming.remote_folder = Some(folder.remote.clone());
            incoming.remote_uid_validity = mailbox.uid_validity;
            incoming.read = row.flags().iter().any(|f| matches!(f, imap::types::Flag::Seen));
            incoming.starred = row.flags().iter().any(|f| matches!(f, imap::types::Flag::Flagged));
            incoming.replied = row.flags().iter().any(|f| matches!(f, imap::types::Flag::Answered));
            output.push(incoming);
            found = true;
            break;
        }
        if !found && complete_membership {
            let mut tombstone = old.clone();
            tombstone.folder = "Trash".into();
            tombstone.remote_deleted = true;
            tombstone.uid = None;
            tombstone.remote_folder = None;
            tombstone.remote_uid_validity = None;
            tombstone.remote_locations.clear();
            output.push(tombstone);
        }
    }
    Ok(())
}

fn verified_sent_echo(old: &Email, raw: &[u8]) -> Option<Email> {
    use mail_parser::MimeHeaders;
    use sha2::{Digest, Sha256};
    if !old.local_sent || old.outgoing_fingerprint.is_empty() { return None; }
    let mut incoming = mail::parse_message_inner(raw, old.id, old.account_id, None)?;
    if incoming.message_id.trim_matches(['<', '>']) != old.message_id.trim_matches(['<', '>']) { return None; }
    let parsed = mail_parser::MessageParser::default().parse(raw)?;
    let digests = parsed.attachments().map(|part| (part.attachment_name().unwrap_or("attachment").to_string(), format!("{:x}", Sha256::digest(part.contents())))).collect::<Vec<_>>();
    let fingerprint = crate::domain::mail_identity::outgoing_fingerprint(&incoming, &digests);
    if fingerprint != old.outgoing_fingerprint { return None; }
    incoming.outgoing_fingerprint = fingerprint;
    incoming.remote_content_hash = format!("{:x}", Sha256::digest(raw));
    Some(incoming)
}

/// Online-only reads stay in memory and never enlarge the offline cache.
pub fn download_body(account: &Account, email: &Email) -> Result<Email, String> {
    download_content(account, email, true)
}

/// The original bytes of one attachment of a message that is not downloaded,
/// with a single download. `email` locates the message and carries the
/// attachment list that `attachment_id` belongs to. Like `download_body`,
/// nothing is cached.
pub fn download_attachment(account: &Account, email: &Email, attachment_id: u64) -> Result<Vec<u8>, String> {
    let originals = mail::download_originals(account, std::slice::from_ref(email))?;
    let raw = originals
        .first()
        .filter(|raw| !raw.is_empty())
        .ok_or("Gmail did not return this message")?;
    attachment_from_original(raw, email, attachment_id)
}

/// Ids are given by `assign_attachment_ids` over the attachments in the order
/// the parser lists them, and `parse_downloaded_content` pairs each with the
/// part at the same position. The position of `attachment_id` therefore
/// selects its part; names and sizes never do, so same-named files stay apart.
fn attachment_from_original(raw: &[u8], email: &Email, attachment_id: u64) -> Result<Vec<u8>, String> {
    let mut loaded = mail::parse_message_inner(raw, email.id, email.account_id, None)
        .ok_or("Gmail message could not be read")?;
    assign_attachment_ids(email, &mut loaded.attachments);
    let index = loaded
        .attachments
        .iter()
        .position(|attachment| attachment.id == attachment_id)
        .ok_or("The attachment is no longer in this message")?;
    let parsed = mail_parser::MessageParser::default()
        .parse(raw)
        .ok_or("Gmail message could not be read")?;
    parsed
        .attachments()
        .nth(index)
        .map(|part| part.contents().to_vec())
        .ok_or_else(|| "The attachment is no longer in this message".into())
}

fn download_content(account: &Account, email: &Email, extract_attachments: bool) -> Result<Email, String> {
    let originals = mail::download_originals(account, std::slice::from_ref(email))?;
    let raw = originals
        .first()
        .ok_or("Gmail did not return this message")?;
    parse_downloaded_content(raw, email, extract_attachments)
}

fn parse_downloaded_content(raw: &[u8], email: &Email, extract_attachments: bool) -> Result<Email, String> {
    let mut loaded = mail::parse_message_inner(raw, email.id, email.account_id, None)
        .ok_or("Gmail message could not be read")?;
    use sha2::{Digest, Sha256};
    loaded.remote_content_hash = format!("{:x}", Sha256::digest(raw));
    assign_attachment_ids(email, &mut loaded.attachments);
    if let Some(parsed) = mail_parser::MessageParser::default().parse(raw) {
        for (attachment, part) in loaded
            .attachments
            .iter_mut()
            .zip(parsed.attachments())
        {
            if extract_attachments {
                attachment.inline_text =
                    super::attach::bytes_to_text(&attachment.name, part.contents()).ok();
            }
        }
    }
    Ok(loaded)
}

#[cfg(test)]
#[test]
fn message_preview_skips_attachment_extraction_but_preserves_names_sizes_and_ids() {
    let raw = b"From: test@example.com\r\nSubject: Invoice\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=parts\r\n\r\n--parts\r\nContent-Type: text/plain\r\n\r\nRead this message first.\r\n--parts\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=invoice.txt\r\n\r\nExtract this only on request.\r\n--parts--\r\n";
    let email = Email { id: 9, attachments: vec![crate::model::Attachment { id: 12345, name: "invoice.txt".into(), mime: "text/plain".into(), ..Default::default() }], ..Default::default() };
    let preview = parse_downloaded_content(raw, &email, false).unwrap();
    assert!(preview.body.contains("Read this message first."));
    assert_eq!(preview.attachments[0].id, 12345);
    assert_eq!(preview.attachments[0].name, "invoice.txt");
    assert!(preview.attachments[0].size > 0);
    assert!(preview.attachments[0].inline_text.is_none());
    let full = parse_downloaded_content(raw, &email, true).unwrap();
    assert_eq!(full.attachments[0].inline_text.as_deref(), Some("Extract this only on request."));
}

#[cfg(test)]
#[test]
fn an_online_attachment_is_the_part_its_id_names_even_when_names_repeat() {
    // Stored in a different order than the original lists them, with two
    // different files both called scan.txt: ids follow `assign_attachment_ids`.
    let raw = b"From: test@example.com\r\nSubject: Scans\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=parts\r\n\r\n--parts\r\nContent-Type: text/plain\r\n\r\nBody.\r\n--parts\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=scan.txt\r\n\r\nfirst\r\n--parts\r\nContent-Type: application/octet-stream\r\nContent-Disposition: attachment; filename=photo.jpg\r\nContent-Transfer-Encoding: base64\r\n\r\n/9j/2Q==\r\n--parts\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=scan.txt\r\n\r\nsecond\r\n--parts--\r\n";
    let stored = |id, name: &str, mime: &str| crate::model::Attachment { id, name: name.into(), mime: mime.into(), ..Default::default() };
    let email = Email {
        id: 9,
        attachments: vec![
            stored(30, "scan.txt", "text/plain"),
            stored(20, "photo.jpg", "application/octet-stream"),
            stored(10, "scan.txt", "text/plain"),
        ],
        ..Default::default()
    };
    assert_eq!(attachment_from_original(raw, &email, 30).unwrap(), b"first");
    assert_eq!(attachment_from_original(raw, &email, 10).unwrap(), b"second");
    assert_eq!(attachment_from_original(raw, &email, 20).unwrap(), [0xFF, 0xD8, 0xFF, 0xD9]);
    assert!(attachment_from_original(raw, &email, 99).is_err());
}

#[cfg(test)]
#[test]
fn an_online_attachment_is_found_by_the_ids_the_reader_listed_even_with_an_unnamed_part() {
    // The calendar part has no file name, so the server structure does not
    // list it and the stored ids skip it, while the parser counts it first.
    let raw = b"From: test@example.com\r\nSubject: Invitation\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: multipart/alternative; boundary=a\r\n\r\n--a\r\nContent-Type: text/plain\r\n\r\nJoin us.\r\n--a\r\nContent-Type: text/calendar; method=REQUEST\r\n\r\nBEGIN:VCALENDAR\r\n--a--\r\n--m\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=report.pdf\r\nContent-Transfer-Encoding: base64\r\n\r\nJVBERi0=\r\n--m--\r\n";
    let listed = |id, name: &str, mime: &str| crate::model::Attachment { id, name: name.into(), mime: mime.into(), ..Default::default() };
    // The list the reader showed after a server search: parser order, 9000 + index.
    let shown = Email {
        id: 9,
        attachments: vec![listed(9000, "attachment", "text/calendar"), listed(9001, "report.pdf", "application/pdf")],
        ..Default::default()
    };
    assert_eq!(attachment_from_original(raw, &shown, 9001).unwrap(), b"%PDF-");
    assert_eq!(attachment_from_original(raw, &shown, 9000).unwrap(), b"BEGIN:VCALENDAR");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::Cursor,
        sync::{Arc, Mutex},
    };
    struct Wire {
        input: Cursor<Vec<u8>>,
        output: Arc<Mutex<Vec<u8>>>,
    }
    impl Read for Wire {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.input.read(buf)
        }
    }
    impl Write for Wire {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.output.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn policy_session(replies: String) -> (imap::Session<Wire>, Arc<Mutex<Vec<u8>>>) {
        let output = Arc::new(Mutex::new(Vec::new()));
        let mut client = imap::Client::new(Wire {
            input: Cursor::new(format!("* OK ready\r\na1 OK login\r\n{replies}").into_bytes()),
            output: output.clone(),
        });
        client.read_greeting().unwrap();
        let session = client
            .login("synthetic", "synthetic")
            .map_err(|e| e.0)
            .unwrap();
        (session, output)
    }

    struct NoExtensions;
    impl GmailExtensions for NoExtensions {
        fn ids(&self, _: &str) -> Result<std::collections::HashMap<u32, u64>, String> { panic!("Membership searches already use the verified global ID") }
    }
    fn cached_inbox() -> Email {
        Email { id: 9, account_id: 3, folder: "Inbox".into(), remote_folder: Some("INBOX".into()), uid: Some(9), remote_uid_validity: Some(7), remote_global_id: Some(700), body: "cached body".into(), pinned: true, ..Default::default() }
    }
    fn roles() -> &'static str {
        "* LIST (\\All) \"/\" \"All\"\r\n* LIST (\\Trash) \"/\" \"Bin\"\r\n* LIST (\\Junk) \"/\" \"Junk\"\r\na2 OK listed\r\n"
    }
    fn absent_responses(start: usize, folders: usize) -> String {
        (0..folders).map(|n| format!("* 0 EXISTS\r\na{} OK examined\r\n* SEARCH\r\na{} OK searched\r\n", start + n*2, start + n*2 + 1)).collect()
    }
    #[test]
    fn authoritative_absence_retains_cache_tasks_as_local_trash() {
        let (mut session, _) = policy_session(format!("{}{}", roles(), absent_responses(3, 4)));
        let mut output = vec![];
        reconcile_known(&mut session, &NoExtensions, &Account { id: 3, ..Default::default() }, &[cached_inbox()], Some(&(Some(7), HashSet::new())), &mut output).unwrap();
        assert_eq!(output.len(), 1);
        assert_eq!(output[0].folder, "Trash");
        assert!(output[0].remote_deleted && output[0].pinned);
        assert_eq!(output[0].body, "cached body");
        assert!(output[0].uid.is_none() && output[0].remote_folder.is_none() && output[0].remote_locations.is_empty());
    }
    #[test]
    fn missing_hidden_roles_or_transient_errors_never_prove_deletion() {
        let (mut session, _) = policy_session(format!("* LIST (\\All) \"/\" \"All\"\r\na2 OK listed\r\n{}", absent_responses(3, 2)));
        let mut output = vec![];
        let account = Account { id: 3, ..Default::default() };
        reconcile_known(&mut session, &NoExtensions, &account, &[cached_inbox()], None, &mut output).unwrap();
        assert!(output.is_empty());
        let (mut session, _) = policy_session(format!("{}* 0 EXISTS\r\na3 OK examined\r\na4 NO retry later\r\n", roles()));
        assert!(reconcile_known(&mut session, &NoExtensions, &account, &[cached_inbox()], None, &mut output).is_err());
        assert!(output.is_empty());
    }
    #[test]
    fn old_inbox_outside_window_and_unverified_legacy_messages_are_left_intact() {
        let (mut session, commands) = policy_session(String::new());
        let mut legacy = cached_inbox();
        legacy.remote_global_id = None;
        let mut output = vec![];
        reconcile_known(&mut session, &NoExtensions, &Account { id: 3, ..Default::default() }, &[cached_inbox(), legacy], Some(&(Some(7), HashSet::from([9]))), &mut output).unwrap();
        assert!(output.is_empty());
        assert!(!String::from_utf8(commands.lock().unwrap().clone()).unwrap().contains("LIST"));
    }
    #[test]
    fn missing_inbox_global_identity_is_found_in_archive() {
        let (mut session, _) = policy_session(format!("{}{}* OK [UIDVALIDITY 12] stable\r\na9 OK examined\r\n* SEARCH 90\r\na10 OK searched\r\n* 1 FETCH (UID 90 FLAGS (\\Seen))\r\na11 OK fetched\r\n", roles(), absent_responses(3, 3)));
        let mut output = vec![];
        reconcile_known(&mut session, &NoExtensions, &Account { id: 3, ..Default::default() }, &[cached_inbox()], None, &mut output).unwrap();
        assert_eq!(output[0].folder, "Archive");
        assert_eq!(output[0].uid, Some(90));
        assert_eq!(output[0].remote_uid_validity, Some(12));
        assert!(output[0].pinned && output[0].read);
        assert!(!output[0].remote_deleted);
    }
    #[test]
    fn sent_echo_requires_full_payload_not_just_matching_headers() {
        use sha2::{Digest, Sha256};
        let mut sent = Email {
            account_id: 3, local_sent: true, message_id: "<qamille-opaque@qamille.local>".into(),
            from_addr: "sender@example.com".into(), to: vec!["Recipient Name <receiver@EXAMPLE.COM>".into()], cc: vec!["Copy Name <copy@EXAMPLE.COM>".into()], subject: "Invoice".into(), body: "Hello".into(),
            date: 2_000_000_000, attachments: vec![crate::model::Attachment { name: "invoice.txt".into(), mime: "application/octet-stream".into(), ..Default::default() }], ..Default::default()
        };
        sent.outgoing_fingerprint = crate::domain::mail_identity::outgoing_fingerprint(&sent, &[("invoice.txt".into(), format!("{:x}", Sha256::digest(b"invoice")))]);
        let raw = crate::domain::mail_export::eml(&sent, vec![b"invoice".to_vec()]).unwrap();
        let verified = verified_sent_echo(&sent, &raw).unwrap();
        assert!(crate::domain::mail_identity::same_message(&sent, &verified));
        let impostor = crate::domain::mail_export::eml(&sent, vec![b"changed".to_vec()]).unwrap();
        assert!(verified_sent_echo(&sent, &impostor).is_none());
        let mut legacy = sent.clone();
        legacy.local_sent = false;
        assert!(verified_sent_echo(&legacy, &raw).is_none());
    }

    #[test]
    fn normal_policy_sync_requests_headers_without_body_download_or_disk_files() {
        let header = "From: sender@example.com\r\nSubject: Online\r\nMessage-ID: <online@example.com>\r\nDate: Sun, 01 Nov 2026 10:00:00 +0000\r\n\r\n";
        let (mut session, commands) = policy_session(format!(
            "a2 OK listed\r\n* 1 EXISTS\r\na3 OK examined\r\n* SEARCH 9\r\na4 OK searched\r\n* 1 FETCH (UID 9 FLAGS (\\Seen) RFC822.SIZE 20000000 BODY[HEADER] {{{}}}\r\n{header})\r\na5 OK fetched\r\n",
            header.len()
        ));
        let account = Account {
            id: 3,
            ..Default::default()
        };
        let result = fetch_policy(
            &mut session,
            &account,
            &std::env::temp_dir(),
            &SyncRequest {
                max_bytes: Some(100_000),
                known: &[],
            },
        )
        .unwrap();
        assert_eq!(result.batch.emails.len(), 1);
        assert!(result.batch.emails[0].remote_only);
        assert!(result.batch.emails[0].body.is_empty());
        assert!(result.batch.emails[0].read);
        assert!(!result.offline_complete);
        assert!(result.notice.is_none());
        let wire = String::from_utf8(commands.lock().unwrap().clone()).unwrap();
        assert!(wire.contains("EXAMINE \"INBOX\""));
        assert!(wire.contains("BODY.PEEK[HEADER]"));
        assert!(!wire.contains("BODY.PEEK[]"));
    }

    #[test]
    fn row_without_headers_is_skipped_and_marks_sync_incomplete() {
        let header = "From: sender@example.com\r\nSubject: Online\r\nMessage-ID: <online@example.com>\r\nDate: Sun, 01 Nov 2026 10:00:00 +0000\r\n\r\n";
        let (mut session, _) = policy_session(format!(
            "a2 OK listed\r\n* 2 EXISTS\r\na3 OK examined\r\n* SEARCH 8 9\r\na4 OK searched\r\n* 1 FETCH (UID 8 FLAGS (\\Seen))\r\n* 2 FETCH (UID 9 FLAGS (\\Seen) RFC822.SIZE 200 BODY[HEADER] {{{}}}\r\n{header})\r\na5 OK fetched\r\n",
            header.len()
        ));
        let account = Account { id: 3, ..Default::default() };
        let result = fetch_policy(
            &mut session, &account, &std::env::temp_dir(),
            &SyncRequest { max_bytes: Some(100_000), known: &[] },
        ).unwrap();
        assert_eq!(result.batch.emails.len(), 1);
        assert_eq!(result.batch.emails[0].uid, Some(9));
        assert!(!result.offline_complete);
        assert!(result.notice.unwrap().contains("readable headers"));
    }

    #[test]
    fn gmail_policy_headers_capture_global_identity_through_real_session_transport() {
        let header = "From: sender@example.com\r\nSubject: Online\r\nMessage-ID: <online@example.com>\r\nDate: Sun, 01 Nov 2026 10:00:00 +0000\r\n\r\n";
        let script = format!("* OK ready\r\na1 OK login\r\na2 OK listed\r\n* OK [UIDVALIDITY 7] stable\r\n* 1 EXISTS\r\na3 OK examined\r\n* SEARCH 9\r\na4 OK searched\r\n* 1 FETCH (UID 9 X-GM-MSGID 700)\r\ngmail1 OK fetched\r\n* 1 FETCH (UID 9 FLAGS (\\Seen) RFC822.SIZE 200 BODY[HEADER] {{{}}}\r\n{header})\r\na5 OK fetched\r\na6 OK noop\r\n", header.len());
        let output = Arc::new(Mutex::new(Vec::new()));
        let account = Account { id: 3, ..Default::default() };
        let mut session = mail::extension::Session::new(Wire { input: Cursor::new(script.into_bytes()), output }, &account).unwrap();
        let extensions = session.extensions.clone();
        let result = fetch_policy_extended(&mut session, &account, &std::env::temp_dir(), &SyncRequest { max_bytes: Some(100_000), known: &[] }, Some(&extensions)).unwrap();
        assert_eq!(result.batch.emails[0].remote_global_id, Some(700));
        assert!(result.batch.emails[0].remote_only);
        session.noop().unwrap();
    }

    #[test]
    fn full_offline_folder_discovery_is_localized_and_all_mail_is_last() {
        let (mut session, _) = policy_session(
            "* LIST (\\All) \"/\" \"Posta/Tutti\"\r\n* LIST (\\Sent) \"/\" \"Posta/Inviati\"\r\n* LIST (\\Trash) \"/\" \"Posta/Cestino\"\r\n* LIST (\\Junk) \"/\" \"Posta/Spam\"\r\n* LIST (\\Drafts) \"/\" \"Posta/Bozze\"\r\na2 OK listed\r\n".into()
        );
        let mut account = Account::default();
        account.storage_policy.keep_all_offline = true;
        let (folders, archive, trash, notice) = folder_plan(&mut session, &account).unwrap();
        assert_eq!(folders.len(), 6);
        assert_eq!(folders.last().unwrap().logical, "Archive");
        assert_eq!(archive.as_deref(), Some("Posta/Tutti"));
        assert_eq!(trash.as_deref(), Some("Posta/Cestino"));
        assert!(notice.is_none());
    }

    #[test]
    fn absent_folders_never_claim_complete_offline_coverage() {
        let (mut session, _) =
            policy_session("* LIST (\\All) \"/\" \"All Mail\"\r\na2 OK listed\r\n".into());
        let mut account = Account::default();
        account.storage_policy.keep_all_offline = true;
        let (_, _, _, notice) = folder_plan(&mut session, &account).unwrap();
        assert!(notice.unwrap().contains("partial"));
    }

    #[test]
    fn attachment_descriptors_do_not_require_content_or_local_files() {
        use imap_proto::{
            BodyContentCommon, BodyContentSinglePart, BodyStructure, ContentDisposition,
            ContentEncoding, ContentType,
        };
        let structure = BodyStructure::Basic {
            common: BodyContentCommon {
                ty: ContentType {
                    ty: "APPLICATION",
                    subtype: "PDF",
                    params: None,
                },
                disposition: Some(ContentDisposition {
                    ty: "ATTACHMENT",
                    params: Some(vec![("FILENAME", "report.pdf")]),
                }),
                language: None,
                location: None,
            },
            other: BodyContentSinglePart {
                id: None,
                md5: None,
                description: None,
                transfer_encoding: ContentEncoding::Base64,
                octets: 20_000_000,
            },
            extension: None,
        };
        let mut attachments = Vec::new();
        attachment_metadata(&structure, &mut attachments);
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].name, "report.pdf");
        assert_eq!(attachments[0].mime, "application/pdf");
        assert!(attachments[0].path.is_none() && attachments[0].inline_text.is_none());
    }

    #[test]
    fn exhausted_account_allowance_does_not_request_an_offline_body() {
        let header = "From: sender@example.com\r\nSubject: Too large\r\nMessage-ID: <large@example.com>\r\nDate: Sun, 01 Nov 2026 10:00:00 +0000\r\n\r\n";
        let (mut session, commands) = policy_session(format!(
            "a2 OK listed\r\n* OK [UIDVALIDITY 7] stable\r\n* 1 EXISTS\r\na3 OK examined\r\n* SEARCH 9\r\na4 OK searched\r\n* 1 FETCH (UID 9 FLAGS () RFC822.SIZE 20000000 BODY[HEADER] {{{}}}\r\n{header})\r\na5 OK fetched\r\n",
            header.len()
        ));
        let mut account = Account {
            id: 3,
            ..Default::default()
        };
        account.storage_policy.keep_all_offline = true;
        let result = fetch_policy(
            &mut session,
            &account,
            &std::env::temp_dir(),
            &SyncRequest {
                max_bytes: Some(10_000),
                known: &[],
            },
        )
        .unwrap();
        assert_eq!(result.batch.emails.len(), 1);
        assert!(result.batch.emails[0].remote_only);
        assert!(!result.offline_complete);
        assert!(
            !String::from_utf8(commands.lock().unwrap().clone())
                .unwrap()
                .contains("BODY.PEEK[]")
        );
    }

    #[test]
    fn reused_uid_from_another_folder_is_not_the_same_message() {
        let email = Email {
            account_id: 1,
            uid: Some(7),
            remote_folder: Some("INBOX".into()),
            subject: "A".into(),
            ..Default::default()
        };
        let mut moved = email.clone();
        moved.remote_folder = Some("Sent".into());
        assert!(!same_location(&email, &moved));
        moved.remote_folder = Some("inbox".into());
        assert!(same_location(&email, &moved));
        moved.account_id = 2;
        assert!(!same_location(&email, &moved));
    }
}
