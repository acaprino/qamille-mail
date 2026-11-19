//! The other messages of a conversation as a tree, flattened for the mail list (ADR-0010).
//! A conversation is a set (`mail_export::conversation_ids`); this adds the parent of each
//! message from `Email::in_reply_to` and walks it in tree order.
use super::model::{Email, Store};
use std::collections::{HashMap, HashSet};

/// Replies a list row carries; the rest are read on demand (`read_replies`).
pub const REPLY_CAP: usize = 30;

/// One message under a conversation's main row. `parent_id` is `None` for a root of the
/// tree, including a message that answers the main row itself.
pub struct ReplyNode<'a> {
    pub email: &'a Email,
    pub parent_id: Option<u64>,
}

/// A Message-ID without angle brackets or surrounding spaces.
pub fn normalize_id(value: &str) -> &str {
    super::mail_export::key(value)
}

/// The header a message answers. A store written before `in_reply_to` existed (the JSON engine
/// keeps it as it was, and a migration cannot reach it) holds that header in `thread_id` for
/// received mail, so the same fallback the SQLite migration applies is applied here. A reply
/// Qamille sent copied its parent's thread, one level too high, so it is never read that way.
fn parent_header(email: &Email) -> &str {
    let stored = normalize_id(&email.in_reply_to);
    if !stored.is_empty() || email.local_sent || email.folder.eq_ignore_ascii_case("Sent") {
        return stored;
    }
    let thread = normalize_id(&email.thread_id);
    if thread == normalize_id(&email.message_id) {
        ""
    } else {
        thread
    }
}

/// The other messages of the conversation that holds `main_id`, in tree order, as the open view
/// `view_folder` lists them.
pub fn reply_nodes<'a>(
    store: &'a Store,
    groups: &HashMap<u64, u64>,
    main_id: u64,
    view_folder: &str,
) -> Vec<ReplyNode<'a>> {
    let (Some(main), Some(group)) = (store.email(main_id), groups.get(&main_id)) else {
        return Vec::new();
    };
    let all: Vec<&Email> = store
        .emails
        .iter()
        .filter(|e| e.account_id == main.account_id && groups.get(&e.id) == Some(group))
        .collect();
    reply_tree(main, &all, view_folder)
}

/// `group` is every message of the conversation, the main one included.
///
/// Listed: not Trash or Spam (unless that folder is the open view, which holds its own members:
/// ADR-0010 INV-004 hides them from the live views only), not the main message nor a copy of it
/// (same Message-ID), and one copy per Message-ID, the lowest id. A message's parent is the one
/// its In-Reply-To names; when that cannot be found it hangs under the oldest message of the
/// whole conversation, if that one is listed, otherwise it is a root. A reply cycle opens at its
/// oldest message. Siblings and roots go by `(date, id)`, depth first.
pub fn reply_tree<'a>(
    main: &Email,
    group: &[&'a Email],
    view_folder: &str,
) -> Vec<ReplyNode<'a>> {
    let main_key = normalize_id(&main.message_id);
    let oldest = group.iter().min_by_key(|e| (e.date, e.id)).map(|e| e.id);
    // The Trash and Spam views list their own folder, the same way `list_mail` matches a folder.
    let hidden = |folder: &str| {
        matches!(folder, "Trash" | "Spam") && !folder.eq_ignore_ascii_case(view_folder)
    };

    let mut candidates: Vec<&'a Email> = group
        .iter()
        .copied()
        .filter(|e| e.id != main.id && !hidden(&e.folder))
        .filter(|e| {
            let key = normalize_id(&e.message_id);
            key.is_empty() || key != main_key
        })
        .collect();
    candidates.sort_by_key(|e| e.id);
    let mut seen = HashSet::new();
    candidates.retain(|e| {
        let key = normalize_id(&e.message_id);
        key.is_empty() || seen.insert(key)
    });
    candidates.sort_by_key(|e| (e.date, e.id));

    let by_id: HashMap<u64, &'a Email> = candidates.iter().map(|e| (e.id, *e)).collect();
    let by_key: HashMap<&str, u64> = candidates
        .iter()
        .filter(|e| !normalize_id(&e.message_id).is_empty())
        .map(|e| (normalize_id(&e.message_id), e.id))
        .collect();

    let mut parent: HashMap<u64, Option<u64>> = HashMap::new();
    for e in &candidates {
        let named = parent_header(e);
        let direct = if named.is_empty() {
            None
        } else if named == main_key {
            Some(None)
        } else {
            by_key.get(named).filter(|&&id| id != e.id).map(|&id| Some(id))
        };
        let chosen = direct.unwrap_or_else(|| {
            oldest.filter(|id| *id != e.id && by_id.contains_key(id))
        });
        parent.insert(e.id, chosen);
    }

    for e in &candidates {
        let mut path = vec![e.id];
        let mut current = e.id;
        while let Some(Some(next)) = parent.get(&current).copied() {
            if let Some(at) = path.iter().position(|id| *id == next) {
                let lowest = path[at..]
                    .iter()
                    .copied()
                    .min_by_key(|id| (by_id[id].date, *id))
                    .expect("a cycle has members");
                parent.insert(lowest, None);
                break;
            }
            path.push(next);
            current = next;
        }
    }

    let mut children: HashMap<Option<u64>, Vec<u64>> = HashMap::new();
    for e in &candidates {
        children.entry(parent[&e.id]).or_default().push(e.id);
    }
    let mut out = Vec::with_capacity(candidates.len());
    let mut visited = HashSet::new();
    let mut stack: Vec<u64> = children.get(&None).into_iter().flatten().rev().copied().collect();
    while let Some(id) = stack.pop() {
        if !visited.insert(id) {
            continue;
        }
        out.push(ReplyNode { email: by_id[&id], parent_id: parent[&id] });
        stack.extend(children.get(&Some(id)).into_iter().flatten().rev().copied());
    }
    // Opening each cycle makes every message reachable; should a case slip through, the
    // message still shows, in date order, instead of silently disappearing.
    for e in &candidates {
        if visited.insert(e.id) {
            out.push(ReplyNode { email: e, parent_id: parent[&e.id] });
        }
    }
    out
}
