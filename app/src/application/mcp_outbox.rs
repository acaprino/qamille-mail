//! Durable outbox use cases. Provider I/O happens after an atomic, persisted claim.
use super::{AppError, AppResult, ServiceContext, drafts::DraftService};
use crate::domain::{
    model::{Access, Draft},
    outbox::{Delivery, DeliveryStatus as Status, RemoteDraft},
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;

fn number(a: &Value, key: &str) -> AppResult<u64> {
    a[key]
        .as_u64()
        .or_else(|| a[key].as_str().and_then(|s| s.parse().ok()))
        .ok_or_else(|| AppError::validation(format!("{key} is required")))
}
fn send_time(value: &str, settings: &crate::model::Settings) -> AppResult<i64> {
    if let Ok(d) = chrono::DateTime::parse_from_rfc3339(value) {
        return Ok(d.timestamp());
    }
    let d = chrono::NaiveDateTime::parse_from_str(value, "%Y-%m-%dT%H:%M")
        .map_err(|_| AppError::validation("Use RFC3339 or YYYY-MM-DDTHH:MM"))?;
    let zone = if settings.time_zone.is_empty() {
        "UTC"
    } else {
        &settings.time_zone
    };
    crate::domain::regional::local_timestamp(d, zone).map_err(AppError::validation)
}
fn visible(context: &ServiceContext, operation: &Delivery) -> AppResult<()> {
    if context
        .read()?
        .account(operation.draft.account_id)
        .is_none()
    {
        return Err(AppError::missing("Operation"));
    }
    Ok(())
}
fn public(
    operation: &Delivery,
    attachments: &dyn super::ports::AttachmentGateway,
) -> AppResult<Value> {
    let id = match &operation.receipt_message_id {
        Some(id) => id.clone(),
        None => message_id(&operation.draft, attachments)?,
    };
    Ok(
        json!({"id":operation.id.to_string(),"operation_id":operation.id.to_string(),"message_id":id,"draft_id":operation.draft.id.to_string(),"account_id":operation.draft.account_id.to_string(),"status":operation.status,"created_at":operation.created_at,"updated_at":operation.updated_at,"send_at":operation.send_at,"error":operation.error,"can_cancel":matches!(operation.status,Status::Queued|Status::Scheduled|Status::NeedsReview),"retryable":operation.status==Status::Failed}),
    )
}

pub fn enqueue(
    context: Arc<ServiceContext>,
    id: u64,
    revision: Option<i64>,
    key: Option<&str>,
    when: Option<i64>,
    requires_mcp_send: bool,
    source_profile: Option<&str>,
) -> AppResult<Delivery> {
    enqueue_inner(context, id, revision, key, when, requires_mcp_send, source_profile, false)
}

/// A scheduled draft from an older desktop writes its durable delivery in the same
/// transaction that consumes its due date. Restart can always find one or the other.
pub fn enqueue_due(context: Arc<ServiceContext>, draft: &Draft) -> AppResult<Delivery> {
    let key = format!("scheduled-draft:{}:{}", draft.id, draft.updated);
    enqueue_inner(context, draft.id, Some(draft.updated), Some(&key), None, false, None, true)
}

#[allow(clippy::too_many_arguments)]
fn enqueue_inner(
    context: Arc<ServiceContext>, id: u64, revision: Option<i64>, key: Option<&str>,
    when: Option<i64>, requires_mcp_send: bool, source_profile: Option<&str>, consume_due: bool,
) -> AppResult<Delivery> {
    let now = context.clock.now();
    // Resolve an existing request before looking up a draft removed by a successful send.
    if let Some(key) = key {
        if let Some(existing) = context.read()?.outbox.into_iter().find(|o| o.key == key) {
            visible(&context, &existing)?;
            if existing.draft.id != id
                || revision.is_some_and(|r| r != existing.draft.updated)
                || when != existing.requested_send_at
            {
                return Err(AppError::new(
                    "idempotency_conflict",
                    "The key was already used for another delivery",
                ));
            }
            return Ok(existing);
        }
    }
    DraftService::new(context.clone()).validate_send(id)?;
    let source = context
        .read()?
        .drafts
        .into_iter()
        .find(|d| d.id == id)
        .ok_or_else(|| AppError::missing("Draft"))?;
    let snapshot_paths = context
        .attachments
        .snapshot_outgoing(&source.attachments, &format!("{id}:{}", source.updated))?;
    let attachment_bytes = context.attachments.total_size(&snapshot_paths)?;
    if when.is_some_and(|w| w < now || w > now + crate::rules::MAX_DAYS_SEND_LATER * 86_400) {
        return Err(AppError::validation("Choose a future time within 180 days"));
    }
    let queued = context.update_with_events(|store| {
        let mut draft = store
            .drafts
            .iter()
            .find(|d| d.id == id)
            .cloned()
            .ok_or_else(|| AppError::missing("Draft"))?;
        if draft.updated != source.updated {
            return Err(AppError::new(
                "revision_conflict",
                "Draft changed while preparing its attachments",
            ));
        }
        if revision.is_some_and(|r| r != draft.updated) {
            return Err(AppError::new(
                "revision_conflict",
                "The draft changed; read it again before sending",
            ));
        }
        if consume_due && !draft.send_at.is_some_and(|at| at <= now) {
            return Err(AppError::new("revision_conflict", "The draft schedule changed"));
        }
        let key = key
            .map(str::to_owned)
            .unwrap_or_else(|| format!("draft:{id}:{}", draft.updated));
        let hash = format!(
            "{:x}",
            Sha256::digest(serde_json::to_vec(&(
                identity_draft(&draft, context.attachments.as_ref())?,
                when
            ))?)
        );
        if let Some(previous) = store.outbox.iter().find(|o| o.key == key) {
            if previous.request_hash != hash {
                return Err(AppError::new(
                    "idempotency_conflict",
                    "The key was already used with different content",
                ));
            }
            return Ok((previous.clone(), Vec::new()));
        }
        if store.outbox.iter().any(|o| {
            o.draft.id == id
                && matches!(
                    o.status,
                    Status::Queued
                        | Status::Scheduled
                        | Status::Sending
                        | Status::Unknown
                        | Status::SentNotSaved
                        | Status::NeedsReview
                )
        }) {
            return Err(AppError::new(
                "delivery_pending",
                "This draft already has an active or uncertain delivery. Check the outbox.",
            ));
        }
        // Cleanup and enqueue share the store's write lock. If a previously unreferenced
        // snapshot was removed after preparation, no inaccessible attachment is committed.
        context.attachments.total_size(&snapshot_paths)?;
        let mut events = Vec::new();
        if consume_due {
            let current = store.drafts.iter_mut().find(|current| current.id == id)
                .ok_or_else(|| AppError::missing("Draft"))?;
            current.send_at = None;
            events.push(super::events::AppEvent::DraftsChanged { ids: vec![id] });
        }
        draft.attachments = snapshot_paths;
        let rendered = crate::domain::richtext::render_draft(store, &draft);
        let operation = Delivery {
            id: store.new_id(),
            key,
            request_hash: hash,
            receipt_message_id: None,
            draft,
            plain: rendered.plain,
            html: rendered.html,
            sender_name: store.settings.user_name.clone(),
            attachment_bytes,
            status: if when.is_some() {
                Status::Scheduled
            } else {
                Status::Queued
            },
            created_at: now,
            updated_at: now,
            send_at: when.unwrap_or(now),
            requested_send_at: when,
            error: None,
            owner_pid: None,
            requires_mcp_send,
            source_profile: source_profile.map(str::to_owned),
        };
        store.outbox.push(operation.clone());
        Ok((operation, events))
    })?;
    // A queued or scheduled delivery is a new deadline for the scheduler
    // thread, which no longer polls (INV-005): note it after the commit.
    super::scheduler::note_schedule_change();
    Ok(queued)
}

pub fn execute(context: Arc<ServiceContext>, operation_id: u64) -> AppResult<Delivery> {
    let now = context.clock.now();
    let initial = context.read()?;
    let existing = initial.outbox.iter().find(|o| o.id == operation_id).cloned()
        .ok_or_else(|| AppError::missing("Operation"))?;
    let has_source = |store: &crate::model::Store| {
        store.account(existing.draft.account_id).is_some() && store.drafts.iter().any(|draft|
            draft.id == existing.draft.id && draft.account_id == existing.draft.account_id)
    };
    if matches!(existing.status, Status::Queued | Status::Scheduled) && !has_source(&initial) {
        return context.update(|store| {
            let orphan = !has_source(store);
            let operation = store.outbox.iter_mut().find(|o| o.id == operation_id)
                .ok_or_else(|| AppError::missing("Operation"))?;
            if orphan && matches!(operation.status, Status::Queued | Status::Scheduled) {
                operation.status = Status::NeedsReview;
                operation.error = Some("The original mailbox or draft is gone. Review the delivery before sending.".into());
                operation.updated_at = now;
            }
            Ok(operation.clone())
        });
    }
    visible(&context, &existing)?;
    if !matches!(existing.status, Status::Queued | Status::Scheduled) {
        return Ok(existing);
    }
    if existing.requires_mcp_send {
        if let Err(error) = context.mail.authorize_mcp_delivery(
            existing.source_profile.as_deref(),
            existing.draft.account_id,
        ) {
            context.update(|store| {
                if let Some(o) = store.outbox.iter_mut().find(|o| o.id == operation_id) {
                    if matches!(o.status, Status::Queued | Status::Scheduled) {
                        o.status = Status::NeedsReview;
                        o.error = Some(error.message.clone());
                        o.updated_at = now;
                    }
                }
                Ok(())
            })?;
            return Err(error);
        }
    }
    // Validate credentials/attachments before claiming; no network request has started yet.
    let snapshot = context.read()?;
    let preflight = context
        .account(&snapshot, existing.draft.account_id)
        .map(|_| ())
        .and_then(|_| {
            context
                .attachments
                .total_size(&existing.draft.attachments)
                .map(|_| ())
        });
    if let Err(error) = preflight {
        if error.is_vault() && existing.key == format!("scheduled-draft:{}:{}", existing.draft.id, existing.draft.updated)
            && existing.draft.send_at.is_some() && !existing.requires_mcp_send {
            context.update_with_events(|store| {
                let Some(index) = store.outbox.iter().position(|o| o.id == operation_id && o.status == Status::Queued) else {
                    return Ok(((), Vec::new()));
                };
                let Some(draft) = store.drafts.iter_mut().find(|d| d.id == existing.draft.id
                    && d.updated == existing.draft.updated && d.send_at.is_none()) else {
                    return Ok(((), Vec::new()));
                };
                draft.send_at = existing.draft.send_at;
                store.outbox.remove(index);
                Ok(((), vec![super::events::AppEvent::DraftsChanged { ids: vec![existing.draft.id] }]))
            })?;
            return Err(error);
        }
        context.update(|store| {
            if let Some(o) = store.outbox.iter_mut().find(|o| o.id == operation_id) {
                if matches!(o.status, Status::Queued | Status::Scheduled) {
                    o.status = Status::Failed;
                    o.error = Some(format!("Delivery has not started: {}", error.message));
                    o.updated_at = now;
                }
            }
            Ok(())
        })?;
        return Err(error);
    }
    let receipt_id = message_id(&existing.draft, context.attachments.as_ref())?;
    let claimed = context.update(|store| {
        let original_present = has_source(store);
        let access = store.settings.mcp_access;
        let o = store
            .outbox
            .iter_mut()
            .find(|o| o.id == operation_id)
            .ok_or_else(|| AppError::missing("Operation"))?;
        if !matches!(o.status, Status::Queued | Status::Scheduled) || o.send_at > now {
            return Ok(None);
        }
        if !original_present {
            o.status = Status::NeedsReview;
            o.error = Some("The original mailbox or draft is gone. Review the delivery before sending.".into());
            o.updated_at = now;
            return Ok(None);
        }
        if o.requires_mcp_send && access != Access::Send {
            o.status = Status::NeedsReview;
            o.error = Some("Send permission was revoked".into());
            o.updated_at = now;
            return Ok(None);
        }
        if o.status == Status::Scheduled && now.saturating_sub(o.send_at) > 900 {
            o.status = Status::NeedsReview;
            o.error =
                Some("Scheduled delivery is over 15 minutes late; review it explicitly".into());
            o.updated_at = now;
            return Ok(None);
        }
        o.status = Status::Sending;
        o.owner_pid = Some(std::process::id());
        o.updated_at = now;
        Ok(Some(o.draft.clone()))
    })?;
    if let Some(draft) = claimed {
        let result = DraftService::new(context.clone()).deliver_snapshot(
            &draft,
            &existing.plain,
            existing.html.clone(),
            &existing.sender_name,
        );
        let (status, error) = match &result {
            Ok(()) => (Status::Sent, None),
            Err(e) if e.code == "delivery_not_started" => (Status::Failed, Some(e.message.clone())),
            Err(e) if e.code == "sent_not_saved" => (Status::SentNotSaved, Some(e.message.clone())),
            Err(e) => (
                Status::Unknown,
                Some(format!(
                    "Delivery result is uncertain; do not resend automatically. {}",
                    e.message
                )),
            ),
        };
        context.update(|store| {
            let o=store.outbox.iter_mut().find(|o| o.id==operation_id).ok_or_else(||AppError::missing("Operation"))?;
            o.status = status;
            o.error = error;
            o.updated_at = context.clock.now();
            if o.status == Status::Sent {
                o.compact(receipt_id);
            }
            Ok(())
        }).map_err(|e| AppError::new("delivery_record_failed",format!("Delivery was attempted but its result could not be saved. Do not resend. {}",e.message)))?;
        // Preserve existing application callers' error semantics while the durable record
        // separately communicates whether a remote effect is uncertain.
        result?;
    }
    context
        .read()?
        .outbox
        .into_iter()
        .find(|o| o.id == operation_id)
        .ok_or_else(|| AppError::missing("Operation"))
}

pub(crate) fn retry_failed(context: &ServiceContext, id: u64) -> AppResult<Delivery> {
    context.update(|store| {
        let operation = store.outbox.iter_mut().find(|operation| operation.id == id)
            .ok_or_else(|| AppError::missing("Operation"))?;
        if operation.status != Status::Failed {
            return Err(AppError::new("review_required", "Only a delivery that did not start may be retried."));
        }
        operation.status = Status::Queued;
        operation.send_at = context.clock.now();
        operation.updated_at = operation.send_at;
        operation.error = None;
        Ok(operation.clone())
    })
}

pub fn tick(context: Arc<ServiceContext>) -> AppResult<()> {
    let now = context.clock.now();
    let pending: Vec<_> = context
        .read()?
        .outbox
        .into_iter()
        .filter(|o| matches!(o.status, Status::Queued | Status::Scheduled) && o.send_at <= now)
        .map(|o| o.id)
        .collect();
    for id in pending {
        let _ = execute(context.clone(), id);
    }
    // An expired claim is uncertain, never eligible for an automatic retry.
    if context
        .read()?
        .outbox
        .iter()
        .any(|o| o.status == Status::Sending && now.saturating_sub(o.updated_at) > 600)
    {
        context.update(|store| {
            for o in &mut store.outbox {
                if o.status == Status::Sending && now.saturating_sub(o.updated_at) > 600 {
                    o.status = Status::Unknown;
                    o.updated_at = now;
                    o.error = Some(
                        "Delivery claim expired; verify the provider before sending again".into(),
                    );
                }
            }
            Ok(())
        })?;
    }
    Ok(())
}

pub fn handles(name: &str) -> bool {
    matches!(
        name,
        "send_draft"
            | "send_email"
            | "schedule_draft"
            | "cancel_scheduled_send"
            | "list_outbox"
            | "get_operation"
            | "list_operations"
            | "cancel_operation"
            | "retry_operation"
            | "sync_drafts"
    )
}
pub fn required_access(name: &str) -> Access {
    if matches!(name, "list_outbox" | "get_operation" | "list_operations") {
        Access::ReadOnly
    } else if name == "sync_drafts" {
        Access::Triage
    } else {
        Access::Send
    }
}
pub fn catalog() -> Vec<Value> {
    let id = json!({"type":["string","integer"]});
    let text = json!({"type":"string"});
    let mut tools = Vec::new();
    for name in [
        "send_draft",
        "schedule_draft",
        "cancel_scheduled_send",
        "get_operation",
        "cancel_operation",
        "retry_operation",
        "list_outbox",
        "list_operations",
        "sync_drafts",
        "send_email",
    ] {
        let (props, required, description) = match name {
            "send_draft" | "schedule_draft" => (
                json!({"id":id,"expected_revision":{"type":"integer"},"idempotency_key":text,"date":text}),
                vec!["id", "expected_revision", "idempotency_key"],
                "Send or schedule an immutable draft revision through the durable outbox. Returns delivery state, not a delivery receipt. date is RFC3339 or an app-local date.",
            ),
            "get_operation" | "cancel_operation" | "retry_operation" | "cancel_scheduled_send" => (
                json!({"id":id}),
                vec!["id"],
                "Read/cancel a delivery operation. Cancellation is possible only before the provider attempt. Uncertain sends cannot be retried.",
            ),
            "sync_drafts" => (
                json!({"account_id":id,"direction":{"type":"string","enum":["pull","push","both"]}}),
                vec!["account_id"],
                "Synchronize cloud drafts using the account provider. Conflicting local and remote changes are preserved and reported.",
            ),
            "send_email" => (
                json!({"account_id":id,"to":{"type":["string","array"],"items":text},"cc":{"type":["string","array"],"items":text},"bcc":{"type":["string","array"],"items":text},"subject":text,"body":text,"html":text,"format":{"type":"string","enum":["plain","html","rich"]},"signature_id":{"type":["string","integer","null"]},"attachments":{"type":"array","items":text,"maxItems":100},"idempotency_key":text}),
                vec!["account_id", "subject", "body", "idempotency_key"],
                "Create a durable draft and send through the same outbox pipeline. Repeating the same key never creates another delivery.",
            ),
            _ => (
                json!({"account_id":id,"offset":{"type":"integer","minimum":0},"limit":{"type":"integer","minimum":1,"maximum":100}}),
                vec![],
                "List durable deliveries, including scheduled, sent, failed and uncertain states.",
            ),
        };
        tools.push(json!({"name":name,"description":description,"inputSchema":{"type":"object","properties":props,"required":required,"additionalProperties":false},"outputSchema":{"type":"object"},"annotations":{"readOnlyHint":required_access(name)==Access::ReadOnly,"destructiveHint":required_access(name)==Access::Send,"openWorldHint":true}}));
    }
    tools
}

pub fn call(context: Arc<ServiceContext>, name: &str, args: &Value) -> AppResult<Value> {
    match name {
        "send_draft" | "schedule_draft" => {
            let id = number(args, "id")?;
            let revision = args["expected_revision"]
                .as_i64()
                .ok_or_else(|| AppError::validation("expected_revision is required"))?;
            let key = args["idempotency_key"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AppError::validation("idempotency_key is required"))?;
            let when = args["date"]
                .as_str()
                .map(|s| {
                    let mut settings = context.read()?.settings;
                    settings.time_zone = context.clock.regional(&settings).time_zone;
                    send_time(s, &settings)
                })
                .transpose()?;
            if name == "schedule_draft" && when.is_none() {
                return Err(AppError::validation("date is required"));
            }
            let operation = enqueue(
                context.clone(),
                id,
                Some(revision),
                Some(key),
                when,
                true,
                args["_source_profile"].as_str(),
            )?;
            let current = context
                .read()?
                .outbox
                .into_iter()
                .find(|o| o.id == operation.id)
                .ok_or_else(|| AppError::missing("Operation"))?;
            public(&current, context.attachments.as_ref())
        }
        "send_email" => {
            let key = args["idempotency_key"]
                .as_str()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| AppError::validation("idempotency_key is required"))?;
            let mut canonical = args.clone();
            if let Some(object) = canonical.as_object_mut() {
                object.remove("_attachment_paths");
            }
            let payload_hash = format!("{:x}", Sha256::digest(serde_json::to_vec(&canonical)?));
            let request_id = format!("send:{key}:{payload_hash}");
            let account = u32::try_from(number(args, "account_id")?)
                .map_err(|_| AppError::validation("Invalid account_id"))?;
            if let Some(existing) = context.read()?.outbox.into_iter().find(|o| o.key == key) {
                visible(&context, &existing)?;
                if existing.draft.request_id != request_id {
                    return Err(AppError::new(
                        "idempotency_conflict",
                        "Key already used with other message content",
                    ));
                }
                return public(&existing, context.attachments.as_ref());
            }
            let format = if args["html"].is_string() {
                crate::model::BodyFormat::Html
            } else {
                match args["format"].as_str().unwrap_or("plain") {
                    "plain" => crate::model::BodyFormat::Plain,
                    "html" => crate::model::BodyFormat::Html,
                    "rich" => crate::model::BodyFormat::Rich,
                    _ => return Err(AppError::validation("format must be plain, html or rich")),
                }
            };
            let attachment_handles = args["attachments"].as_array().map_or(0, Vec::len);
            let attachment_paths: Vec<String> = args["_attachment_paths"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect();
            if attachment_handles != attachment_paths.len() {
                return Err(AppError::validation(
                    "Attachments must be authorized managed file handles",
                ));
            }
            let draft = context.update(|store| {
                if store.account(account).is_none() {
                    return Err(AppError::missing("Account"));
                }
                if let Some(o) = store.outbox.iter().find(|o| o.key == key) {
                    if o.draft.request_id != request_id {
                        return Err(AppError::new(
                            "idempotency_conflict",
                            "Key already used with other message content",
                        ));
                    }
                    return Ok(o.draft.clone());
                }
                if let Some(d) = store.drafts.iter().find(|d| d.request_id == request_id) {
                    return Ok(d.clone());
                }
                let list = |k: &str| -> String {
                    args[k].as_str().map(str::to_owned).unwrap_or_else(|| {
                        args[k]
                            .as_array()
                            .map(|v| {
                                v.iter()
                                    .filter_map(Value::as_str)
                                    .collect::<Vec<_>>()
                                    .join(", ")
                            })
                            .unwrap_or_default()
                    })
                };
                let signature_id = if args.get("signature_id").is_some() {
                    if args["signature_id"].is_null() {
                        None
                    } else {
                        let id = number(args, "signature_id")?;
                        if store.signature(id).is_none() {
                            return Err(AppError::missing("Signature"));
                        }
                        Some(id)
                    }
                } else {
                    store.signature_for_new(account)
                };
                let d = Draft {
                    id: store.new_id(),
                    account_id: account,
                    to: list("to"),
                    cc: list("cc"),
                    bcc: list("bcc"),
                    subject: args["subject"].as_str().unwrap_or("").into(),
                    body: args["html"]
                        .as_str()
                        .or_else(|| args["body"].as_str())
                        .unwrap_or("")
                        .into(),
                    format,
                    attachments: attachment_paths,
                    updated: context.clock.now(),
                    request_id,
                    signature_id,
                    ..Default::default()
                };
                store.drafts.push(d.clone());
                Ok(d)
            })?;
            let op = enqueue(
                context.clone(),
                draft.id,
                Some(draft.updated),
                Some(key),
                None,
                true,
                args["_source_profile"].as_str(),
            )?;
            public(&op, context.attachments.as_ref())
        }
        "list_outbox" | "list_operations" => {
            let store = context.read()?;
            let allowed: Vec<_> = store.accounts.iter().map(|a| a.id).collect();
            let account = args["account_id"]
                .as_u64()
                .or_else(|| args["account_id"].as_str().and_then(|s| s.parse().ok()));
            let items: Vec<_> = store
                .outbox
                .iter()
                .filter(|o| {
                    allowed.contains(&o.draft.account_id)
                        && account.is_none_or(|a| a == u64::from(o.draft.account_id))
                })
                .rev()
                .collect();
            let total = items.len();
            let offset = args["offset"].as_u64().unwrap_or(0) as usize;
            let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
            let items = items
                .into_iter()
                .skip(offset)
                .take(limit)
                .map(|operation| public(operation, context.attachments.as_ref()))
                .collect::<AppResult<Vec<_>>>()?;
            Ok(
                json!({"items":items,"total":total,"next_offset":(offset+limit<total).then_some(offset+limit)}),
            )
        }
        "get_operation" => {
            let id = number(args, "id")?;
            let o = context
                .read()?
                .outbox
                .into_iter()
                .find(|o| o.id == id)
                .ok_or_else(|| AppError::missing("Operation"))?;
            visible(&context, &o)?;
            public(&o, context.attachments.as_ref())
        }
        "cancel_scheduled_send" | "cancel_operation" => {
            let id = number(args, "id")?;
            let existing = context
                .read()?
                .outbox
                .into_iter()
                .find(|o| o.id == id)
                .ok_or_else(|| AppError::missing("Operation"))?;
            visible(&context, &existing)?;
            let o = context.update(|store| {
                let o = store
                    .outbox
                    .iter_mut()
                    .find(|o| o.id == id)
                    .ok_or_else(|| AppError::missing("Operation"))?;
                if !matches!(
                    o.status,
                    Status::Queued | Status::Scheduled | Status::NeedsReview
                ) {
                    return Err(AppError::new(
                        "cannot_cancel",
                        "Delivery has already started or finished",
                    ));
                }
                let message_id = message_id(&o.draft, context.attachments.as_ref())?;
                o.status = Status::Cancelled;
                o.compact(message_id);
                o.updated_at = context.clock.now();
                Ok(o.clone())
            })?;
            public(&o, context.attachments.as_ref())
        }
        "retry_operation" => {
            let id = number(args, "id")?;
            let existing = context
                .read()?
                .outbox
                .into_iter()
                .find(|o| o.id == id)
                .ok_or_else(|| AppError::missing("Operation"))?;
            visible(&context, &existing)?;
            let o = retry_failed(&context, id)?;
            // The retry is due immediately: the deadline moved (INV-005).
            super::scheduler::note_schedule_change();
            public(&o, context.attachments.as_ref())
        }
        "sync_drafts" => sync_drafts(context, args),
        _ => Err(AppError::new("unknown_tool", "Unknown outbox tool")),
    }
}

fn sync_drafts(context: Arc<ServiceContext>, args: &Value) -> AppResult<Value> {
    let id = u32::try_from(number(args, "account_id")?)
        .map_err(|_| AppError::validation("Invalid account_id"))?;
    let store = context.read()?;
    let account = context.account(&store, id)?;
    if !context.mail.capabilities(&account).remote_drafts {
        return Err(AppError::new(
            "unsupported_capability",
            "This account has no cloud draft support; local drafts remain available",
        ));
    }
    let direction = args["direction"].as_str().unwrap_or("both");
    if !["both", "pull", "push"].contains(&direction) {
        return Err(AppError::validation("direction must be pull, push or both"));
    }
    let remote = context.mail.list_remote_drafts(&account)?;
    let mut pulled = 0;
    let mut pushed = 0;
    let mut conflicts = Vec::new();
    for item in &remote {
        let existing = context.read()?.drafts.into_iter().find(|d| {
            d.account_id == id
                && (d.remote_id.as_deref() == Some(&item.id)
                    || (!d.cloud_sync_key.is_empty()
                        && d.cloud_sync_key == item.draft.cloud_sync_key))
        });
        if let Some(local) = existing {
            if context.read()?.outbox.iter().any(|o| {
                o.draft.id == local.id
                    && matches!(
                        o.status,
                        Status::Queued
                            | Status::Scheduled
                            | Status::Sending
                            | Status::Unknown
                            | Status::SentNotSaved
                            | Status::NeedsReview
                    )
            }) {
                conflicts.push(json!({"draft_id":local.id.to_string(),"remote_id":item.id,"reason":"delivery_pending","message":"Cancel or reconcile delivery before synchronizing this draft"}));
                continue;
            }
            if local.remote_revision.as_deref() != Some(&item.revision) {
                // The last synchronized content hash identifies independent local edits.
                if (local.cloud_sync_pending && local.cloud_content_hash.is_empty())
                    || (!local.cloud_content_hash.is_empty()
                        && local.cloud_content_hash
                            != draft_hash(&local, context.attachments.as_ref())?)
                {
                    conflicts.push(json!({"draft_id":local.id.to_string(),"remote_id":item.id,"local_revision":local.updated,"remote_revision":item.revision,"remote_content":item.draft}));
                    continue;
                }
                if direction != "push" {
                    context.update(|store| {
                        let mut d = item.draft.clone();
                        d.id = local.id;
                        d.request_id = local.request_id.clone();
                        d.account_id = id;
                        d.updated = context.clock.now().max(local.updated + 1);
                        d.remote_id = Some(item.id.clone());
                        d.remote_revision = Some(item.revision.clone());
                        d.cloud_content_hash = draft_hash(&d, context.attachments.as_ref())?;
                        d.cloud_sync_pending = false;
                        let current = store
                            .drafts
                            .iter_mut()
                            .find(|d| d.id == local.id)
                            .ok_or_else(|| AppError::missing("Draft"))?;
                        if current.updated != local.updated {
                            return Err(AppError::new(
                                "revision_conflict",
                                "Draft changed during synchronization",
                            ));
                        }
                        *current = d;
                        Ok(())
                    })?;
                    pulled += 1;
                }
            }
        } else if direction != "push" {
            context.update(|store| {
                let mut d = item.draft.clone();
                d.id = store.new_id();
                d.account_id = id;
                d.updated = context.clock.now();
                d.remote_id = Some(item.id.clone());
                d.remote_revision = Some(item.revision.clone());
                d.cloud_content_hash = draft_hash(&d, context.attachments.as_ref())?;
                d.cloud_sync_pending = false;
                store.drafts.push(d);
                Ok(())
            })?;
            pulled += 1;
        }
    }
    if direction != "pull" {
        for draft in context
            .read()?
            .drafts
            .into_iter()
            .filter(|d| d.account_id == id)
        {
            if context.read()?.outbox.iter().any(|o| {
                o.draft.id == draft.id
                    && matches!(
                        o.status,
                        Status::Queued
                            | Status::Scheduled
                            | Status::Sending
                            | Status::Unknown
                            | Status::SentNotSaved
                            | Status::NeedsReview
                    )
            }) {
                conflicts.push(json!({"draft_id":draft.id.to_string(),"reason":"delivery_pending","message":"Cancel or reconcile delivery before synchronizing this draft"}));
                continue;
            }
            if conflicts
                .iter()
                .any(|c| c["draft_id"] == draft.id.to_string())
            {
                continue;
            }
            if draft.remote_id.is_some()
                && draft.cloud_content_hash == draft_hash(&draft, context.attachments.as_ref())?
            {
                continue;
            }
            // Finish fallible local preparation before claiming a remote write.
            let rendered = crate::domain::richtext::render_draft(&context.read()?, &draft);
            let previous_content_hash = draft.cloud_content_hash.clone();
            let draft = context.update(|store| {
                let current = store.drafts.iter_mut().find(|d| d.id == draft.id)
                    .ok_or_else(|| AppError::missing("Draft"))?;
                if current.updated != draft.updated {
                    return Err(AppError::new("revision_conflict", "Draft changed before cloud save"));
                }
                if current.cloud_sync_pending {
                    return Err(AppError::new("cloud_sync_unknown", "A previous cloud save has an uncertain result. Synchronize remote drafts before retrying."));
                }
                if current.cloud_sync_key.is_empty() {
                    current.cloud_sync_key = format!("{}:{}", id, current.id);
                }
                // Keep a baseline before the first provider attempt, including
                // when its response is lost after the cloud draft was created.
                if current.cloud_content_hash.is_empty() {
                    current.cloud_content_hash = draft_hash(current, context.attachments.as_ref())?;
                }
                current.cloud_sync_pending = true;
                Ok(current.clone())
            })?;
            let mut remote_content = draft.clone();
            remote_content.body = rendered.html.clone().unwrap_or(rendered.plain);
            remote_content.format = if rendered.html.is_some() {
                crate::model::BodyFormat::Html
            } else {
                crate::model::BodyFormat::Plain
            };
            remote_content.signature_id = None;
            remote_content.quote.clear();
            let remote_draft = RemoteDraft {
                id: draft.remote_id.clone().unwrap_or_default(),
                revision: draft.remote_revision.clone().unwrap_or_default(),
                raw: String::new(),
                thread_id: None,
                draft: remote_content,
            };
            let saved = match context.mail.save_remote_draft(&account, &remote_draft) {
                Ok(saved) => saved,
                Err(error) => {
                    if error.code == "cloud_save_not_started" {
                        // This outcome is a provider guarantee, not an inference
                        // from a timeout or an empty remote listing. Preserve any
                        // newer local edits while releasing this attempt's claim.
                        context.update(|store| {
                            if let Some(current) =
                                store.drafts.iter_mut().find(|d| d.id == draft.id)
                            {
                                if current.account_id == draft.account_id
                                    && current.cloud_sync_key == draft.cloud_sync_key
                                    && current.remote_id == draft.remote_id
                                    && current.remote_revision == draft.remote_revision
                                    && current.cloud_content_hash == draft.cloud_content_hash
                                {
                                    current.cloud_sync_pending = false;
                                    current.cloud_content_hash = previous_content_hash.clone();
                                }
                            }
                            Ok(())
                        })?;
                    }
                    return Err(error);
                }
            };
            context.update(|store| {
                let current = store
                    .drafts
                    .iter_mut()
                    .find(|d| d.id == draft.id)
                    .ok_or_else(|| AppError::missing("Draft"))?;
                if current.account_id != draft.account_id
                    || current.cloud_sync_key != draft.cloud_sync_key
                {
                    return Err(AppError::new(
                        "revision_conflict",
                        "The cloud draft connection changed during synchronization. Reopen the draft before retrying.",
                    ));
                }
                // A newer local edit is valid. Only acknowledge the content
                // actually uploaded so it remains dirty for the next push.
                current.remote_id = Some(saved.id);
                current.remote_revision = Some(saved.revision);
                current.cloud_content_hash = draft_hash(&draft, context.attachments.as_ref())?;
                current.cloud_sync_pending = false;
                Ok(())
            })?;
            pushed += 1;
        }
    }
    Ok(
        json!({"pulled":pulled,"pushed":pushed,"conflicts":conflicts,"provider_id":context.mail.capabilities(&account).provider_id}),
    )
}

fn draft_hash(
    draft: &Draft,
    attachments: &dyn super::ports::AttachmentGateway,
) -> AppResult<String> {
    let draft = identity_draft(draft, attachments)?;
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            &draft.to,
            &draft.cc,
            &draft.bcc,
            &draft.subject,
            &draft.body,
            &draft.attachments,
            draft.format
        ))?)
    ))
}

pub(crate) fn message_id(
    draft: &Draft,
    attachments: &dyn super::ports::AttachmentGateway,
) -> AppResult<String> {
    Ok(crate::domain::outbox::message_id(&identity_draft(
        draft,
        attachments,
    )?))
}

fn identity_draft(
    draft: &Draft,
    attachments: &dyn super::ports::AttachmentGateway,
) -> AppResult<Draft> {
    let mut identity = draft.clone();
    identity.attachments = attachments.persistent_paths(&draft.attachments)?;
    Ok(identity)
}
