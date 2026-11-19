//! MCP server (stdio, JSON-RPC 2.0). Start with `qamille --mcp`. Lets an AI assistant search
//! mail, read threads and attachments, manage drafts, act on mail, read the calendar and meeting
//! notes. Access level (read-only / triage / send) is set in Qamille's Settings.

use crate::actions;
use crate::ai;
use crate::attach;
use crate::model::{Access, Draft, Event, Store, now_ts};
use crate::rag;
use crate::rules;
use chrono::{NaiveDate, TimeZone};
use serde_json::{Value, json};
use std::io::{BufRead, Write};
pub mod server;
pub use server::serve;
pub mod tools;
pub mod files;
pub mod calendar;
pub mod contacts;
pub mod jobs;
mod grants;
pub use crate::application::mcp_outbox as outbox;

fn tool(name: &str, desc: &str, props: Value, required: &[&str]) -> Value {
    json!({"name": name, "description": desc, "inputSchema": {"type": "object", "properties": props, "required": required}})
}

fn tools() -> Value {
    let s = |d: &str| json!({"type": "string", "description": d});
    let n = |d: &str| json!({"type": "integer", "description": d});
    json!([
        tool("list_accounts", "List mail accounts, folders/labels with counts and the access level.", json!({}), &[]),
        tool("list_emails", "List local emails newest first. Validated filters: from:, to:, cc:, subject:, body:, on:YYYY-MM-DD, between:YYYY-MM-DD..YYYY-MM-DD (inclusive), around:YYYY-MM-DD/6d (default ±6 calendar days), before:, after:, newer_than:7d, older_than:, has:attachment, is:unread|read|starred|pinned|unreplied|task, category:, label:, filename:, type:, account:. AND, OR, negation and parentheses supported. Dates use app time zone and regional settings. Local coverage may be incomplete.",
            json!({"folder": s("Folder name (default Inbox)"), "filter": s("Filter operators"), "page": n("Page, from 1"), "page_size": n("Default 20")}), &[]),
        tool("search_emails", "Search local mail and extracted attachment text. Shared validated operators include around:, between:, on:, from:, to:, subject:, type:, account:, in:anywhere, OR and exclusions. Exact addresses match exactly. Results are paged without a 20-result total cap. Coverage may be incomplete.",
            json!({"query": s("Words and/or operators"), "folder": s("Optional folder"), "page": n("Page from 1"), "page_size": n("Default 20, maximum 100")}), &["query"]),
        tool("semantic_search", "Search mail and attachment text by meaning with the local embedding model, combined with exact-word matching. Works across languages and when the query's words don't appear in the mail. Operators such as from: or is:unread narrow the results. Returns the passage that matched.",
            json!({"query": s("A question or description, optionally with operators"), "limit": n("Default 20")}), &["query"]),
        tool("read_email", "Read one email with headers, body and attachment list.", json!({"id": n("Email ID")}), &["id"]),
        tool("read_attachment", "Read an attachment as text (PDF, Word, Excel, PowerPoint, text, CSV, HTML, calendar).",
            json!({"email_id": n("Email ID"), "attachment_id": n("Attachment ID from read_email")}), &["email_id", "attachment_id"]),
        tool("list_drafts", "List drafts.", json!({}), &[]),
        tool("create_draft", "Create a draft (never sends). reply_to links it to an email.",
            json!({"to": s("Comma-separated"), "cc": s("Comma-separated"), "subject": s("Subject"), "body": s("Body text"), "reply_to": n("Email ID being answered"), "account": s("From account email")}), &["body"]),
        tool("update_draft", "Edit an existing draft.", json!({"id": n("Draft ID"), "to": s(""), "cc": s(""), "subject": s(""), "body": s("")}), &["id"]),
        tool("delete_draft", "Delete a draft permanently.", json!({"id": n("Draft ID")}), &["id"]),
        tool("send_draft", "Queue a draft for sending by the Qamille window (needs send access). date for Send Later (yyyy-MM-ddTHH:mm or preset like Tomorrow).",
            json!({"id": n("Draft ID"), "date": s("Optional")}), &["id"]),
        tool("email_action", &format!("Act on emails. action: {}. value: date/preset for snooze, changeReminder, setDue; folder for moveToFolder; label for attach/detachLabel.", actions::ACTIONS.join(", ")),
            json!({"action": s("Action name"), "ids": {"type": "array", "items": {"type": "integer"}}, "value": s("Optional value")}), &["action", "ids"]),
        tool("classify_email", "Suggest category and labels for an email (uses Qamille's AI settings).", json!({"id": n("Email ID")}), &["id"]),
        tool("generate_reply", "Write a reply suggestion for an email and save it as a draft.", json!({"id": n("Email ID"), "instruction": s("What the reply should say")}), &["id"]),
        tool("list_events", "Calendar events in a range (default: next 7 days).", json!({"from": s("yyyy-MM-dd"), "to": s("yyyy-MM-dd")}), &[]),
        tool("create_event", "Create a calendar event (needs send access). online=true adds a video meeting link.",
            json!({"title": s("Title"), "start": s("yyyy-MM-ddTHH:mm"), "minutes": n("Duration, default 30"), "attendees": s("Comma-separated emails"), "online": {"type": "boolean"}, "location": s("")}), &["title", "start"]),
        tool("find_free_slots", "Free time slots on a day between 09:00 and 20:00.", json!({"date": s("yyyy-MM-dd"), "minutes": n("Length, default 30")}), &["date"]),
        tool("list_meetings", "List meeting notes (recordings, transcripts, summaries).", json!({}), &[]),
        tool("search_meetings", "Search meeting transcripts, notes and summaries with source references; respects search exclusions.", json!({"query": s("Words to find")}), &["query"]),
        tool("read_meeting_segments", "Read paginated timestamped transcript passages. Optional version chooses a preserved transcript.", json!({"id": n("Meeting ID"), "version": s("Transcript version"), "offset": n("Offset, default 0"), "limit": n("Maximum 100")}), &["id"]),
        tool("read_meeting", "Read a meeting note: summary, key points, action items; optional transcript and notes.",
            json!({"id": n("Meeting ID"), "transcript": {"type": "boolean"}, "notes": {"type": "boolean"}}), &["id"]),
    ])
}

fn need(level: Access, have: Access) -> Result<(), String> {
    let rank = |a: Access| match a {
        Access::ReadOnly => 0,
        Access::Triage => 1,
        Access::Send => 2,
    };
    if rank(have) >= rank(level) {
        Ok(())
    } else {
        Err(format!(
            "This needs '{}' access; Qamille allows '{}'. Change it in Qamille → Settings → Assistant access.",
            level.label(),
            have.label()
        ))
    }
}

fn arg_str<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a[k].as_str().filter(|s| !s.is_empty())
}

fn day_start(s: &str, store: &Store) -> Result<i64, String> {
    let date = NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| "Use YYYY-MM-DD".to_owned())?;
    crate::domain::regional::day_start(date, &crate::infrastructure::timezone::resolved_timezone(&store.settings))
}

pub fn call(store: &mut Store, name: &str, a: &Value) -> Result<(String, bool), String> {
    let access = store.settings.mcp_access;
    let mut changed = false;
    let out = match name {
        "list_accounts" => {
            let mut s = format!("Access level: {}\n\nAccounts:\n", access.label());
            for acc in &store.accounts {
                s.push_str(&format!("  {} <{}> ({:?})\n", acc.name, acc.email, acc.kind));
            }
            let mut folders: Vec<String> = store.emails.iter().map(|e| e.folder.clone()).collect();
            folders.sort();
            folders.dedup();
            s.push_str("\nFolders:\n");
            for f in folders {
                let n = store.emails.iter().filter(|e| e.folder == f).count();
                let u = store.emails.iter().filter(|e| e.folder == f && !e.read).count();
                s.push_str(&format!("  {f}: {n} ({u} unread)\n"));
            }
            s.push_str("\nLabels:\n");
            for l in store.all_labels() {
                s.push_str(&format!("  {l}\n"));
            }
            s
        }
        "list_emails" => {
            let folder = arg_str(a, "folder").unwrap_or("Inbox");
            let filter = arg_str(a, "filter").unwrap_or("");
            let page = a["page"].as_u64().unwrap_or(1).max(1) as usize;
            let size = a["page_size"].as_u64().unwrap_or(rules::PAGE_SIZE as u64).clamp(1, 200) as usize;
            let now = now_ts();
            let scope = format!("in:\"{}\"", folder.replace('"', "\\\""));
            let scoped_filter = if filter.is_empty() { scope } else { format!("({filter}) {scope}") };
            let mut prepared = store.clone();
            crate::infrastructure::search_support::prepare(&mut prepared, &scoped_filter, None, now)?;
            let parsed = crate::domain::search::ParsedQuery::parse(&prepared, &scoped_filter, now)?;
            let mut v: Vec<&crate::model::Email> = prepared
                .emails
                .iter()
                .filter(|e| match folder.to_lowercase().as_str() {
                    "tasks" => e.is_task(),
                    "snoozed" => e.is_snoozed(now),
                    f => e.folder.to_lowercase() == f && !e.is_snoozed(now) && !(f == "inbox" && e.done),
                })
                .filter(|e| parsed.matches(e, store))
                .collect();
            v.sort_by(|x, y| y.inbox_date().cmp(&x.inbox_date()));
            let total = v.len();
            let mut s = format!("{total} local emails in {folder} (page {page}, {size} per page). Server coverage is not verified.\n\nID | From | Date | Subject | Flags\n");
            for e in v.into_iter().skip(page.saturating_sub(1).saturating_mul(size)).take(size) {
                s.push_str(&format!(
                    "{} | {} | {} | {} | {}{}{}\n",
                    e.id, e.from_name, crate::infrastructure::timezone::format_timestamp(&store.settings, e.date), e.subject,
                    if e.read { "" } else { "unread " },
                    if e.pinned { "pinned " } else { "" },
                    if e.attachments.is_empty() { "" } else { "attachment" }
                ));
            }
            s
        }
        "search_emails" => {
            let q = arg_str(a, "query").ok_or("query is required")?;
            let query = if let Some(folder) = arg_str(a, "folder") { format!("({q}) in:\"{}\"", folder.replace('"', "\\\"")) } else { q.into() };
            let ids = crate::infrastructure::search_support::execute(store, &query, None, now_ts(), "")?;
            let page = a["page"].as_u64().unwrap_or(1).max(1) as usize;
            let size = a["page_size"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
            let mut display = store.clone();
            crate::infrastructure::search_support::resolve_region(&mut display);
            let mut s = format!("{} local results for \"{q}\" (page {page}, {size} per page). Server coverage is not verified.\n\n", ids.len());
            for id in ids.into_iter().skip(page.saturating_sub(1).saturating_mul(size)).take(size) {
                if let Some(e) = store.email(id) {
                    s.push_str(&actions::email_text(&display, e, true));
                    s.push_str("\n---\n");
                }
            }
            s
        }
        "semantic_search" => {
            let q = arg_str(a, "query").ok_or("query is required")?;
            crate::infrastructure::search_support::parse(store, q, None, now_ts())?;
            let limit = a["limit"].as_u64().unwrap_or(rules::SEARCH_LIMIT as u64).clamp(1, 100) as usize;
            let compute = crate::infrastructure::compute_factory::current();
            let (hits, unavailable) = rag::search_disk_reported(store, q, limit, &*compute)?;
            let mut s = format!("{} results for \"{q}\"\n", hits.len());
            if let Some(error) = unavailable {
                s.push_str(crate::application::compute::mcp_semantic_notice(error.code));
                s.push('\n');
            }
            s.push('\n');
            s.push_str(&rag::context_from_hits(store, &hits));
            s
        }
        "read_email" => {
            let id = a["id"].as_u64().ok_or("id is required")?;
            let mut display = store.clone();
            crate::infrastructure::search_support::resolve_region(&mut display);
            let e = display.email(id).ok_or("No such email")?;
            let thread: Vec<String> = display
                .emails
                .iter()
                .filter(|o| o.thread_id == e.thread_id && o.id != e.id)
                .map(|o| actions::email_text(&display, o, true))
                .collect();
            let mut s = actions::email_text(&display, e, true);
            if !thread.is_empty() {
                s.push_str("\nOther messages in this thread:\n");
                s.push_str(&thread.join("\n---\n"));
            }
            s
        }
        "read_attachment" => {
            let id = a["email_id"].as_u64().ok_or("email_id is required")?;
            let aid = a["attachment_id"].as_u64().ok_or("attachment_id is required")?;
            let e = store.email(id).ok_or("No such email")?;
            let at = e.attachments.iter().find(|x| x.id == aid).ok_or("No such attachment")?;
            format!("{} ({})\n\n{}", at.name, attach::human_size(at.size), attach::read_text(at)?)
        }
        "list_drafts" => {
            let mut s = String::from("ID | To | Subject | Send at\n");
            for d in &store.drafts {
                s.push_str(&format!("{} | {} | {} | {}\n", d.id, d.to, d.subject, d.send_at.map(|t| crate::infrastructure::timezone::format_timestamp(&store.settings, t)).unwrap_or_default()));
            }
            s
        }
        "create_draft" => {
            need(Access::Triage, access)?;
            let mut d = Draft { body: arg_str(a, "body").unwrap_or("").to_string(), updated: now_ts(), ..Default::default() };
            d.account_id = arg_str(a, "account")
                .and_then(|m| store.accounts.iter().find(|x| x.email.eq_ignore_ascii_case(m)).map(|x| x.id))
                .unwrap_or_else(|| store.accounts.first().map(|x| x.id).unwrap_or(1));
            if let Some(r) = a["reply_to"].as_u64() {
                let e = store.email(r).ok_or("reply_to email not found")?;
                d.reply_to = Some(r);
                d.to = e.from_addr.clone();
                d.subject = if e.subject.to_lowercase().starts_with("re:") { e.subject.clone() } else { format!("Re: {}", e.subject) };
                d.account_id = e.account_id;
                d.signature_id = store.signature_for_reply(&e.clone());
            } else {
                d.signature_id = store.signature_for_new(d.account_id);
            }
            if let Some(t) = arg_str(a, "to") {
                d.to = t.into();
            }
            if let Some(t) = arg_str(a, "cc") {
                d.cc = t.into();
            }
            if let Some(t) = arg_str(a, "subject") {
                d.subject = t.into();
            }
            d.id = store.new_id();
            let id = d.id;
            store.drafts.push(d);
            changed = true;
            format!("Draft created. ID: {id}. It is in Qamille → Drafts for review; nothing was sent.")
        }
        "update_draft" => {
            need(Access::Triage, access)?;
            let id = a["id"].as_u64().ok_or("id is required")?;
            let d = store.drafts.iter_mut().find(|d| d.id == id).ok_or("No such draft")?;
            for (k, f) in [("to", &mut d.to), ("cc", &mut d.cc), ("subject", &mut d.subject), ("body", &mut d.body)] {
                if let Some(v) = a[k].as_str() {
                    *f = v.to_string();
                }
            }
            d.updated = now_ts();
            changed = true;
            format!("Draft {id} updated.")
        }
        "delete_draft" => {
            need(Access::Triage, access)?;
            let id = a["id"].as_u64().ok_or("id is required")?;
            let before = store.drafts.len();
            store.drafts.retain(|d| d.id != id);
            if before == store.drafts.len() {
                return Err("No such draft".into());
            }
            changed = true;
            format!("Draft {id} deleted permanently.")
        }
        "send_draft" => {
            need(Access::Send, access)?;
            let id = a["id"].as_u64().ok_or("id is required")?;
            let when = match arg_str(a, "date") {
                Some(t) => crate::infrastructure::timezone::parse_when(&store.settings, t)?,
                None => now_ts(),
            };
            if when < now_ts() - 60 {
                return Err("Date is in the past.".into());
            }
            if when > now_ts() + rules::MAX_DAYS_SEND_LATER * 86_400 {
                return Err(format!("Send Later can be at most {} days ahead.", rules::MAX_DAYS_SEND_LATER));
            }
            let d = store.drafts.iter_mut().find(|d| d.id == id).ok_or("No such draft")?;
            if d.to.trim().is_empty() || d.subject.trim().is_empty() {
                return Err("A draft needs at least one recipient and a subject.".into());
            }
            d.send_at = Some(when);
            changed = true;
            format!("Draft {id} scheduled for {}. Qamille sends it while its window is open.", crate::infrastructure::timezone::format_timestamp(&store.settings, when))
        }
        "email_action" => {
            need(Access::Triage, access)?;
            let action = arg_str(a, "action").ok_or("action is required")?;
            let ids: Vec<u64> = a["ids"].as_array().map(|v| v.iter().filter_map(|x| x.as_u64()).collect()).unwrap_or_default();
            if ids.is_empty() {
                return Err("ids is required".into());
            }
            let mut lines = Vec::new();
            for id in ids {
                lines.push(crate::infrastructure::timezone::apply_email_action(store, id, action, arg_str(a, "value")).unwrap_or_else(|e| e));
            }
            changed = true;
            lines.join("\n")
        }
        "classify_email" => {
            let id = a["id"].as_u64().ok_or("id is required")?;
            let e = store.email(id).ok_or("No such email")?.clone();
            let (cat, labels) = ai::classify(&store.settings, &e, &store.all_labels());
            format!("Category: {}\nLabels: {}", cat.key(), labels.join(", "))
        }
        "generate_reply" => {
            need(Access::Triage, access)?;
            let id = a["id"].as_u64().ok_or("id is required")?;
            let e = store.email(id).ok_or("No such email")?.clone();
            let body = ai::generate_reply(&store.settings, &e, arg_str(a, "instruction").unwrap_or(""));
            let did = store.new_id();
            let signature_id = store.signature_for_reply(&e);
            store.drafts.push(Draft {
                signature_id,
                id: did,
                account_id: e.account_id,
                to: e.from_addr.clone(),
                subject: format!("Re: {}", e.subject.trim_start_matches("Re: ")),
                body: body.clone(),
                reply_to: Some(id),
                updated: now_ts(),
                ..Default::default()
            });
            changed = true;
            format!("Draft {did} saved:\n\n{body}")
        }
        "list_events" => {
            let zone = crate::infrastructure::timezone::zone(&store.settings);
            let today = chrono::Utc::now().with_timezone(&zone).date_naive();
            let from_day = match arg_str(a, "from") {
                Some(value) => NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| "Use YYYY-MM-DD for from")?,
                None => today,
            };
            let until_day = match arg_str(a, "to") {
                Some(value) => NaiveDate::parse_from_str(value, "%Y-%m-%d").map_err(|_| "Use YYYY-MM-DD for to")?.succ_opt().ok_or("End date out of range")?,
                None => from_day.checked_add_signed(chrono::Duration::days(7)).ok_or("End date out of range")?,
            };
            let from = crate::domain::regional::day_start(from_day, zone.name())?;
            let to = crate::domain::regional::day_start(until_day, zone.name())?;
            if to <= from { return Err("The start date must not be after the end date".into()); }
            let mut ev: Vec<&Event> = store.events.iter().filter(|e| e.end > from && e.start < to).collect();
            ev.sort_by_key(|e| e.start);
            let mut s = String::from("ID | Start | End | Title | Location | Link | Attendees\n");
            for e in ev {
                s.push_str(&format!(
                    "{} | {} | {} | {} | {} | {} | {}\n",
                    e.id, crate::infrastructure::timezone::format_timestamp(&store.settings, e.start), crate::infrastructure::timezone::format_timestamp(&store.settings, e.end), e.title, e.location,
                    e.meeting_link.clone().unwrap_or_default(), e.attendees.join(", ")
                ));
            }
            s
        }
        "create_event" => {
            need(Access::Send, access)?;
            let title = arg_str(a, "title").ok_or("title is required")?.to_string();
            let start = crate::infrastructure::timezone::parse_when(&store.settings, arg_str(a, "start").ok_or("start is required")?)?;
            let mins = a["minutes"].as_i64().unwrap_or(30).clamp(5, 1440);
            let id = store.new_id();
            let link = a["online"].as_bool().unwrap_or(false).then(|| meeting_link(&title, id));
            store.events.push(Event {
                id,
                title,
                start,
                end: start + mins * 60,
                location: arg_str(a, "location").unwrap_or("").into(),
                attendees: arg_str(a, "attendees").map(crate::mail::split_addrs).unwrap_or_default(),
                conference: if link.is_some() { "jitsi".into() } else { String::new() },
                meeting_link: link.clone(),
                color: [47, 111, 237],
                ..Default::default()
            });
            changed = true;
            format!("Event {id} created.{}", link.map(|l| format!(" Meeting link: {l}")).unwrap_or_default())
        }
        "find_free_slots" => {
            let day = day_start(arg_str(a, "date").ok_or("date is required")?, store)?;
            let mins = a["minutes"].as_i64().unwrap_or(30).clamp(5, 1440) * 60;
            free_slots(store, day, mins).iter().map(|(s, e)| format!("{} - {}", crate::infrastructure::timezone::format_timestamp(&store.settings, *s), crate::infrastructure::timezone::format_timestamp(&store.settings, *e))).collect::<Vec<_>>().join("\n")
        }
        "list_meetings" => {
            let mut s = String::from("ID | Date | Title | Duration\n");
            for detail in crate::infrastructure::meeting_repository::merged_details(Store::data_dir().join("meetings"), &store.meetings).map_err(|e| e.message)? {
                let m = detail.meeting;
                s.push_str(&format!("{} | {} | {} | {} min\n", m.id, crate::infrastructure::timezone::format_timestamp(&store.settings, m.date), m.title, m.duration_secs / 60));
            }
            s
        }
        "read_meeting" => {
            let id = a["id"].as_u64().ok_or("id is required")?;
            let details = crate::infrastructure::meeting_repository::merged_details(Store::data_dir().join("meetings"), &store.meetings).map_err(|e| e.message)?;
            let detail = details.into_iter().find(|d| d.meeting.id == id).ok_or("No such meeting")?;
            let m = &detail.meeting;
            let mut s = format!(
                "{} — {}\n\nSummary: {}\n\nKey Points:\n{}\n\nAction Items:\n{}\n",
                m.title, crate::infrastructure::timezone::format_timestamp(&store.settings, m.date), m.summary,
                m.key_points.iter().map(|x| format!("- {x}")).collect::<Vec<_>>().join("\n"),
                m.action_items.iter().map(|x| format!("- {x}")).collect::<Vec<_>>().join("\n")
            );
            if a["transcript"].as_bool().unwrap_or(false) {
                s.push_str(&format!("\nTranscript:\n{}\n", m.transcript));
            }
            if a["notes"].as_bool().unwrap_or(false) {
                s.push_str(&format!("\nNotes:\n{}\n", m.notes));
                for note in detail.notes.iter().filter(|note| note.id != "free") {
                    let position = note.at_ms.map(|ms| format!(" [{} ms]", ms)).unwrap_or_default();
                    s.push_str(&format!("- {}{}: {}\n", note.id, position, note.text));
                }
            }
            s
        }
        "search_meetings" => {
            let details = crate::infrastructure::meeting_repository::merged_details(Store::data_dir().join("meetings"), &store.meetings).map_err(|e| e.message)?;
            let hits = crate::application::meetings::search_details(&details, a["query"].as_str().ok_or("query is required")?, 100).map_err(|e| e.message)?;
            serde_json::to_string(&hits).map_err(|e| e.to_string())?
        }
        "read_meeting_segments" => {
            let id = a["id"].as_u64().ok_or("id is required")?;
            let details = crate::infrastructure::meeting_repository::merged_details(Store::data_dir().join("meetings"), &store.meetings).map_err(|e| e.message)?;
            let detail = details.iter().find(|d| d.meeting.id == id).ok_or("No such meeting")?;
            let transcript = a["version"].as_str().and_then(|id| detail.transcripts.iter().find(|t| t.id == id)).or_else(|| if a["version"].is_null() { detail.transcript() } else { None }).ok_or("No timestamped transcript for that version")?;
            let offset = a["offset"].as_u64().unwrap_or(0).min(usize::MAX as u64) as usize;
            let limit = a["limit"].as_u64().unwrap_or(50).clamp(1, 100) as usize;
            let segments: Vec<_> = transcript.segments.iter().skip(offset).take(limit).collect();
            serde_json::to_string(&json!({"version": transcript.id, "segments": segments, "total": transcript.segments.len(), "next_offset": (offset.saturating_add(limit) < transcript.segments.len()).then_some(offset.saturating_add(limit))})).map_err(|e| e.to_string())?
        }
        other => return Err(format!("Unknown tool {other}")),
    };
    Ok((out, changed))
}

pub fn meeting_link(title: &str, id: u64) -> String {
    let slug: String = title
        .to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-");
    format!("https://meet.jit.si/qamille-{}-{}", slug, id ^ (now_ts() as u64 & 0xffff))
}

pub fn free_slots(store: &Store, day: i64, len: i64) -> Vec<(i64, i64)> {
    let zone = crate::infrastructure::timezone::zone(&store.settings);
    let Some(date) = zone.timestamp_opt(day, 0).single().map(|time| time.date_naive()) else { return Vec::new(); };
    let Some(morning) = date.and_hms_opt(9, 0, 0) else { return Vec::new(); };
    let Some(evening) = date.and_hms_opt(20, 0, 0) else { return Vec::new(); };
    let (Ok(start), Ok(end)) = (crate::domain::regional::local_timestamp(morning, zone.name()), crate::domain::regional::local_timestamp(evening, zone.name())) else { return Vec::new(); };
    let mut busy: Vec<(i64, i64)> = store.events.iter().filter(|e| e.end > start && e.start < end).map(|e| (e.start, e.end)).collect();
    busy.sort();
    let mut out = Vec::new();
    let mut t = start.max(now_ts() - now_ts() % 1800 + 1800).min(end);
    if day > now_ts() {
        t = start;
    }
    for (bs, be) in busy {
        if bs - t >= len {
            out.push((t, bs));
        }
        t = t.max(be);
    }
    if end - t >= len {
        out.push((t, end));
    }
    out
}

#[cfg(test)]
mod regional_calendar_tests {
    use super::*;
    fn ts(value: &str) -> i64 { chrono::DateTime::parse_from_rfc3339(value).unwrap().timestamp() }
    #[test]
    fn event_range_includes_the_whole_configured_twenty_five_hour_day() {
        let mut store = Store::default();
        store.settings.time_zone = "Europe/Rome".into();
        store.events = vec![
            Event { id: 1, title: "Inside".into(), start: ts("2026-10-25T22:30:00Z"), end: ts("2026-10-25T22:50:00Z"), ..Default::default() },
            Event { id: 2, title: "After".into(), start: ts("2026-10-25T23:00:00Z"), end: ts("2026-10-26T00:00:00Z"), ..Default::default() },
            Event { id: 3, title: "Before".into(), start: ts("2026-10-24T21:00:00Z"), end: ts("2026-10-24T22:00:00Z"), ..Default::default() },
        ];
        let (out, changed) = call(&mut store, "list_events", &json!({"from":"2026-10-25","to":"2026-10-25"})).unwrap();
        assert!(!changed);
        assert!(out.contains("Inside"));
        assert!(!out.contains("After"));
        assert!(!out.contains("Before"));
    }
    #[test]
    fn event_creation_uses_the_configured_zone_and_rejects_a_gap() {
        let mut store = Store::default();
        store.settings.time_zone = "Europe/Rome".into();
        store.settings.mcp_access = Access::Send;
        assert!(call(&mut store, "create_event", &json!({"title":"Gap","start":"2026-03-29T02:30"})).is_err());
        assert!(store.events.is_empty());
        call(&mut store, "create_event", &json!({"title":"Repeated","start":"2026-10-25T02:30"})).unwrap();
        assert_eq!(store.events[0].start, ts("2026-10-25T00:30:00Z"));
    }
}

/// Runs the stdio loop until stdin closes.
pub fn serve_legacy() {
    crate::infrastructure::compute_factory::configure(
        std::sync::Arc::new(crate::infrastructure::repository::JsonStoreRepository::new(Store::path())),
        std::sync::Arc::new(crate::infrastructure::gateways::KeyringSecrets),
    );
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(req) = serde_json::from_str::<Value>(&line) else {
            let _ = writeln!(stdout, "{}", json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "Parse error"}}));
            let _ = stdout.flush();
            continue;
        };
        let id = req.get("id").cloned();
        let method = req["method"].as_str().unwrap_or("");
        let result: Result<Value, (i64, String)> = match method {
            "initialize" => Ok(json!({
                "protocolVersion": req["params"]["protocolVersion"].as_str().unwrap_or("2025-06-18"),
                "capabilities": {"tools": {"listChanged": false}},
                "serverInfo": {"name": "qamille", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Qamille mail: search and read emails and attachments, manage drafts (never sends unless send_draft and send access), triage with email_action, calendar, meeting notes."
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({"tools": tools()})),
            "tools/call" => {
                let name = req["params"]["name"].as_str().unwrap_or("");
                let args = req["params"]["arguments"].clone();
                use crate::application::ports::StoreRepository;
                let repository = crate::infrastructure::repository::JsonStoreRepository::new(Store::path());
                // Legacy tools retain their access checks, but read-modify-write
                // now shares the repository lock with the desktop services.
                let mutation = matches!(name, "create_draft" | "update_draft" | "delete_draft" | "send_draft" | "email_action" | "classify_email" | "generate_reply" | "create_event");
                let result = if mutation {
                    let mut output = None;
                    let committed = repository.transaction(&mut |store| {
                        store.load_secrets();
                        output = Some(call(store, name, &args).map_err(crate::application::AppError::validation)?);
                        Ok(())
                    });
                    committed.map_err(|e| e.message).and_then(|()| output.ok_or_else(|| "No tool result".to_string()))
                } else {
                    repository.read().map_err(|e| e.message).and_then(|mut store| { store.load_secrets(); call(&mut store, name, &args) })
                };
                match result {
                    Ok((text, changed)) => {
                        let _ = changed;
                        Ok(json!({"content": [{"type": "text", "text": text}], "isError": false}))
                    }
                    Err(e) => Ok(json!({"content": [{"type": "text", "text": e}], "isError": true})),
                }
            }
            m if m.starts_with("notifications/") => continue,
            _ => Err((-32601, format!("Method not found: {method}"))),
        };
        if id.is_none() {
            continue;
        }
        let resp = match result {
            Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
            Err((code, msg)) => json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": msg}}),
        };
        let _ = writeln!(stdout, "{resp}");
        let _ = stdout.flush();
    }
}
