//! MCP adapters for existing application use cases. Network work stays outside transactions.
use super::files;
use crate::application::{
    AppError, AppResult, ServiceContext,
    drafts::DraftService,
    mail::{ExportFormat, MailQuery, MailService},
    ports::AssistantGateway,
};
use crate::domain::activity::{ActivityKind, ActivitySource, LOGGED_MAIL_ACTIONS, NewActivity};
use crate::domain::meeting::*;
use crate::model::{Access, BodyFormat, Draft, Signature, Store};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{File, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

fn s() -> Value {
    json!({"type":"string"})
}
fn id_schema() -> Value {
    json!({"anyOf":[{"type":"string","pattern":"^[0-9]+$"},{"type":"integer","minimum":1}]})
}
fn integer() -> Value {
    json!({"type":"integer","minimum":0})
}
fn boolean() -> Value {
    json!({"type":"boolean"})
}
fn addresses_schema() -> Value {
    json!({"anyOf":[{"type":"string"},{"type":"array","items":{"type":"string"}}]})
}
fn tool(name: &str, description: &str, props: Value, required: &[&str], access: Access) -> Value {
    let mut p = props.as_object().cloned().unwrap_or_default();
    p.insert("account_id".into(), id_schema());
    p.insert("cursor".into(), s());
    p.insert(
        "limit".into(),
        json!({"type":"integer","minimum":1,"maximum":100}),
    );
    json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":p,"required":required,"additionalProperties":false},"outputSchema":{"type":"object","properties":{"data":{},"meta":{"type":"object"}},"required":["data","meta"]},"annotations":{"readOnlyHint":access==Access::ReadOnly,"destructiveHint":matches!(name,"delete_draft"|"delete_signature"),"openWorldHint":true}})
}
pub fn catalog() -> Vec<Value> {
    let r = Access::ReadOnly;
    let w = Access::Triage;
    vec![
        tool(
            "get_server_info",
            "Read server version, effective account count and regional settings. No credentials or permission changes.",
            json!({}),
            &[],
            r,
        ),
        tool(
            "list_accounts",
            "List accounts, effective mail capabilities and synchronization state. Never returns credentials.",
            json!({}),
            &[],
            r,
        ),
        tool(
            "get_account_status",
            "Read one account's connection, retention, and mail capabilities.",
            json!({}),
            &["account_id"],
            r,
        ),
        tool(
            "sync_accounts",
            "Synchronize authorized accounts; result contains success/error for each account.",
            json!({}),
            &[],
            w,
        ),
        tool(
            "list_folders",
            "List locally observed mail folders and counts; coverage is explicit.",
            json!({}),
            &[],
            r,
        ),
        tool(
            "list_labels",
            "List observed message labels and locally created label names.",
            json!({"scope":{"enum":["local","server"]}}),
            &[],
            r,
        ),
        tool(
            "manage_label",
            "Create, rename or delete provider labels; explicit local scope is also available. Deleting a label never deletes its messages.",
            json!({"action":{"enum":["create","rename","delete"]},"name":s(),"new_name":s(),"scope":{"enum":["local","server"]},"expected_revision":{"anyOf":[integer(),s()]},"idempotency_key":s()}),
            &["action", "name", "account_id"],
            w,
        ),
        tool(
            "search_contacts",
            "Search the unified address book: local and Google contacts, plus people seen in mail. Each result lists its sources.",
            json!({"query":s()}),
            &[],
            r,
        ),
        tool(
            "list_address_books",
            "List the address books (local, Google, mail-observed) with whether each can be written and when it last synchronized.",
            json!({}),
            &[],
            r,
        ),
        tool(
            "get_contact",
            "Read one contact of the unified address book with all its sources.",
            json!({"id":s()}),
            &["id"],
            r,
        ),
        tool(
            "find_duplicates",
            "List groups of contacts that may be the same person. Read-only: merging is done by the user in the app.",
            json!({"include_possible":boolean()}),
            &[],
            r,
        ),
        tool(
            "create_contact",
            "Create a contact in a chosen address book (local or a Google book already allowed to write). The book is required.",
            json!({"book":s(),"name":s(),"org":s(),"title":s(),"birthday":s(),"notes":s(),"emails":{"type":"array","items":s()},"phones":{"type":"array","items":s()}}),
            &["book"],
            w,
        ),
        tool(
            "update_contact",
            "Edit a contact in every writable address book that holds it. Only the fields you pass change.",
            json!({"id":s(),"name":s(),"org":s(),"title":s(),"birthday":s(),"notes":s(),"emails":{"type":"array","items":s()},"phones":{"type":"array","items":s()}}),
            &["id"],
            w,
        ),
        tool(
            "link_contacts",
            "Mark cards as the same person in the unified address book. Writes to no address book and deletes nothing.",
            json!({"keys":{"type":"array","items":s(),"minItems":2}}),
            &["keys"],
            w,
        ),
        tool(
            "list_signatures",
            "List composer signatures.",
            json!({}),
            &[],
            r,
        ),
        tool(
            "save_signature",
            "Create or update a composer signature with optimistic revision control.",
            json!({"id":id_schema(),"name":s(),"body":s(),"html":boolean(),"expected_revision":integer(),"idempotency_key":s()}),
            &["name", "body"],
            w,
        ),
        tool(
            "delete_signature",
            "Delete a signature after verifying its revision.",
            json!({"id":id_schema(),"expected_revision":integer()}),
            &["id", "expected_revision"],
            w,
        ),
        tool(
            "list_emails",
            "List mail with shared validated filters, account isolation, and stable cursor pagination.",
            json!({"folder":s(),"filter":s(),"query":s(),"page":integer(),"page_size":integer(),"sort":s(),"threads":boolean()}),
            &[],
            r,
        ),
        tool(
            "search_emails",
            "Search local/server/auto mail with from:, to:, cc:, subject:, body:, dates, labels, OR, exclusions, and parentheses. Returns actual coverage.",
            json!({"query":s(),"folder":s(),"scope":{"enum":["local","server","auto"]},"page":integer(),"page_size":integer()}),
            &["query"],
            r,
        ),
        tool(
            "semantic_search",
            "Search the local semantic index. Coverage is limited to indexed content; citations are retrieved from current sources.",
            json!({"query":s()}),
            &["query"],
            r,
        ),
        tool(
            "read_email",
            "Read a message without marking it read. Body is paginated. include_thread retains the legacy local-conversation view when requested.",
            json!({"id":id_schema(),"include_thread":boolean(),"format":{"enum":["text","html","headers"]},"offset":integer(),"max_chars":{"type":"integer","minimum":1,"maximum":16000}}),
            &["id"],
            r,
        ),
        tool(
            "read_thread",
            "Read a full provider conversation or explicitly request locally available members.",
            json!({"id":id_schema(),"scope":{"enum":["local","server"]}}),
            &["id"],
            r,
        ),
        tool(
            "list_attachments",
            "List attachment metadata without exposing internal paths.",
            json!({"email_id":id_schema()}),
            &["email_id"],
            r,
        ),
        tool(
            "read_attachment",
            "Read extracted attachment text with pagination; unsupported extraction is an explicit error.",
            json!({"email_id":id_schema(),"attachment_id":id_schema(),"offset":integer(),"max_chars":{"type":"integer","minimum":1,"maximum":16000}}),
            &["email_id", "attachment_id"],
            r,
        ),
        tool(
            "download_attachment",
            "Copy an attachment to managed storage and return a readable file handle.",
            json!({"email_id":id_schema(),"attachment_id":id_schema()}),
            &["email_id", "attachment_id"],
            r,
        ),
        tool(
            "export_emails",
            "Export exact original EML or MBOX to a managed file; no messages marked read.",
            json!({"ids":{"type":"array","items":id_schema(),"minItems":1,"maxItems":100},"format":{"enum":["eml","mbox"]}}),
            &["ids", "format"],
            r,
        ),
        tool(
            "email_action",
            "Apply a validated message action with individual batch outcomes. Remote changes use the same service as desktop.",
            json!({"action":{"enum":crate::actions::ACTIONS},"ids":{"type":"array","items":id_schema(),"minItems":1,"maxItems":100},"value":s(),"scope":{"enum":["message","thread"]}}),
            &["action", "ids"],
            w,
        ),
        tool(
            "undo_operation",
            "Undo a pending desktop/MCP Undo action before its deadline; cannot recall sent mail.",
            json!({"id":id_schema()}),
            &["id"],
            w,
        ),
        tool(
            "list_drafts",
            "List local/remote-linked drafts, revisions and scheduling state.",
            json!({}),
            &[],
            r,
        ),
        tool(
            "read_draft",
            "Read a draft including paginated body/quote and expected_revision.",
            json!({"id":id_schema(),"offset":integer(),"max_chars":{"type":"integer","minimum":1,"maximum":16000}}),
            &["id"],
            r,
        ),
        tool(
            "create_draft",
            "Create a recoverable draft; never sends. Uses an explicit account, reply source, or user-configured default account. Supports reply/reply_all/forward.",
            json!({"account":s(),"to":addresses_schema(),"cc":addresses_schema(),"bcc":addresses_schema(),"subject":s(),"body":s(),"format":{"enum":["text","html"]},"mode":{"enum":["new","reply","reply_all","forward"]},"reply_to":id_schema(),"signature_id":id_schema(),"idempotency_key":s()}),
            &["body", "idempotency_key"],
            w,
        ),
        tool(
            "update_draft",
            "Patch an existing draft; omitted fields remain unchanged and stale revisions are rejected.",
            json!({"id":id_schema(),"to":addresses_schema(),"cc":addresses_schema(),"bcc":addresses_schema(),"subject":s(),"body":s(),"format":{"enum":["text","html"]},"signature_id":{"anyOf":[id_schema(),{"type":"null"}]},"expected_revision":integer()}),
            &["id", "expected_revision"],
            w,
        ),
        tool(
            "delete_draft",
            "Remove a draft after revision verification; queued drafts must be cancelled first.",
            json!({"id":id_schema(),"expected_revision":integer()}),
            &["id", "expected_revision"],
            w,
        ),
        tool(
            "attach_to_draft",
            "Attach a managed/registered file snapshot to a draft.",
            json!({"id":id_schema(),"handle":s(),"expected_revision":integer()}),
            &["id", "handle", "expected_revision"],
            w,
        ),
        tool(
            "remove_draft_attachment",
            "Remove an attachment by index with revision verification.",
            json!({"id":id_schema(),"index":integer(),"expected_revision":integer()}),
            &["id", "index", "expected_revision"],
            w,
        ),
        tool(
            "classify_email",
            "Suggest category and labels using configured AI; never applies changes.",
            json!({"id":id_schema()}),
            &["id"],
            r,
        ),
        tool(
            "generate_reply",
            "Generate a reply and save it as a draft, never send. Requires an idempotency key.",
            json!({"id":id_schema(),"instruction":s(),"idempotency_key":s()}),
            &["id", "idempotency_key"],
            w,
        ),
        tool(
            "get_preferences",
            "Read safe regional/composer preferences without credentials or MCP grants.",
            json!({}),
            &[],
            r,
        ),
        tool(
            "update_preferences",
            "Update regional settings, working hours, weekdays and default account/calendar with a recoverable transaction. MCP grants cannot be changed.",
            json!({"user_name":s(),"time_zone":s(),"locale":s(),"undo_seconds":{"type":"integer","minimum":5,"maximum":30},"working_hours":{"type":"object","properties":{"start":s(),"end":s(),"weekdays":{"type":"array","items":{"type":"integer","minimum":1,"maximum":7},"minItems":1,"maxItems":7}},"additionalProperties":false},"default_account_id":{"anyOf":[id_schema(),{"type":"null"}]},"default_calendar_id":s(),"expected_revision":s()}),
            &["expected_revision"],
            w,
        ),
        tool(
            "register_file",
            "Snapshot a file from host-configured QAMILLE_MCP_FILE_ROOTS into managed storage. Paths outside those roots are denied.",
            json!({"path":s()}),
            &["path"],
            w,
        ),
        tool(
            "read_file",
            "Read a managed file as bounded base64 chunks.",
            json!({"handle":s(),"offset":integer(),"bytes":{"type":"integer","minimum":1,"maximum":49152}}),
            &["handle"],
            r,
        ),
        tool(
            "list_meetings",
            "List saved meetings with content excluded from summaries and stable pagination.",
            json!({}),
            &[],
            r,
        ),
        tool(
            "search_meetings",
            "Search included meeting transcripts/notes/summaries with versioned sources.",
            json!({"query":s()}),
            &["query"],
            r,
        ),
        tool(
            "read_meeting",
            "Read saved meeting summary, actions, notes, revision and jobs. Transcript requires read_meeting_segments.",
            json!({"id":id_schema(),"transcript":boolean(),"notes":boolean()}),
            &["id"],
            r,
        ),
        tool(
            "read_meeting_segments",
            "Read timestamped transcript segments from a selected preserved version.",
            json!({"id":id_schema(),"version":s(),"offset":integer()}),
            &["id"],
            r,
        ),
        tool(
            "update_meeting_notes",
            "Save meeting notes with revision verification.",
            json!({"id":id_schema(),"notes":s(),"expected_revision":integer()}),
            &["id", "notes", "expected_revision"],
            w,
        ),
        tool(
            "import_meeting",
            "Import a managed audio file as a saved meeting with a processing job. No microphone initialization.",
            json!({"handle":s(),"idempotency_key":s()}),
            &["handle", "idempotency_key"],
            w,
        ),
        tool(
            "transcribe_meeting",
            "Start a saved-audio transcription job using an already configured model; no microphone access. provider: compute (the compute server, default) or external (the OpenAI-compatible endpoint); local and remote are their former names.",
            json!({"id":id_schema(),"provider":{"enum":["compute","external","local","remote"]},"model":s(),"language":s(),"expected_revision":integer(),"idempotency_key":s()}),
            &["id", "expected_revision", "idempotency_key"],
            w,
        ),
        tool(
            "summarize_meeting",
            "Start a source-linked meeting summary job using configured AI.",
            json!({"id":id_schema(),"language":s(),"expected_revision":integer(),"idempotency_key":s()}),
            &["id", "expected_revision", "idempotency_key"],
            w,
        ),
        tool(
            "get_meeting_job",
            "Read a durable meeting processing job.",
            json!({"id":id_schema(),"job_id":s()}),
            &["id", "job_id"],
            r,
        ),
        tool(
            "cancel_meeting_job",
            "Cancel a meeting job owned by this runtime; cross-process work is refused.",
            json!({"id":id_schema(),"job_id":s()}),
            &["id", "job_id"],
            w,
        ),
        tool(
            "export_meeting",
            "Export selected saved meeting content as Markdown/TXT/JSON to a managed file.",
            json!({"id":id_schema(),"format":{"enum":["markdown","txt","json"]},"version":s()}),
            &["id"],
            r,
        ),
        tool(
            "list_tasks",
            "List persistent local tasks linked to email or meeting sources.",
            json!({"completed":boolean()}),
            &[],
            r,
        ),
        tool(
            "create_task_from_source",
            "Create a local task from a real email or saved meeting, keeping the original source reference.",
            json!({"source_type":{"enum":["email","meeting"]},"source_id":id_schema(),"title":s(),"due":{"anyOf":[s(),{"type":"null"}]},"idempotency_key":s()}),
            &["source_type", "source_id", "idempotency_key"],
            w,
        ),
        tool(
            "update_task",
            "Patch a local task with optimistic revision checking.",
            json!({"id":s(),"title":s(),"due":{"anyOf":[s(),{"type":"null"}]},"expected_revision":integer()}),
            &["id", "expected_revision"],
            w,
        ),
        tool(
            "complete_task",
            "Mark a local task complete or reopen it with revision checking.",
            json!({"id":s(),"completed":boolean(),"expected_revision":integer()}),
            &["id", "expected_revision"],
            w,
        ),
        tool(
            "list_activity",
            "Read local task/label operation metadata, without message bodies or secrets.",
            json!({}),
            &[],
            r,
        ),
    ]
}
pub fn handles(name: &str) -> bool {
    catalog().iter().any(|t| t["name"] == name)
}
pub fn required_access(name: &str) -> Access {
    if catalog()
        .iter()
        .find(|t| t["name"] == name)
        .is_some_and(|t| t["annotations"]["readOnlyHint"] == true)
    {
        Access::ReadOnly
    } else {
        Access::Triage
    }
}
fn check_access(context: &ServiceContext, name: &str) -> AppResult<()> {
    let have = context.read()?.settings.mcp_access;
    if required_access(name) != Access::ReadOnly && have == Access::ReadOnly {
        return Err(AppError::new(
            "permission_denied",
            "This operation requires organization access",
        ));
    }
    Ok(())
}
fn arg<'a>(a: &'a Value, k: &str) -> Option<&'a str> {
    a.get(k).and_then(Value::as_str)
}
fn required<'a>(a: &'a Value, k: &str) -> AppResult<&'a str> {
    arg(a, k)
        .filter(|s| !s.trim().is_empty())
        .ok_or_else(|| AppError::validation(format!("{k} is required")))
}
fn number(a: &Value, k: &str) -> AppResult<u64> {
    a.get(k)
        .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
        .filter(|v| *v > 0)
        .ok_or_else(|| AppError::validation(format!("{k} must be a positive ID")))
}
fn account(a: &Value) -> AppResult<Option<u32>> {
    a.get("account_id")
        .map(|_| {
            number(a, "account_id").and_then(|n| {
                u32::try_from(n).map_err(|_| AppError::validation("Invalid account ID"))
            })
        })
        .transpose()
}
fn revision(a: &Value) -> AppResult<u64> {
    a["expected_revision"].as_u64().ok_or_else(|| {
        AppError::validation("expected_revision is required; read the resource first")
    })
}
fn encode<T: Serialize>(value: T) -> AppResult<Value> {
    Ok(serde_json::to_value(value)?)
}
fn normalize(v: &mut Value) {
    match v {
        Value::Object(m) => {
            m.remove("path");
            m.remove("audio_path");
            m.remove("password");
            for (k, v) in m {
                if (k == "id" || k.ends_with("_id")) && v.is_number() {
                    *v = Value::String(v.to_string());
                } else {
                    normalize(v)
                }
            }
        }
        Value::Array(a) => {
            for v in a {
                normalize(v)
            }
        }
        _ => {}
    }
}
fn output(mut data: Value) -> Value {
    normalize(&mut data);
    json!({"data":data,"meta":{"updated_at":crate::model::now_ts()}})
}
fn hash(v: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(v).unwrap_or_default())
    )
}
fn page(items: Vec<Value>, a: &Value, scope: &str) -> AppResult<Value> {
    let mut q = a.clone();
    if let Some(m) = q.as_object_mut() {
        for k in ["cursor", "page", "page_size", "limit"] {
            m.remove(k);
        }
    }
    let fingerprint = hash(&json!([scope, q, items]));
    let limit = a["limit"]
        .as_u64()
        .or_else(|| a["page_size"].as_u64())
        .unwrap_or(20)
        .clamp(1, 100) as usize;
    let offset = if let Some(cursor) = arg(a, "cursor") {
        let parts: Vec<_> = cursor.split(':').collect();
        if parts.len() != 3 || parts[0] != "v1" || parts[2] != fingerprint {
            return Err(AppError::new(
                "cursor_expired",
                "The result set or query changed. Restart from its first page",
            ));
        }
        parts[1]
            .parse::<usize>()
            .map_err(|_| AppError::validation("Invalid cursor"))?
    } else {
        a["page"]
            .as_u64()
            .unwrap_or(1)
            .max(1)
            .saturating_sub(1)
            .saturating_mul(limit as u64)
            .min(usize::MAX as u64) as usize
    };
    let total = items.len();
    if offset > total {
        return Err(AppError::validation("Cursor is beyond the result set"));
    }
    let next = offset.saturating_add(limit);
    let selected: Vec<_> = items.into_iter().skip(offset).take(limit).collect();
    Ok(output(
        json!({"items":selected,"total":total,"total_kind":"exact","has_more":next<total,"next_cursor":(next<total).then(||format!("v1:{next}:{fingerprint}"))}),
    ))
}
fn text_page(text: &str, a: &Value) -> AppResult<Value> {
    let offset = a["offset"].as_u64().unwrap_or(0) as usize;
    let limit = a["max_chars"].as_u64().unwrap_or(16000).clamp(1, 16000) as usize;
    let total = text.chars().count();
    if offset > total {
        return Err(AppError::validation("Text offset exceeds content"));
    }
    let text: String = text.chars().skip(offset).take(limit).collect();
    let next = offset + text.chars().count();
    Ok(
        json!({"text":text,"offset":offset,"total_chars":total,"next_offset":(next<total).then_some(next),"truncated":next<total}),
    )
}
fn ids(a: &Value) -> AppResult<Vec<u64>> {
    let values = a["ids"]
        .as_array()
        .ok_or_else(|| AppError::validation("ids is required"))?;
    if values.is_empty() || values.len() > 100 {
        return Err(AppError::validation("Select 1–100 messages"));
    }
    values
        .iter()
        .map(|v| number(&json!({"id":v}), "id"))
        .collect()
}
fn get_draft(context: &ServiceContext, a: &Value) -> AppResult<Draft> {
    let id = number(a, "id")?;
    let d = context
        .read()?
        .drafts
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| AppError::missing("Draft"))?;
    if account(a)?.is_some_and(|id| id != d.account_id) {
        return Err(AppError::missing("Draft"));
    }
    Ok(d)
}
fn ensure_revision(d: &Draft, a: &Value) -> AppResult<()> {
    if d.updated.max(0) as u64 != revision(a)? {
        return Err(AppError::new(
            "revision_conflict",
            format!("Draft changed; current revision is {}", d.updated),
        ));
    }
    Ok(())
}
fn recipients(a: &Value, k: &str) -> AppResult<Option<String>> {
    match a.get(k) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(Value::Array(v)) => v
            .iter()
            .map(|v| {
                v.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| AppError::validation("Addresses must be strings"))
            })
            .collect::<AppResult<Vec<_>>>()
            .map(|v| Some(v.join(", "))),
        _ => Err(AppError::validation(format!("Invalid {k}"))),
    }
}
fn patch(d: &mut Draft, a: &Value) -> AppResult<()> {
    for k in ["to", "cc", "bcc"] {
        if let Some(v) = recipients(a, k)? {
            match k {
                "to" => d.to = v,
                "cc" => d.cc = v,
                _ => d.bcc = v,
            }
        }
    }
    if let Some(s) = arg(a, "body") {
        d.body = s.into();
    }
    if let Some(s) = arg(a, "subject") {
        d.subject = s.into();
    }
    if let Some(s) = arg(a, "format") {
        d.format = match s {
            "text" => BodyFormat::Plain,
            "html" => BodyFormat::Html,
            _ => return Err(AppError::validation("Use text or html format")),
        };
    }
    if a.get("signature_id").is_some() {
        d.signature_id = if a["signature_id"].is_null() {
            None
        } else {
            Some(number(a, "signature_id")?)
        };
    }
    Ok(())
}

type MailCache = Mutex<HashMap<usize, (Arc<ServiceContext>, MailService)>>;
fn mail(context: Arc<ServiceContext>) -> AppResult<MailService> {
    static CACHE: OnceLock<MailCache> = OnceLock::new();
    let key = Arc::as_ptr(&context) as usize;
    let mut c = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| AppError::storage("Mail runtime unavailable"))?;
    Ok(c.entry(key)
        .or_insert_with(|| (context.clone(), MailService::new(context)))
        .1
        .clone())
}

pub fn call(context: Arc<ServiceContext>, name: &str, a: &Value) -> AppResult<Value> {
    check_access(&context, name)?;
    if !handles(name) {
        return Err(AppError::new(
            "unknown_tool",
            format!("Unknown tool {name}"),
        ));
    }
    let mail = mail(context.clone())?;
    let drafts = DraftService::new(context.clone());
    match name {
      "get_server_info"=>{let store=context.read()?;Ok(output(json!({"name":"qamille","version":env!("CARGO_PKG_VERSION"),"protocol_versions":["2026-07-28","2025-11-25","2025-06-18"],"accounts":store.accounts.len(),"regional":context.clock.regional(&store.settings),"access":store.settings.mcp_access.label(),"recording_available":false})))},
      "list_accounts"|"get_account_status"=>{let store=context.read()?;let requested=account(a)?;if name=="get_account_status"&&requested.is_none(){return Err(AppError::validation("account_id required"));}let data:Vec<_>=store.accounts.iter().filter(|v|requested.is_none_or(|id|id==v.id)).map(|v|{let capabilities=context.mail.capabilities(v);json!({"id":v.id,"name":v.name,"email":v.email,"provider_id":capabilities.provider_id,"last_sync":v.last_sync,"offline_complete":v.offline_sync_complete,"offline_error":v.offline_sync_error,"capabilities":capabilities,"access":store.settings.mcp_access.label()})}).collect();if name=="get_account_status"&&data.is_empty(){return Err(AppError::missing("Account"));}page(data,a,name)}
      "sync_accounts"=>{let store=context.read()?;let requested=account(a)?;if requested.is_some_and(|id|store.account(id).is_none()){return Err(AppError::missing("Account"));}let results:Vec<_>=store.accounts.iter().filter(|v|requested.is_none_or(|id|id==v.id)).map(|v|match mail.sync_account(v.id){Ok(report)=>json!({"account_id":v.id,"status":"completed","report":report}),Err(e)=>json!({"account_id":v.id,"status":"failed","error":e})}).collect();Ok(output(json!({"items":results})))}
      "list_folders"=>{let store=context.read()?;let requested=account(a)?;let mut counts=BTreeMap::<(u32,String),(usize,usize)>::new();for e in store.emails.iter().filter(|e|requested.is_none_or(|id|id==e.account_id)){let n=counts.entry((e.account_id,e.folder.clone())).or_default();n.0+=1;if !e.read{n.1+=1;}}page(counts.into_iter().map(|((id,name),(count,unread))|json!({"account_id":id,"name":name,"count":count,"unread":unread,"coverage":"local_observed"})).collect(),a,name)}
      "list_labels"=>list_labels(context.clone(),a),
      "manage_label"=>manage_label(&context,a),
      "search_contacts"|"list_address_books"|"get_contact"|"find_duplicates"|"create_contact"|"update_contact"|"link_contacts"=>{let v=super::contacts::call(&context,name,a)?;match v.get("rows").and_then(|r|r.as_array()).cloned(){Some(rows)=>page(rows,a,name),None=>Ok(output(v["one"].clone()))}}
      "list_signatures"=>page(context.read()?.signatures.into_iter().map(|s|json!({"id":s.id,"name":s.name,"body":s.body,"html":s.html,"revision":s.modified})).collect(),a,name),
      "save_signature"=>save_signature(context.clone(),a),
      "delete_signature"=>{let id=number(a,"id")?;context.update(|store|{let v=store.signatures.iter().find(|v|v.id==id).ok_or_else(||AppError::missing("Signature"))?;if v.modified.max(0) as u64!=revision(a)?{return Err(AppError::new("revision_conflict","Signature changed"));}store.remove_signature(id);Ok(())})?;Ok(output(json!({"id":id,"deleted":true})))}
      "list_emails"|"search_emails"=>search_mail(context.clone(), &mail, name, a),
      "semantic_search"=>{let store=context.read()?;let gateway=crate::infrastructure::gateways::LocalSearch::new(context.compute.clone());let (hits,unavailable)=gateway.search_reported(&store,required(a,"query")?,100)?;let mut result=Vec::new();for hit in hits{let current=mail.read(hit.email_id)?;result.push(json!({"email_id":current.id,"account_id":current.account_id,"subject":current.subject,"score":hit.score,"passage":if current.remote_only{current.snippet(1800)}else{hit.passage},"source_verified":true}));}let mut out=page(result,a,name)?;out["meta"]["coverage"]=json!("indexed_content_only");if let Some(error)=unavailable{out["meta"]["semantic"]=json!("unavailable");out["meta"]["notice"]=json!(crate::application::compute::mcp_semantic_notice(error.code));}Ok(out)}
      "read_email"=>read_email(&mail, a),
      "read_thread"=>{let id=number(a,"id")?;check_email_account(&mail.read(id)?,a)?;let members=if arg(a,"scope")==Some("local"){mail.conversation(id)?}else{mail.thread(id)?};page(members.into_iter().map(|e|json!({"id":e.id,"account_id":e.account_id,"date":e.date,"subject":e.subject,"from":e.from_addr,"snippet":e.snippet(500),"body_available":!e.remote_only})).collect(),a,name)}
      "list_attachments"|"read_attachment"|"download_attachment"=>{let id=number(a,"email_id")?;let email=mail.read(id)?;check_email_account(&email,a)?;if name=="list_attachments"{return page(email.attachments.into_iter().map(|v|serde_json::to_value(v).unwrap_or(Value::Null)).collect(),a,name);}let aid=number(a,"attachment_id")?;let attachment=email.attachments.iter().find(|v|v.id==aid).ok_or_else(||AppError::missing("Attachment"))?;if name=="read_attachment"{return Ok(output(text_page(&mail.attachment_text(id,aid)?,a)?));}let bytes=context.attachments.read_bytes(attachment)?;Ok(output(files::write_scoped(&attachment.name,"application/octet-stream",&bytes,&[email.account_id])?))}
      "export_emails"=>{let selected=ids(a)?;for id in &selected{check_email_account(&mail.read(*id)?,a)?;}let f=match required(a,"format")?{"eml"=>ExportFormat::Eml,"mbox"=>ExportFormat::Mbox,_=>return Err(AppError::validation("Use eml or mbox"))};let e=mail.export(&selected,f)?;let accounts:Vec<_>=context.read()?.emails.iter().filter(|e|selected.contains(&e.id)).map(|e|e.account_id).collect();let mut v=files::write_scoped(&e.filename,"application/octet-stream",&e.bytes,&accounts)?;v["count"]=json!(e.count);Ok(output(v))}
      "email_action"=>{let action=required(a,"action")?;if !crate::actions::ACTIONS.contains(&action){return Err(AppError::validation("Unknown email action"));}let mut selected=ids(a)?;for id in selected.clone(){let email=mail.read(id)?;check_email_account(&email,a)?;if arg(a,"scope")==Some("thread"){selected.extend(mail.conversation(id)?.into_iter().map(|e|e.id));}}selected.sort_unstable();selected.dedup();if selected.len()>100{return Err(AppError::validation("Expanded conversation exceeds 100 messages"));}let mut results=Vec::new();let mut succeeded=Vec::new();for id in selected{let info=mail.read(id).ok().map(|e|(e.account_id,e.subject.clone()));match mail.act(id,action,arg(a,"value")){Ok(())=>{if let Some((account,subject))=info{succeeded.push((id,account,subject));}results.push(json!({"id":id,"status":"succeeded"}))}Err(e)=>results.push(json!({"id":id,"status":"failed","error":e}))}}if let Some(entry)=assistant_mail_activity(action,&succeeded){crate::application::activity::record(&context,entry);}Ok(output(json!({"items":results})))}
      "undo_operation"=>{let undo=crate::application::undo::UndoService::new(context.clone(),mail,drafts);Ok(output(encode(undo.undo(number(a,"id")?)?)?))}
      "list_drafts"=>{let requested=account(a)?;page(drafts.list()?.into_iter().filter(|d|requested.is_none_or(|id|id==d.account_id)).map(|d|json!({"id":d.id,"account_id":d.account_id,"to":d.to,"subject":d.subject,"revision":d.updated,"send_at":d.send_at,"remote_id":d.remote_id,"attachment_count":d.attachments.len()})).collect(),a,name)},
      "read_draft"=>{let d=get_draft(&context,a)?;let mut value=draft_value(&d)?;value["body"]=text_page(&d.body,a)?;value["quote"]=text_page(&d.quote,a)?;Ok(output(value))}
      "create_draft"=>Ok(output(draft_value(&create_draft(context.clone(),a)?)?)),
      "update_draft"|"attach_to_draft"|"remove_draft_attachment"=>{let mut d=get_draft(&context,a)?;ensure_revision(&d,a)?;if name=="attach_to_draft"{let visible:Vec<_>=context.read()?.accounts.iter().map(|a|a.id).collect();let p=files::resolve_scoped(required(a,"handle")?,Some(&visible))?;if !d.attachments.contains(&p.to_string_lossy().to_string()){d.attachments.push(p.to_string_lossy().into());}let total=context.attachments.total_size(&d.attachments)?;if total>crate::rules::ATTACHMENT_CAP_BYTES{return Err(AppError::validation("Attachments exceed 25 MiB"));}}else if name=="remove_draft_attachment"{let index=a["index"].as_u64().ok_or_else(||AppError::validation("index required"))? as usize;if index>=d.attachments.len(){return Err(AppError::missing("Attachment index"));}d.attachments.remove(index);}else{patch(&mut d,a)?;}let d=drafts.save(d)?;Ok(output(draft_value(&d)?))}
      "delete_draft"=>{let d=get_draft(&context,a)?;ensure_revision(&d,a)?;drafts.discard_revision(d.id,d.updated)?;crate::application::activity::record(&context,assistant_draft_activity(&d));Ok(output(json!({"deleted":true,"id":d.id})))}
      "classify_email"=>{let e=mail.read(number(a,"id")?)?;check_email_account(&e,a)?;let store=context.read()?;let gateway=crate::infrastructure::gateways::ConfiguredAssistant;let (category,labels)=gateway.classify(&context.settings(&store)?,&e,&store.all_labels())?;Ok(output(json!({"category":category.key(),"labels":labels,"applied":false})))}
      "generate_reply"=>{let id=number(a,"id")?;let email=mail.read(id)?;check_email_account(&email,a)?;let args=json!({"reply_to":id,"body":"","mode":"reply","_generation_instruction":arg(a,"instruction"),"idempotency_key":required(a,"idempotency_key")?});let mut d=create_draft(context.clone(),&args)?;if d.body.is_empty(){let store=context.read()?;let gateway=crate::infrastructure::gateways::ConfiguredAssistant;d.body=gateway.reply(&context.settings(&store)?,&email,arg(a,"instruction").unwrap_or(""))?;d=drafts.save(d)?;}Ok(output(draft_value(&d)?))}
      "get_preferences"=>Ok(output(read_preferences(&context)?)),
      "update_preferences"=>Ok(output(update_preferences(&context,a)?)),
      "register_file"=>Ok(output(files::register(Path::new(required(a,"path")?))?)),
      "read_file"=>{let visible:Vec<_>=context.read()?.accounts.iter().map(|a|a.id).collect();Ok(output(files::read_scoped(required(a,"handle")?,a["offset"].as_u64().unwrap_or(0),a["bytes"].as_u64().unwrap_or(49152) as usize,Some(&visible))?))},
      "list_meetings"|"search_meetings"|"read_meeting"|"read_meeting_segments"|"update_meeting_notes"|"import_meeting"|"transcribe_meeting"|"summarize_meeting"|"export_meeting"|"get_meeting_job"|"cancel_meeting_job"=>meeting_call(context,name,a),
      "list_tasks"|"create_task_from_source"|"update_task"|"complete_task"|"list_activity"=>task_call(context,name,a),
      _=>Err(AppError::new("unknown_tool",name))
    }
}
fn check_email_account(e: &crate::model::Email, a: &Value) -> AppResult<()> {
    if account(a)?.is_some_and(|id| id != e.account_id) {
        Err(AppError::missing("Message"))
    } else {
        Ok(())
    }
}
fn search_mail(
    context: Arc<ServiceContext>,
    mail: &MailService,
    name: &str,
    a: &Value,
) -> AppResult<Value> {
    let original = arg(a, "query").or_else(|| arg(a, "filter")).unwrap_or("");
    if name == "search_emails" && original.trim().is_empty() {
        return Err(AppError::validation("query is required"));
    }
    // MailService treats nonempty queries as the complete scope, so a selected
    // folder must be part of the validated expression rather than a UI-only filter.
    let query = if !original.is_empty() {
        if let Some(folder) = arg(a, "folder") {
            format!("({original}) in:\"{}\"", folder.replace('"', "\\\""))
        } else {
            original.into()
        }
    } else {
        String::new()
    };
    let scope = arg(a, "scope").unwrap_or(if name == "search_emails" {
        "auto"
    } else {
        "local"
    });
    if !matches!(scope, "local" | "server" | "auto") {
        return Err(AppError::validation("Invalid search scope"));
    }
    let requested = account(a)?;
    let store = context.read()?;
    let accounts: Vec<_> = store
        .accounts
        .iter()
        .filter(|v| requested.is_none_or(|id| id == v.id))
        .collect();
    if requested.is_some() && accounts.is_empty() {
        return Err(AppError::missing("Account"));
    }
    let incapable: Vec<_> = accounts
        .iter()
        .filter(|v| !context.mail.capabilities(v).remote_search)
        .map(|v| v.id)
        .collect();
    if scope == "server" && !incapable.is_empty() {
        return Err(AppError::new(
            "unsupported_capability",
            format!(
                "Server search is unavailable for accounts {incapable:?}; choose local or auto scope"
            ),
        ));
    }
    let remote = if scope != "local"
        && accounts
            .iter()
            .any(|v| context.mail.capabilities(v).remote_search)
    {
        Some(mail.search_remote(&query, requested, None)?)
    } else {
        None
    };
    let mut offset = 0;
    let mut items = Vec::new();
    let info;
    loop {
        let result = mail.list(MailQuery {
            folder: arg(a, "folder").unwrap_or("Inbox").into(),
            account_id: requested,
            search: query.clone(),
            offset,
            limit: 100,
            threads: a["threads"].as_bool().unwrap_or(false),
            sort: arg(a, "sort").unwrap_or("date_desc").into(),
        })?;
        items.extend(
            result
                .items
                .into_iter()
                .map(|e| serde_json::to_value(e).unwrap_or(Value::Null)),
        );
        if let Some(next) = result.next_offset {
            offset = next;
        } else {
            info = result.search;
            break;
        }
    }
    let complete = info.as_ref().is_some_and(|v| v.coverage == "complete")
        && (scope == "local" || incapable.is_empty())
        && remote.as_ref().is_none_or(|r| r.errors.is_empty());
    let mut result = page(items, a, name)?;
    result["meta"]["scope"] = json!(scope);
    result["meta"]["coverage"] = json!(if complete {
        "complete_for_scope"
    } else {
        "partial"
    });
    result["meta"]["search"] = encode(info)?;
    result["meta"]["remote_report"] = encode(remote)?;
    result["meta"]["accounts_without_remote_search"] = json!(incapable);
    Ok(result)
}
fn read_email(mail: &MailService, a: &Value) -> AppResult<Value> {
    let e = mail.read(number(a, "id")?)?;
    check_email_account(&e, a)?;
    let format = arg(a, "format").unwrap_or("text");
    let mut v = encode(&e)?;
    if format == "headers" || format == "html" {
        let export = mail.export(&[e.id], ExportFormat::Eml)?;
        if format == "headers" {
            let raw = String::from_utf8_lossy(&export.bytes);
            let end = raw
                .find("\r\n\r\n")
                .or_else(|| raw.find("\n\n"))
                .unwrap_or(raw.len());
            v["body"] = Value::Null;
            v["headers"] = text_page(&raw[..end], a)?;
        } else {
            let message = mail_parser::MessageParser::default()
                .parse(&export.bytes)
                .ok_or_else(|| AppError::external("Original MIME could not be parsed"))?;
            let html = message.body_html(0).ok_or_else(|| {
                AppError::new(
                    "unsupported_capability",
                    "This original message has no HTML body",
                )
            })?;
            v["body"] = text_page(&html, a)?;
        }
    } else if format == "text" {
        v["body"] = text_page(&e.body, a)?;
    } else {
        return Err(AppError::validation("format must be text, html or headers"));
    }
    if a["include_thread"].as_bool().unwrap_or(true) {
        let members = mail.conversation(e.id)?;
        v["thread"]=json!(members.iter().take(100).map(|m|json!({"id":m.id,"account_id":m.account_id,"subject":m.subject,"from":m.from_addr,"date":m.date,"snippet":m.snippet(400)})).collect::<Vec<_>>());
        v["thread_truncated"] = json!(members.len() > 100);
    }
    Ok(output(v))
}
fn list_labels(context: Arc<ServiceContext>, a: &Value) -> AppResult<Value> {
    let store = context.read()?;
    let requested = account(a)?;
    let scope = arg(a, "scope").unwrap_or("local");
    if scope == "server" {
        let mut labels = Vec::new();
        for acc in store
            .accounts
            .iter()
            .filter(|v| requested.is_none_or(|id| id == v.id))
        {
            let authenticated = context.account(&store, acc.id)?;
            let data = context.mail.list_labels(&authenticated)?;
            for mut label in data["labels"].as_array().cloned().unwrap_or_default() {
                label["account_id"] = json!(acc.id);
                label["scope"] = json!("server");
                labels.push(label);
            }
        }
        let revision = hash(&json!(labels));
        let mut result = page(labels, a, "list_labels")?;
        result["meta"]["revision"] = json!(revision);
        return Ok(result);
    }
    let mut labels = HashSet::new();
    for e in store
        .emails
        .iter()
        .filter(|e| requested.is_none_or(|id| id == e.account_id))
    {
        for l in &e.labels {
            labels.insert((e.account_id, l.clone()));
        }
    }
    let state = state_read()?;
    for (id, names) in state.labels {
        if requested.is_none_or(|v| v == id) && store.account(id).is_some() {
            for label in names {
                labels.insert((id, label));
            }
        }
    }
    let mut labels: Vec<_> = labels.into_iter().collect();
    labels.sort();
    let mut result = page(
        labels
            .into_iter()
            .map(|(id, label)| json!({"account_id":id,"name":label,"scope":"local"}))
            .collect(),
        a,
        "list_labels",
    )?;
    result["meta"]["revision"] = json!(state.label_revision);
    Ok(result)
}
fn draft_value(d: &Draft) -> AppResult<Value> {
    let mut value = encode(d)?;
    value["revision"] = json!(d.updated);
    value["attachments"]=json!(d.attachments.iter().enumerate().map(|(i,p)|json!({"index":i,"name":Path::new(p).file_name().and_then(|s|s.to_str()).unwrap_or("attachment")})).collect::<Vec<_>>());
    Ok(value)
}
fn core_preferences(store: &Store) -> Value {
    json!({"user_name":store.settings.user_name,"time_zone":store.settings.time_zone,"locale":store.settings.locale,"undo_seconds":store.settings.undo_seconds})
}
fn extra_preferences(state: &State) -> Value {
    let mut value = json!({"working_hours":{"start":"09:00","end":"18:00","weekdays":[1,2,3,4,5]},"default_account_id":null,"default_calendar_id":"local"});
    if let Some(stored) = state.preferences.as_object() {
        for (k, v) in stored {
            value[k] = v.clone();
        }
    }
    value
}
fn preference_value(store: &Store, state: &State) -> Value {
    let mut value = core_preferences(store);
    for (k, v) in extra_preferences(state).as_object().unwrap() {
        value[k] = v.clone();
    }
    value["revision"] = json!(hash(&value));
    value
}
fn apply_core_preferences(store: &mut Store, value: &Value) -> AppResult<()> {
    if let Some(zone) = arg(value, "time_zone") {
        if !zone.is_empty() && zone.parse::<chrono_tz::Tz>().is_err() {
            return Err(AppError::validation("Use an IANA time zone"));
        }
        store.settings.time_zone = zone.into();
    }
    if let Some(locale) = arg(value, "locale") {
        if !locale.is_empty() && !crate::domain::regional::valid_locale(locale) {
            return Err(AppError::validation("Use a recognized locale"));
        }
        store.settings.locale = locale.into();
    }
    if let Some(name) = arg(value, "user_name") {
        if name.trim().is_empty() {
            return Err(AppError::validation("user_name cannot be empty"));
        }
        store.settings.user_name = name.into();
    }
    if let Some(seconds) = value["undo_seconds"].as_u64() {
        if !(5..=30).contains(&seconds) {
            return Err(AppError::validation("undo_seconds must be 5–30"));
        }
        store.settings.undo_seconds = seconds as u32;
    }
    Ok(())
}
// Journal first, then commit Store, then finalize extras. A crash between those
// writes is recoverable and never reported as an atomic rollback after commit.
fn recover_preferences(context: &ServiceContext, state: &mut State) -> AppResult<()> {
    let Some(intent) = state.pending_preferences.clone() else {
        return Ok(());
    };
    let current = core_preferences(&context.read()?);
    if current != intent.new_core {
        if hash(&current) != intent.expected_core {
            state.pending_preferences = None;
            state_save(state)?;
            return Err(AppError::new(
                "revision_conflict",
                "Preferences were changed externally during recovery. Read preferences again; external edits were preserved",
            ));
        }
        context.update(|store| {
            if hash(&core_preferences(store)) != intent.expected_core {
                return Err(AppError::new(
                    "revision_conflict",
                    "Preferences changed during recovery",
                ));
            }
            apply_core_preferences(store, &intent.new_core)
        })?;
    }
    state.preferences = intent.new_extras;
    state.pending_preferences = None;
    state_save(state).map_err(|e|AppError::new("preferences_pending",format!("Core preferences are saved; scheduling preferences will finish on the next read. {}",e.message)))
}
fn read_preferences(context: &ServiceContext) -> AppResult<Value> {
    let _lock = state_lock()?;
    let mut state = state_load()?;
    recover_preferences(context, &mut state)?;
    let store = context.read()?;
    let mut value = preference_value(&store, &state);
    if value["default_account_id"]
        .as_u64()
        .is_some_and(|id| store.account(id as u32).is_none())
    {
        value["default_account_id"] = Value::Null;
    }
    Ok(value)
}
pub fn scheduling_preferences(context: &ServiceContext) -> AppResult<Value> {
    let value = read_preferences(context)?;
    Ok(
        json!({"work_start":value["working_hours"]["start"],"work_end":value["working_hours"]["end"],"weekdays":value["working_hours"]["weekdays"],"default_account_id":value["default_account_id"],"default_calendar_id":value["default_calendar_id"]}),
    )
}
fn update_preferences(context: &ServiceContext, a: &Value) -> AppResult<Value> {
    let expected = required(a, "expected_revision")?;
    let _lock = state_lock()?;
    let mut state = state_load()?;
    recover_preferences(context, &mut state)?;
    let store = context.read()?;
    if preference_value(&store, &state)["revision"].as_str() != Some(expected) {
        return Err(AppError::new(
            "revision_conflict",
            "Preferences changed; read them again",
        ));
    }
    let mut updated = store.clone();
    apply_core_preferences(&mut updated, a)?;
    let mut extras = extra_preferences(&state);
    if let Some(hours) = a.get("working_hours") {
        let object = hours
            .as_object()
            .ok_or_else(|| AppError::validation("working_hours must be an object"))?;
        for (k, v) in object {
            if !matches!(k.as_str(), "start" | "end" | "weekdays") {
                return Err(AppError::validation("Unknown working_hours field"));
            }
            extras["working_hours"][k] = v.clone();
        }
        let parse = |field: &str| -> AppResult<chrono::NaiveTime> {
            let value = extras["working_hours"][field]
                .as_str()
                .ok_or_else(|| AppError::validation("Working hours use HH:MM"))?;
            chrono::NaiveTime::parse_from_str(value, "%H:%M")
                .map_err(|_| AppError::validation("Working hours use HH:MM"))
        };
        if parse("end")? <= parse("start")? {
            return Err(AppError::validation("Working day must end after its start"));
        }
        let days = extras["working_hours"]["weekdays"]
            .as_array()
            .ok_or_else(|| AppError::validation("weekdays must be a list of 1–7"))?;
        let mut seen = HashSet::new();
        if days.is_empty()
            || days.iter().any(|d| {
                d.as_u64()
                    .is_none_or(|n| !(1..=7).contains(&n) || !seen.insert(n))
            })
        {
            return Err(AppError::validation(
                "weekdays must contain distinct days 1–7, Monday=1",
            ));
        }
    }
    if a.get("default_account_id").is_some() {
        extras["default_account_id"] = if a["default_account_id"].is_null() {
            Value::Null
        } else {
            let id = number(a, "default_account_id")?;
            if id > u32::MAX as u64 || store.account(id as u32).is_none() {
                return Err(AppError::missing("Default account"));
            }
            json!(id)
        };
    }
    if a.get("default_calendar_id").is_some() {
        let id = required(a, "default_calendar_id")?;
        if id.len() > 2048 || id.chars().any(char::is_control) {
            return Err(AppError::validation("Invalid calendar ID"));
        }
        extras["default_calendar_id"] = json!(id);
    }
    state.pending_preferences = Some(PreferenceIntent {
        expected_core: hash(&core_preferences(&store)),
        new_core: core_preferences(&updated),
        new_extras: extras,
    });
    state_save(&state)?;
    recover_preferences(context, &mut state)?;
    Ok(preference_value(&context.read()?, &state))
}
fn save_signature(context: Arc<ServiceContext>, a: &Value) -> AppResult<Value> {
    let name = required(a, "name")?;
    let body = arg(a, "body").ok_or_else(|| AppError::validation("body is required"))?;
    let id = if a.get("id").is_some() {
        number(a, "id")?
    } else {
        0
    };
    let receipt = if id == 0 {
        Some(format!(
            "signature:{}",
            hash(&json!(required(a, "idempotency_key")?))
        ))
    } else {
        None
    };
    let fingerprint = hash(a);
    if let Some(key) = &receipt {
        let cached = state_update(|state| {
            if let Some(old) = state.requests.get(key) {
                if old["fingerprint"] != fingerprint {
                    return Err(AppError::new(
                        "idempotency_conflict",
                        "Signature key was used for another request",
                    ));
                }
                return old.get("result").cloned().map(Some).ok_or_else(||AppError::new("operation_pending","The signature request was already claimed; inspect list_signatures before retrying"));
            }
            state.requests.insert(
                key.clone(),
                json!({"fingerprint":fingerprint,"state":"pending"}),
            );
            Ok(None)
        })?;
        if let Some(cached) = cached {
            return Ok(output(cached));
        }
    }
    let v = context.update(|store| {
        if id != 0 {
            let current = store
                .signatures
                .iter()
                .find(|v| v.id == id)
                .ok_or_else(|| AppError::missing("Signature"))?;
            if current.modified.max(0) as u64 != revision(a)? {
                return Err(AppError::new(
                    "revision_conflict",
                    "Signature changed; read it again",
                ));
            }
        }
        let mut value = Signature {
            id: if id == 0 { store.new_id() } else { id },
            name: name.into(),
            body: body.into(),
            html: a["html"].as_bool().unwrap_or(false),
            modified: context.clock.now(),
        };
        if let Some(old) = store.signatures.iter_mut().find(|v| v.id == id) {
            value.modified = value.modified.max(old.modified + 1);
            *old = value.clone();
        } else {
            store.signatures.push(value.clone());
        }
        Ok(value)
    })?;
    let mut value = encode(v)?;
    value["revision"] = value["modified"].clone();
    if let Some(key) = receipt {
        state_update(|state| {
            state
                .requests
                .insert(key, json!({"fingerprint":fingerprint,"result":value}));
            Ok(())
        })?;
    }
    Ok(output(value))
}
fn create_draft(context: Arc<ServiceContext>, a: &Value) -> AppResult<Draft> {
    let key = required(a, "idempotency_key")?;
    if key.len() > 200 {
        return Err(AppError::validation("idempotency_key too long"));
    }
    let key_hash = hash(&json!(key));
    let request_id = format!("mcp:{}:{}", key_hash, hash(a));
    let mail = mail(context.clone())?;
    let source = if a.get("reply_to").is_some() {
        Some(mail.read(number(a, "reply_to")?)?)
    } else {
        None
    };
    let store = context.read()?;
    let account_id = if let Some(id) = account(a)? {
        id
    } else if let Some(email) = arg(a, "account") {
        store
            .accounts
            .iter()
            .find(|v| v.email.eq_ignore_ascii_case(email))
            .map(|v| v.id)
            .ok_or_else(|| AppError::missing("Account"))?
    } else if let Some(e) = &source {
        e.account_id
    } else if let Some(id) = scheduling_preferences(&context)?["default_account_id"].as_u64() {
        u32::try_from(id).map_err(|_| AppError::validation("Invalid default account ID"))?
    } else {
        return Err(AppError::validation("Select account_id explicitly"));
    };
    if source.as_ref().is_some_and(|e| e.account_id != account_id) {
        return Err(AppError::validation(
            "Reply source belongs to another account",
        ));
    }
    let mode = arg(a, "mode").unwrap_or(if source.is_some() { "reply" } else { "new" });
    if !matches!(mode, "new" | "reply" | "reply_all" | "forward") {
        return Err(AppError::validation("Invalid composer mode"));
    }
    if mode != "new" && source.is_none() {
        return Err(AppError::validation("reply_to is required for this mode"));
    }
    context.update(|store| {
        if let Some(d) = store
            .drafts
            .iter()
            .chain(store.outbox.iter().map(|v| &v.draft))
            .find(|v| v.request_id.starts_with(&format!("mcp:{key_hash}:")))
        {
            return if d.request_id == request_id {
                Ok(d.clone())
            } else {
                Err(AppError::new(
                    "idempotency_conflict",
                    "Key already used for another draft request",
                ))
            };
        }
        if store.account(account_id).is_none() {
            return Err(AppError::missing("Account"));
        }
        let mut d = Draft {
            id: store.new_id(),
            account_id,
            request_id: request_id.clone(),
            signature_id: store.signature_for_new(account_id),
            updated: context.clock.now(),
            ..Default::default()
        };
        if let Some(e) = &source {
            let prefix = if mode == "forward" { "Fwd:" } else { "Re:" };
            d.subject = if e.subject.to_lowercase().starts_with(&prefix.to_lowercase()) {
                e.subject.clone()
            } else {
                format!("{prefix} {}", e.subject)
            };
            d.quote = format!(
                "On {}, {} wrote:\n{}",
                crate::rules::fmt_full(e.date),
                e.from_addr,
                e.body
            );
            if mode != "forward" {
                d.reply_to = Some(e.id);
                d.to = e.from_addr.clone();
                d.signature_id = store.signature_for_reply(e);
                if mode == "reply_all" {
                    let own: HashSet<_> = store
                        .accounts
                        .iter()
                        .map(|a| a.email.to_lowercase())
                        .collect();
                    let mut seen = HashSet::new();
                    seen.insert(e.from_addr.to_lowercase());
                    d.cc =
                        e.to.iter()
                            .chain(&e.cc)
                            .filter(|s| {
                                !own.contains(&s.to_lowercase()) && seen.insert(s.to_lowercase())
                            })
                            .cloned()
                            .collect::<Vec<_>>()
                            .join(", ");
                }
            }
        }
        patch(&mut d, a)?;
        if d.signature_id
            .is_some_and(|id| !store.signatures.iter().any(|s| s.id == id))
        {
            return Err(AppError::missing("Signature"));
        }
        store.drafts.push(d.clone());
        Ok(d)
    })
}

#[derive(Serialize, Deserialize, Clone, Default)]
struct Task {
    id: String,
    source_type: String,
    source_id: u64,
    source_account_id: Option<u32>,
    title: String,
    due: Option<String>,
    completed: bool,
    revision: u64,
    request_id: String,
}
#[derive(Serialize, Deserialize, Default)]
struct State {
    tasks: Vec<Task>,
    labels: BTreeMap<u32, Vec<String>>,
    label_revision: u64,
    activity: Vec<Value>,
    requests: BTreeMap<String, Value>,
    #[serde(default)]
    preferences: Value,
    #[serde(default)]
    pending_preferences: Option<PreferenceIntent>,
}
#[derive(Serialize, Deserialize, Clone)]
struct PreferenceIntent {
    expected_core: String,
    new_core: Value,
    new_extras: Value,
}
fn state_path() -> PathBuf {
    Store::data_dir().join("mcp-tools.json")
}
fn state_lock() -> AppResult<File> {
    let p = state_path();
    std::fs::create_dir_all(p.parent().unwrap())?;
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(p.with_extension("lock"))?;
    f.lock()?;
    Ok(f)
}
fn state_load() -> AppResult<State> {
    match std::fs::read(state_path()) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(State::default()),
        Err(e) => Err(e.into()),
    }
}
fn state_read() -> AppResult<State> {
    let _lock = state_lock()?;
    state_load()
}
fn state_update<T>(operation: impl FnOnce(&mut State) -> AppResult<T>) -> AppResult<T> {
    let _lock = state_lock()?;
    let mut state = state_load()?;
    let result = operation(&mut state)?;
    state_save(&state)?;
    Ok(result)
}
fn state_save(state: &State) -> AppResult<()> {
    let target = state_path();
    let tmp = target.with_extension(format!("{}.tmp", std::process::id()));
    let mut f = File::create(&tmp)?;
    f.write_all(&serde_json::to_vec(&state)?)?;
    f.sync_all()?;
    drop(f);
    std::fs::rename(tmp, target)?;
    Ok(())
}
fn manage_label(context: &Arc<ServiceContext>, a: &Value) -> AppResult<Value> {
    if arg(a, "scope") != Some("local") {
        let id = account(a)?.ok_or_else(|| AppError::validation("account_id is required"))?;
        let store = context.read()?;
        let acc = context.account(&store, id)?;
        if !context.mail.capabilities(&acc).labels {
            return Err(AppError::new(
                "unsupported_capability",
                "This connector cannot manage provider labels; explicit local scope remains available",
            ));
        }
        let action = required(a, "action")?;
        if !matches!(action, "create" | "rename" | "delete") {
            return Err(AppError::validation("Invalid label action"));
        }
        let name = required(a, "name")?;
        let receipt = if action == "create" {
            Some(format!(
                "label:{id}:{}",
                hash(&json!(required(a, "idempotency_key")?))
            ))
        } else {
            None
        };
        let fingerprint = hash(a);
        if let Some(key) = &receipt {
            let cached = state_update(|state| {
                if let Some(previous) = state.requests.get(key) {
                    if previous["fingerprint"] != fingerprint {
                        return Err(AppError::new(
                            "idempotency_conflict",
                            "Label key already used for another request",
                        ));
                    }
                    if previous.get("result").is_none() {
                        return Err(AppError::new(
                            "operation_pending",
                            "A label request with this key is in progress or has an uncertain result; inspect provider labels before retrying",
                        ));
                    }
                    return Ok(Some(previous["result"].clone()));
                }
                state.requests.insert(
                    key.clone(),
                    json!({"fingerprint":fingerprint,"state":"pending"}),
                );
                Ok(None)
            })?;
            if let Some(cached) = cached {
                return Ok(output(cached));
            }
        } else {
            let current = list_labels(context.clone(), &json!({"account_id":id,"scope":"server"}))?;
            if a.get("expected_revision") != current["meta"].get("revision") {
                return Err(AppError::new(
                    "revision_conflict",
                    "Read this account's server labels and pass meta.revision",
                ));
            }
        }
        let value = context
            .mail
            .manage_label(&acc, action, name, arg(a, "new_name"))?;
        let result = json!({"account_id":id,"scope":"server","result":value});
        if let Some(key) = receipt {
            state_update(|state| {
                state
                    .requests
                    .insert(key, json!({"fingerprint":fingerprint,"result":result}));
                state.activity.push(json!({"kind":"label","account_id":id,"scope":"server","action":action,"at":crate::model::now_ts()}));
                Ok(())
            })?;
        }
        return Ok(output(result));
    }
    let id = account(a)?.ok_or_else(|| AppError::validation("account_id required"))?;
    if context.read()?.account(id).is_none() {
        return Err(AppError::missing("Account"));
    }
    let name = required(a, "name")?;
    let action = required(a, "action")?;
    state_update(|state| {
        if action != "create" && state.label_revision != revision(a)? {
            return Err(AppError::new(
                "revision_conflict",
                "Labels changed; read list_labels again",
            ));
        }
        let names = state.labels.entry(id).or_default();
        match action {
            "create" => {
                if !names.iter().any(|v| v == name) {
                    names.push(name.into());
                }
            }
            "rename" => {
                let new = required(a, "new_name")?;
                let old = names
                    .iter_mut()
                    .find(|v| v.as_str() == name)
                    .ok_or_else(|| AppError::missing("Local label"))?;
                *old = new.into();
            }
            "delete" => {
                names.retain(|v| v != name);
            }
            _ => return Err(AppError::validation("Invalid label action")),
        }
        state.label_revision += 1;
        state.activity.push(
            json!({"kind":"label","account_id":id,"action":action,"at":crate::model::now_ts()}),
        );
        Ok(output(
            json!({"account_id":id,"scope":"local","revision":state.label_revision,"labels":names}),
        ))
    })
}
fn task_call(context: Arc<ServiceContext>, name: &str, a: &Value) -> AppResult<Value> {
    let store = context.read()?;
    if name == "list_activity" {
        return page(
            state_read()?
                .activity
                .into_iter()
                .filter(|v| {
                    v.get("account_id")
                        .and_then(Value::as_u64)
                        .is_none_or(|id| store.account(id as u32).is_some())
                })
                .collect(),
            a,
            name,
        );
    }
    if name == "list_tasks" {
        let requested = account(a)?;
        return page(
            state_read()?
                .tasks
                .into_iter()
                .filter(|t| {
                    t.source_account_id.is_none_or(|id| {
                        store.account(id).is_some() && requested.is_none_or(|v| v == id)
                    }) && a["completed"].as_bool().is_none_or(|v| v == t.completed)
                })
                .map(|t| serde_json::to_value(t).unwrap_or(Value::Null))
                .collect(),
            a,
            name,
        );
    }
    if name == "create_task_from_source" {
        let source_type = required(a, "source_type")?;
        let source_id = number(a, "source_id")?;
        let key = required(a, "idempotency_key")?;
        let (source_account_id, title) = match source_type {
            "email" => {
                let e = mail(context.clone())?.read(source_id)?;
                check_email_account(&e, a)?;
                (Some(e.account_id), e.subject)
            }
            "meeting" => {
                let d = meetings(context)?.read(source_id)?;
                (None, d.meeting.title)
            }
            _ => return Err(AppError::validation("Use email or meeting source")),
        };
        let title = arg(a, "title").unwrap_or(&title).trim();
        if title.is_empty() {
            return Err(AppError::validation("Task title cannot be empty"));
        }
        if let Some(due) = arg(a, "due") {
            validate_due(due)?;
        }
        let request = hash(a);
        let key = hash(&json!(key));
        return state_update(|state| {
            if let Some(t) = state
                .tasks
                .iter()
                .find(|t| t.request_id.starts_with(&format!("{key}:")))
            {
                return if t.request_id == format!("{key}:{request}") {
                    Ok(output(encode(t)?))
                } else {
                    Err(AppError::new(
                        "idempotency_conflict",
                        "Key already used for another task",
                    ))
                };
            }
            let task = Task {
                id: format!("task-{}", &hash(&json!([key, request]))[..24]),
                source_type: source_type.into(),
                source_id,
                source_account_id,
                title: title.into(),
                due: arg(a, "due").map(str::to_owned),
                completed: false,
                revision: 1,
                request_id: format!("{key}:{request}"),
            };
            state.tasks.push(task.clone());
            state.activity.push(json!({"kind":"task","action":"created","id":task.id,"account_id":source_account_id,"at":crate::model::now_ts()}));
            Ok(output(encode(task)?))
        });
    }
    let id = required(a, "id")?;
    state_update(|state| {
        let t = state
            .tasks
            .iter_mut()
            .find(|t| t.id == id)
            .ok_or_else(|| AppError::missing("Task"))?;
        if t.source_account_id
            .is_some_and(|id| store.account(id).is_none())
        {
            return Err(AppError::missing("Task"));
        }
        if t.revision != revision(a)? {
            return Err(AppError::new(
                "revision_conflict",
                "Task changed; read it again",
            ));
        }
        if name == "complete_task" {
            t.completed = a["completed"].as_bool().unwrap_or(true);
        } else {
            if let Some(title) = arg(a, "title") {
                if title.trim().is_empty() {
                    return Err(AppError::validation("Task title cannot be empty"));
                }
                t.title = title.into();
            }
            if a.get("due").is_some() {
                if let Some(due) = arg(a, "due") {
                    validate_due(due)?;
                    t.due = Some(due.into());
                } else if a["due"].is_null() {
                    t.due = None;
                } else {
                    return Err(AppError::validation("Invalid due date"));
                }
            }
        }
        t.revision += 1;
        let value = encode(t.clone())?;
        let source_account_id = t.source_account_id;
        state
            .activity
            .push(json!({"kind":"task","action":"updated","id":id,"account_id":source_account_id,"at":crate::model::now_ts()}));
        Ok(output(value))
    })
}
fn validate_due(due: &str) -> AppResult<()> {
    if chrono::DateTime::parse_from_rfc3339(due).is_err()
        && chrono::NaiveDate::parse_from_str(due, "%Y-%m-%d").is_err()
    {
        Err(AppError::validation("due must be YYYY-MM-DD or RFC3339"))
    } else {
        Ok(())
    }
}

struct NoCapture;
impl crate::application::meeting_ports::AudioCapture for NoCapture {
    fn devices(&self) -> AppResult<Vec<AudioDevice>> {
        Ok(vec![])
    }
    fn start(&self, _: CaptureRequest, _: PathBuf) -> AppResult<CaptureSnapshot> {
        Err(AppError::new(
            "permission_denied",
            "Recording starts only from the desktop interface",
        ))
    }
    fn pause(&self) -> AppResult<CaptureSnapshot> {
        Err(AppError::new(
            "unsupported_capability",
            "This MCP runtime does not capture audio",
        ))
    }
    fn resume(&self) -> AppResult<CaptureSnapshot> {
        self.pause()
    }
    fn stop(&self) -> AppResult<CaptureSnapshot> {
        self.pause()
    }
    fn snapshot(&self) -> AppResult<CaptureSnapshot> {
        Ok(CaptureSnapshot::default())
    }
    fn audio_range(&self, _: &Path, _: &[AudioChunk], _: u64, _: u64) -> AppResult<Vec<u8>> {
        Err(AppError::new(
            "unsupported_capability",
            "Use the desktop player for audio playback",
        ))
    }
}
type MeetingCache = Mutex<
    HashMap<
        usize,
        (
            Arc<ServiceContext>,
            crate::application::meetings::MeetingService,
        ),
    >,
>;
fn meetings(
    context: Arc<ServiceContext>,
) -> AppResult<crate::application::meetings::MeetingService> {
    static CACHE: OnceLock<MeetingCache> = OnceLock::new();
    let key = Arc::as_ptr(&context) as usize;
    let mut c = CACHE
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .map_err(|_| AppError::storage("Meeting runtime unavailable"))?;
    Ok(c.entry(key)
        .or_insert_with(|| {
            let service = crate::application::meetings::MeetingService::new(
                context.clone(),
                Arc::new(
                    crate::infrastructure::meeting_repository::FileMeetingRepository::new(
                        Store::data_dir().join("meetings"),
                    ),
                ),
                Arc::new(NoCapture),
                crate::infrastructure::meeting_gateways::shared_processor(),
            );
            (context, service)
        })
        .1
        .clone())
}
fn meeting_call(context: Arc<ServiceContext>, name: &str, a: &Value) -> AppResult<Value> {
    let mut service = meetings(context.clone())?;
    if let Some(profile) = a["_source_profile"].as_str() {
        service = service.with_source_profile(profile.to_owned());
    }
    match name {
        "list_meetings" => {
            let values=service.list()?.into_iter().map(|d|json!({"id":d.meeting.id,"title":d.meeting.title,"date":d.meeting.date,"duration_secs":d.meeting.duration_secs,"revision":d.revision,"included_in_search":d.included_in_search,"jobs":d.jobs})).collect();
            page(values, a, name)
        }
        "search_meetings" => page(
            service
                .search(required(a, "query")?)?
                .into_iter()
                .map(|h| serde_json::to_value(h).unwrap_or(Value::Null))
                .collect(),
            a,
            name,
        ),
        "read_meeting" => {
            let mut d = service.read(number(a, "id")?)?;
            if !a["transcript"].as_bool().unwrap_or(false) {
                d.meeting.transcript.clear();
                d.transcripts.clear();
                d.legacy_transcript = None;
            }
            if !a["notes"].as_bool().unwrap_or(false) {
                d.meeting.notes.clear();
                d.notes.clear();
                d.note_history.clear();
                d.legacy_notes = None;
            }
            d.capture.chunks.clear();
            d.recovery_copies.clear();
            Ok(output(encode(d)?))
        }
        "read_meeting_segments" => {
            let d = service.read(number(a, "id")?)?;
            let t = if let Some(version) = arg(a, "version") {
                d.transcripts.iter().find(|t| t.id == version)
            } else {
                d.transcript()
            }
            .ok_or_else(|| AppError::missing("Timestamped transcript version"))?;
            let mut result = page(
                t.segments
                    .iter()
                    .map(|s| serde_json::to_value(s).unwrap_or(Value::Null))
                    .collect(),
                a,
                name,
            )?;
            result["meta"]["version"] = json!(t.id);
            Ok(result)
        }
        "update_meeting_notes" => Ok(output(encode(service.save_notes(
            number(a, "id")?,
            required(a, "notes")?.into(),
            revision(a)?,
        )?)?)),
        "export_meeting" => {
            let id = number(a, "id")?;
            let format = arg(a, "format").unwrap_or("markdown");
            let text = service.export_selected(id, format, arg(a, "version"), None, None)?;
            let ext = match format {
                "markdown" => "md",
                "json" => "json",
                _ => "txt",
            };
            Ok(output(files::write(
                &format!("meeting-{id}.{ext}"),
                if format == "json" {
                    "application/json"
                } else {
                    "text/plain"
                },
                text.as_bytes(),
            )?))
        }
        "get_meeting_job" => {
            let d = service.read(number(a, "id")?)?;
            let id = required(a, "job_id")?;
            Ok(output(encode(
                d.jobs
                    .iter()
                    .find(|j| j.id == id)
                    .ok_or_else(|| AppError::missing("Meeting job"))?,
            )?))
        }
        "cancel_meeting_job" => {
            let id = number(a, "id")?;
            let job = required(a, "job_id")?;
            let owned = owned_meeting_jobs()
                .lock()
                .map_err(|_| AppError::storage("Job registry unavailable"))?
                .contains(&(Arc::as_ptr(&context) as usize, id, job.into()));
            if !owned {
                return Err(AppError::new(
                    "operation_pending",
                    "This job belongs to another runtime; cancel it in its desktop/MCP client",
                ));
            }
            Ok(output(encode(service.cancel_job(id, job)?)?))
        }
        "import_meeting" | "transcribe_meeting" | "summarize_meeting" => {
            let key = required(a, "idempotency_key")?;
            let fingerprint = hash(a);
            let state_key = format!("meeting:{}", hash(&json!(key)));
            let cached = state_update(|state| {
                if let Some(old) = state.requests.get(&state_key) {
                    if old["fingerprint"] != fingerprint {
                        return Err(AppError::new(
                            "idempotency_conflict",
                            "Key already used for another meeting job",
                        ));
                    }
                    if old.get("result").is_none() {
                        return Err(AppError::new(
                            "operation_pending",
                            "This request was already claimed. Inspect meeting jobs before starting another import or processing job",
                        ));
                    }
                    return Ok(Some(old["result"].clone()));
                }
                state.requests.insert(
                    state_key.clone(),
                    json!({"fingerprint":fingerprint,"state":"claimed"}),
                );
                Ok(None)
            })?;
            if let Some(cached) = cached {
                return Ok(output(cached));
            }
            let d = if name == "import_meeting" {
                let visible: Vec<_> = context.read()?.accounts.iter().map(|a| a.id).collect();
                service.import(files::resolve_scoped(
                    required(a, "handle")?,
                    Some(&visible),
                )?)?
            } else {
                let id = number(a, "id")?;
                let d = service.read(id)?;
                if d.revision != revision(a)? {
                    return Err(AppError::new(
                        "revision_conflict",
                        "Meeting changed; read it again",
                    ));
                }
                if name == "transcribe_meeting" {
                    service.transcribe(
                        id,
                        TranscribeOptions {
                            // `local`/`remote` (the names before the compute server) still work.
                            provider: arg(a, "provider").unwrap_or(crate::domain::meeting::ENGINE_COMPUTE).into(),
                            model: arg(a, "model").unwrap_or("base").into(),
                            language: arg(a, "language").unwrap_or("it").into(),
                        },
                    )?
                } else {
                    service.summarize(id, arg(a, "language").unwrap_or("it").into())?
                }
            };
            for job in &d.jobs {
                if matches!(job.state.as_str(), "queued" | "running") {
                    owned_meeting_jobs()
                        .lock()
                        .map_err(|_| AppError::storage("Job registry unavailable"))?
                        .insert((Arc::as_ptr(&context) as usize, d.meeting.id, job.id.clone()));
                }
            }
            let value = json!({"id":d.meeting.id,"revision":d.revision,"jobs":d.jobs});
            state_update(|s| {
                s.requests
                    .insert(state_key, json!({"fingerprint":fingerprint,"result":value}));
                Ok(())
            })?;
            Ok(output(value))
        }
        _ => Err(AppError::new("unknown_tool", name)),
    }
}
fn owned_meeting_jobs() -> &'static Mutex<HashSet<(usize, u64, String)>> {
    static OWNED: OnceLock<Mutex<HashSet<(usize, u64, String)>>> = OnceLock::new();
    OWNED.get_or_init(|| Mutex::new(HashSet::new()))
}

/// The activity row for one `email_action` call of the assistant (centro attività): one
/// row for the whole call, never one per message. `succeeded` holds the (id, account,
/// subject) of the messages the action reached. Only the actions the history keeps count.
fn assistant_mail_activity(action: &str, succeeded: &[(u64, u32, String)]) -> Option<NewActivity> {
    if !LOGGED_MAIL_ACTIONS.contains(&action) {
        return None;
    }
    let (id, account, subject) = succeeded.first()?;
    let single = succeeded.len() == 1;
    Some(NewActivity {
        kind: ActivityKind::Action,
        code: action.to_string(),
        source: ActivitySource::Assistant,
        account_id: succeeded.iter().all(|(_, other, _)| other == account).then_some(*account),
        subject: single.then(|| subject.clone()).filter(|subject| !subject.is_empty()),
        count: (!single).then(|| u32::try_from(succeeded.len()).unwrap_or(u32::MAX)),
        ref_kind: single.then(|| "email".to_string()),
        ref_id: single.then_some(*id),
        ..Default::default()
    })
}

/// The activity row for a draft the assistant discarded.
fn assistant_draft_activity(draft: &Draft) -> NewActivity {
    NewActivity {
        kind: ActivityKind::Action,
        code: crate::domain::activity::code::DISCARD_DRAFT.to_string(),
        source: ActivitySource::Assistant,
        account_id: Some(draft.account_id),
        subject: Some(draft.subject.clone()).filter(|subject| !subject.is_empty()),
        ref_kind: Some("draft".to_string()),
        ref_id: Some(draft.id),
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::ports::*;
    struct ClockTest;
    impl Clock for ClockTest {
        fn now(&self) -> i64 {
            2_000_000_000
        }
    }
    struct SecretsTest;
    impl SecretStore for SecretsTest {
        fn get(&self, _: &str) -> AppResult<String> {
            Ok(String::new())
        }
        fn set(&self, _: &str, _: &str) -> AppResult<()> {
            Err(AppError::new(
                "permission_denied",
                "Test secrets are read-only",
            ))
        }
    }
    struct AttachmentsTest;
    impl AttachmentGateway for AttachmentsTest {
        fn total_size(&self, _: &[String]) -> AppResult<u64> {
            Ok(0)
        }
        fn read_text(&self, _: &crate::model::Attachment) -> AppResult<String> {
            Ok("attachment text".into())
        }
    }
    struct MailTest;
    impl MailGateway for MailTest {
        fn capabilities(
            &self,
            _: &crate::model::Account,
        ) -> crate::domain::outbox::ProviderCapabilities {
            crate::domain::outbox::ProviderCapabilities {
                provider_id: "second-provider".into(),
                ..Default::default()
            }
        }
        fn sync(&self, _: &crate::model::Account) -> AppResult<SyncBatch> {
            Ok(SyncBatch {
                emails: vec![],
                archive_folder: None,
                trash_folder: None,
            })
        }
        fn apply(&self, _: &crate::model::Account, _: u32, _: RemoteAction) -> AppResult<()> {
            Err(AppError::new("unsupported_capability", "No remote write"))
        }
        fn send(&self, _: &crate::model::Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
            panic!("MCP draft tests must never send mail")
        }
        fn check_login(&self, _: &crate::model::Account) -> AppResult<()> {
            Ok(())
        }
    }
    fn context() -> Arc<ServiceContext> {
        let mut store = Store::default();
        store.accounts.push(crate::model::Account {
            id: 1,
            email: "me@example.com".into(),
            ..Default::default()
        });
        store.emails.push(crate::model::Email {
            id: 8,
            account_id: 1,
            from_addr: "sender@example.com".into(),
            to: vec!["me@example.com".into(), "colleague@example.com".into()],
            cc: vec!["sender@example.com".into()],
            subject: "Topic".into(),
            body: "Original".into(),
            thread_id: "thread".into(),
            ..Default::default()
        });
        Arc::new(ServiceContext { compute: std::sync::Arc::new(crate::infrastructure::fake_compute::FakeCompute::default()),
            events: Arc::new(NullSink),
            repository: Arc::new(crate::infrastructure::repository::MemoryRepository::new(
                store,
            )),
            mail: Arc::new(MailTest),
            secrets: Arc::new(SecretsTest),
            clock: Arc::new(ClockTest),
            attachments: Arc::new(AttachmentsTest),
        })
    }
    #[test]
    fn pagination_exposes_every_result_and_rejects_changed_scope() {
        let v: Vec<_> = (0..201).map(|id| json!({"id":id})).collect();
        let mut a = json!({"limit":100});
        let p = page(v.clone(), &a, "test").unwrap();
        assert_eq!(p["data"]["items"].as_array().unwrap().len(), 100);
        a["cursor"] = p["data"]["next_cursor"].clone();
        let p2 = page(v.clone(), &a, "test").unwrap();
        assert_eq!(p2["data"]["items"][0]["id"], "100");
        a["cursor"] = p2["data"]["next_cursor"].clone();
        let p3 = page(v.clone(), &a, "test").unwrap();
        assert_eq!(p3["data"]["items"][0]["id"], "200");
        assert_eq!(p3["data"]["next_cursor"], Value::Null);
        assert!(page(v, &a, "other").is_err());
    }
    #[test]
    fn schema_catalog_has_unique_names_and_strict_inputs() {
        let mut names = HashSet::new();
        for tool in catalog() {
            assert!(names.insert(tool["name"].as_str().unwrap().to_owned()));
            assert_eq!(tool["inputSchema"]["additionalProperties"], false);
            assert!(tool["inputSchema"]["properties"].is_object());
        }
    }
    #[test]
    fn text_paging_is_unicode_safe() {
        let p = text_page("caffè ☕", &json!({"offset":4,"max_chars":2})).unwrap();
        assert_eq!(p["text"], "è ");
        assert_eq!(p["next_offset"], 6);
        assert!(text_page("a", &json!({"offset":2})).is_err());
    }
    #[test]
    fn draft_creation_is_idempotent_and_reply_all_excludes_own_address() {
        let ctx = context();
        let args = json!({"reply_to":"8","mode":"reply_all","body":"Thanks","idempotency_key":"reply-1","bcc":["private@example.com"]});
        let first = call(ctx.clone(), "create_draft", &args).unwrap();
        let second = call(ctx.clone(), "create_draft", &args).unwrap();
        assert_eq!(first["data"]["id"], second["data"]["id"]);
        assert_eq!(first["data"]["cc"], "colleague@example.com");
        assert_eq!(first["data"]["bcc"], "private@example.com");
        assert_eq!(ctx.read().unwrap().drafts.len(), 1);
        let mut changed = args;
        changed["body"] = json!("different");
        assert_eq!(
            call(ctx, "create_draft", &changed).unwrap_err().code,
            "idempotency_conflict"
        );
    }
    #[test]
    fn writes_require_access_and_stale_draft_cannot_overwrite_newer_content() {
        let ctx = context();
        let created = call(
            ctx.clone(),
            "create_draft",
            &json!({"account_id":1,"body":"first","idempotency_key":"new"}),
        )
        .unwrap();
        let id = created["data"]["id"].clone();
        let rev = created["data"]["revision"].clone();
        let updated = call(
            ctx.clone(),
            "update_draft",
            &json!({"id":id,"expected_revision":rev,"body":"second"}),
        )
        .unwrap();
        assert_eq!(updated["data"]["body"], "second");
        assert_eq!(
            call(
                ctx.clone(),
                "update_draft",
                &json!({"id":id,"expected_revision":rev,"body":"lost"})
            )
            .unwrap_err()
            .code,
            "revision_conflict"
        );
        ctx.update(|s| {
            s.settings.mcp_access = Access::ReadOnly;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            call(
                ctx.clone(),
                "delete_draft",
                &json!({"id":id,"expected_revision":updated["data"]["revision"]})
            )
            .unwrap_err()
            .code,
            "permission_denied"
        );
        assert_eq!(ctx.read().unwrap().drafts[0].body, "second");
    }
    #[test]
    fn account_selection_and_provider_capabilities_are_explicit() {
        let ctx = context();
        let accounts = call(ctx.clone(), "list_accounts", &json!({})).unwrap();
        assert_eq!(
            accounts["data"]["items"][0]["provider_id"],
            "second-provider"
        );
        assert_eq!(
            accounts["data"]["items"][0]["capabilities"]["remote_drafts"],
            false
        );
        assert!(call(ctx, "read_email", &json!({"id":8,"account_id":2})).is_err());
    }
    #[test]
    fn file_task_storage_is_tested_in_an_isolated_child() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../checks")
            .join(format!(
                "mcp-tools-test-{}-{}",
                std::process::id(),
                crate::model::now_ts()
            ));
        std::fs::create_dir_all(&root).unwrap();
        let status = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "adapters::mcp::tools::tests::isolated_file_task_child",
                "--nocapture",
            ])
            .env("QAMILLE_DATA", root.join("data"))
            .env("QAMILLE_MCP_TOOL_TEST_CHILD", "1")
            .env_remove("QAMILLE_MCP_FILE_ROOTS")
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn isolated_file_task_child() {
        if std::env::var("QAMILLE_MCP_TOOL_TEST_CHILD").as_deref() != Ok("1") {
            return;
        }
        let ctx = context();
        assert!(
            call(
                ctx.clone(),
                "create_draft",
                &json!({"body":"hello","idempotency_key":"missing-account"})
            )
            .is_err()
        );
        let args = json!({"source_type":"email","source_id":8,"title":"Follow up","due":"2026-11-03","idempotency_key":"task-1"});
        let first = call(ctx.clone(), "create_task_from_source", &args).unwrap();
        let again = call(ctx.clone(), "create_task_from_source", &args).unwrap();
        assert_eq!(first["data"]["id"], again["data"]["id"]);
        let id = first["data"]["id"].clone();
        let completed = call(
            ctx.clone(),
            "complete_task",
            &json!({"id":id,"expected_revision":1}),
        )
        .unwrap();
        assert_eq!(completed["data"]["completed"], true);
        assert_eq!(
            call(
                ctx.clone(),
                "update_task",
                &json!({"id":id,"title":"stale","expected_revision":1})
            )
            .unwrap_err()
            .code,
            "revision_conflict"
        );
        let tasks = call(ctx, "list_tasks", &json!({})).unwrap();
        assert_eq!(tasks["data"]["total"], 1);
        let prefs_context = context();
        let before = call(prefs_context.clone(), "get_preferences", &json!({})).unwrap();
        let changed=call(prefs_context.clone(),"update_preferences",&json!({"expected_revision":before["data"]["revision"],"time_zone":"Europe/Rome","working_hours":{"start":"10:00","end":"17:30","weekdays":[1,2,3,4]},"default_account_id":1,"default_calendar_id":"local"})).unwrap();
        assert_eq!(changed["data"]["working_hours"]["start"], "10:00");
        assert_eq!(
            prefs_context.read().unwrap().settings.time_zone,
            "Europe/Rome"
        );
        assert_eq!(
            scheduling_preferences(&prefs_context).unwrap()["weekdays"],
            json!([1, 2, 3, 4])
        );
        let default_draft = call(
            prefs_context.clone(),
            "create_draft",
            &json!({"body":"Uses configured default","idempotency_key":"default-account"}),
        )
        .unwrap();
        assert_eq!(default_draft["data"]["account_id"], "1");
        assert_eq!(call(prefs_context.clone(),"update_preferences",&json!({"expected_revision":before["data"]["revision"],"working_hours":{"end":"08:00"}})).unwrap_err().code,"revision_conflict");
        // Simulate a crash after the preference intent is durable but before Store.
        let old_core = core_preferences(&prefs_context.read().unwrap());
        let mut next_core = old_core.clone();
        next_core["locale"] = json!("it-IT");
        state_update(|s| {
            s.pending_preferences = Some(PreferenceIntent {
                expected_core: hash(&old_core),
                new_core: next_core,
                new_extras: extra_preferences(s),
            });
            Ok(())
        })
        .unwrap();
        let recovered = call(prefs_context.clone(), "get_preferences", &json!({})).unwrap();
        assert_eq!(recovered["data"]["locale"], "it-IT");
        assert!(state_read().unwrap().pending_preferences.is_none());
        let handle = files::write_scoped("private.txt", "text/plain", b"private", &[1]).unwrap();
        let handle = handle["handle"].as_str().unwrap();
        assert_eq!(
            files::read_scoped(handle, 0, 10, Some(&[1])).unwrap()["data"],
            "cHJpdmF0ZQ=="
        );
        assert_eq!(
            files::read_scoped(handle, 0, 10, Some(&[2]))
                .unwrap_err()
                .code,
            "permission_denied"
        );
        let outside = Store::data_dir().parent().unwrap().join("outside.txt");
        std::fs::write(&outside, b"do not import").unwrap();
        assert_eq!(
            files::register(&outside).unwrap_err().code,
            "permission_denied"
        );
    }
    // ---- the activity log (centro attività, Task 6) ----

    #[test]
    fn assistant_batch_is_one_row() {
        let succeeded: Vec<(u64, u32, String)> =
            (1..=80).map(|id| (id, 2, format!("Message {id}"))).collect();
        let entry = assistant_mail_activity("archive", &succeeded).unwrap();
        assert_eq!(entry.code, "archive");
        assert_eq!(entry.count, Some(80));
        assert_eq!((entry.ref_kind, entry.ref_id), (None, None), "80 messages point at none of them");
        assert_eq!(entry.account_id, Some(2));
        assert_eq!(entry.subject, None);
        assert_eq!(entry.source, crate::domain::activity::ActivitySource::Assistant);
        assert_eq!(entry.kind, crate::domain::activity::ActivityKind::Action);
        assert_eq!(entry.outcome, crate::domain::activity::ActivityOutcome::Done);
    }

    #[test]
    fn assistant_single_action_points_at_the_message() {
        let entry = assistant_mail_activity("markAsDone", &[(8, 1, "Topic".into())]).unwrap();
        assert_eq!(entry.count, None);
        assert_eq!(entry.subject.as_deref(), Some("Topic"));
        assert_eq!((entry.ref_kind.as_deref(), entry.ref_id), (Some("email"), Some(8)));
        assert_eq!(entry.account_id, Some(1));
    }

    #[test]
    fn assistant_batch_across_accounts_names_no_account() {
        let entry = assistant_mail_activity(
            "moveToTrash",
            &[(1, 1, "A".into()), (2, 2, "B".into())],
        )
        .unwrap();
        assert_eq!((entry.count, entry.account_id), (Some(2), None));
    }

    #[test]
    fn assistant_unlogged_or_failed_actions_write_nothing() {
        assert!(assistant_mail_activity("pin", &[(8, 1, "Topic".into())]).is_none());
        assert!(assistant_mail_activity("markAsSeen", &[(8, 1, "Topic".into())]).is_none());
        assert!(assistant_mail_activity("archive", &[]).is_none(), "nothing succeeded");
    }

    #[test]
    fn a_discarded_draft_names_the_draft() {
        let draft = Draft { id: 31, account_id: 1, subject: "Offerta".into(), ..Default::default() };
        let entry = assistant_draft_activity(&draft);
        assert_eq!(entry.code, "discardDraft");
        assert_eq!(entry.subject.as_deref(), Some("Offerta"));
        assert_eq!((entry.ref_kind.as_deref(), entry.ref_id), (Some("draft"), Some(31)));
        assert_eq!(entry.account_id, Some(1));
        assert_eq!(entry.source, crate::domain::activity::ActivitySource::Assistant);
        let untitled = assistant_draft_activity(&Draft { id: 32, account_id: 1, ..Default::default() });
        assert_eq!(untitled.subject, None, "an empty subject is no subject");
    }

    fn logged(ctx: &ServiceContext) -> Vec<crate::domain::activity::ActivityEntry> {
        ctx.repository
            .activity_page(None, 50, crate::domain::activity::ActivityFilter::All)
            .unwrap()
    }

    #[test]
    fn email_action_through_the_tool_writes_one_assistant_row() {
        let ctx = context();
        let result = call(ctx.clone(), "email_action", &json!({"ids":["8"],"action":"markAsDone"})).unwrap();
        assert_eq!(result["data"]["items"][0]["status"], "succeeded", "{result}");
        let rows = logged(&ctx);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].code, "markAsDone");
        assert_eq!(rows[0].source, crate::domain::activity::ActivitySource::Assistant);
        assert_eq!((rows[0].ref_id, rows[0].account_id), (Some(8), Some(1)));
        assert_eq!(rows[0].subject.as_deref(), Some("Topic"));
    }

    #[test]
    fn email_action_that_is_not_logged_or_fails_writes_nothing() {
        let ctx = context();
        call(ctx.clone(), "email_action", &json!({"ids":["8"],"action":"markAsSeen"})).unwrap();
        assert!(logged(&ctx).is_empty(), "read state is not history");
        let missing = call(ctx.clone(), "email_action", &json!({"ids":["999"],"action":"markAsDone"}));
        assert!(missing.is_err() || logged(&ctx).is_empty(), "an unknown message leaves no row");
    }

    #[test]
    fn delete_draft_through_the_tool_writes_one_assistant_row() {
        let ctx = context();
        let created = call(ctx.clone(), "create_draft", &json!({"account_id":1,"to":["x@example.com"],"subject":"Offerta","body":"Hi","idempotency_key":"d-1"})).unwrap();
        let id = created["data"]["id"].clone();
        let revision = created["data"]["revision"].clone();
        call(ctx.clone(), "delete_draft", &json!({"id":id,"expected_revision":revision})).unwrap();
        let rows = logged(&ctx);
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].code, "discardDraft");
        assert_eq!(rows[0].subject.as_deref(), Some("Offerta"));
    }
}
