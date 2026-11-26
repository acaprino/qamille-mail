use super::{AppError, AppResult, ServiceContext, events::AppEvent, ports::OutgoingMessage};
use crate::domain::{
    model::{Draft, Email},
    richtext, rules,
};
use std::sync::{Arc, Mutex};

pub fn addresses(s: &str) -> Vec<String> {
    s.split([',', ';'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect()
}

#[derive(Clone)]
pub struct DraftService {
    context: Arc<ServiceContext>,
    operation: Arc<Mutex<()>>,
}
impl DraftService {
    pub fn new(context: Arc<ServiceContext>) -> Self {
        Self {
            context,
            operation: Arc::new(Mutex::new(())),
        }
    }
    pub fn list(&self) -> AppResult<Vec<Draft>> {
        let mut drafts = self.context.read_shared()?.drafts.clone();
        drafts.sort_by_key(|d| std::cmp::Reverse(d.updated));
        Ok(drafts)
    }
    pub fn create(
        &self,
        account_id: Option<u32>,
        reply_to: Option<u64>,
        forward: bool,
    ) -> AppResult<Draft> {
        let email = reply_to.map(|id| super::mail::MailService::new(self.context.clone()).read(id)).transpose()?;
        self.context.update_with_events(|store| {
            let account_id = account_id
                .or_else(|| email.as_ref().map(|e| e.account_id))
                .or_else(|| store.accounts.first().map(|a| a.id))
                .ok_or_else(|| AppError::validation("Add an account first"))?;
            if store.account(account_id).is_none() {
                return Err(AppError::missing("Account"));
            }
            let mut draft = Draft {
                id: store.new_id(),
                account_id,
                signature_id: store.signature_for_new(account_id),
                updated: self.context.clock.now(),
                ..Default::default()
            };
            if let Some(email) = email {
                let prefix = if forward { "Fwd:" } else { "Re:" };
                draft.subject = if email
                    .subject
                    .to_lowercase()
                    .starts_with(&prefix.to_lowercase())
                {
                    email.subject.clone()
                } else {
                    format!("{prefix} {}", email.subject)
                };
                draft.quote = format!(
                    "On {}, {} wrote:\n{}",
                    rules::fmt_full(email.date),
                    email.from_addr,
                    email.body
                );
                if !forward {
                    draft.to = email.reply_recipients().join(", ");
                    draft.reply_to = Some(email.id);
                    draft.signature_id = store.signature_for_reply(&email);
                }
            }
            store.drafts.push(draft.clone());
            Ok((
                draft.clone(),
                vec![AppEvent::DraftsChanged {
                    ids: vec![draft.id],
                }],
            ))
        })
    }
    pub fn save(&self, mut draft: Draft) -> AppResult<Draft> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| AppError::storage("Draft operation lock failed"))?;
        self.context.update_with_events(|store| {
            if store.outbox.iter().any(|o|o.draft.id==draft.id && matches!(o.status,crate::domain::outbox::DeliveryStatus::Queued|crate::domain::outbox::DeliveryStatus::Scheduled|crate::domain::outbox::DeliveryStatus::Sending|crate::domain::outbox::DeliveryStatus::Unknown|crate::domain::outbox::DeliveryStatus::SentNotSaved)) {
                return Err(AppError::new("delivery_pending","Cancel the queued delivery before editing this draft"));
            }
            if store.undo_actions.iter().any(|a| {
                a.status != crate::domain::undo::UndoStatus::Failed
                    && a.operation.draft_id() == Some(draft.id)
            }) {
                return Err(AppError::validation(
                    "Undo the pending action before editing this draft",
                ));
            }
            if store.account(draft.account_id).is_none() {
                return Err(AppError::missing("Account"));
            }
            if draft
                .send_at
                .is_some_and(|t| t > self.context.clock.now() + rules::MAX_DAYS_SEND_LATER * 86400)
            {
                return Err(AppError::validation(
                    "Scheduled messages must be within 180 days",
                ));
            }
            let existing = store
                .drafts
                .iter_mut()
                .find(|d| d.id == draft.id)
                .ok_or_else(|| AppError::missing("Draft"))?;
            if existing.updated != draft.updated {
                return Err(AppError::new(
                    "conflict",
                    "This draft changed elsewhere. Reopen it before saving.",
                ));
            }
            if draft.account_id != existing.account_id
                && (existing.cloud_sync_pending || existing.remote_id.is_some())
            {
                return Err(AppError::new(
                    "cloud_draft_account_locked",
                    "This draft is linked to a cloud account or is synchronizing. Create a new draft to send from another account.",
                ));
            }
            // Cloud synchronization owns these fields. An editor opened before a
            // provider request must not erase its claim or its completed result.
            draft.remote_id = existing.remote_id.clone();
            draft.remote_revision = existing.remote_revision.clone();
            draft.cloud_content_hash = existing.cloud_content_hash.clone();
            draft.cloud_sync_key = existing.cloud_sync_key.clone();
            draft.cloud_sync_pending = existing.cloud_sync_pending;
            draft.updated = self.context.clock.now().max(existing.updated + 1);
            *existing = draft.clone();
            Ok((
                draft.clone(),
                vec![AppEvent::DraftsChanged {
                    ids: vec![draft.id],
                }],
            ))
        })
    }
    pub fn discard(&self, id: u64) -> AppResult<()> {
        self.discard_checked(id,None)
    }
    pub fn discard_revision(&self,id:u64,expected_revision:i64)->AppResult<()> {
        self.discard_checked(id,Some(expected_revision))
    }
    fn discard_checked(&self,id:u64,revision:Option<i64>)->AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| AppError::storage("Draft operation lock failed"))?;
        let snapshot=self.context.read()?;
        let draft=snapshot.drafts.iter().find(|d|d.id==id).cloned().ok_or_else(||AppError::missing("Draft"))?;
        let check=|store:&crate::model::Store|->AppResult<()>{
            let d=store.drafts.iter().find(|d|d.id==id).ok_or_else(||AppError::missing("Draft"))?;
            if revision.is_some_and(|r|r!=d.updated)||d.updated!=draft.updated {return Err(AppError::new("revision_conflict","Draft changed before deletion"));}
            if store.outbox.iter().any(|o|o.draft.id==id&&matches!(o.status,crate::domain::outbox::DeliveryStatus::Queued|crate::domain::outbox::DeliveryStatus::Scheduled|crate::domain::outbox::DeliveryStatus::Sending|crate::domain::outbox::DeliveryStatus::Unknown|crate::domain::outbox::DeliveryStatus::SentNotSaved)){return Err(AppError::new("delivery_pending","Cancel or reconcile the delivery before deleting this draft"));}
            if store.undo_actions.iter().any(|a|a.status==crate::domain::undo::UndoStatus::Pending&&a.operation.draft_id()==Some(id)){return Err(AppError::new("action_pending","Undo the pending draft action before deletion"));}
            Ok(())
        };
        check(&snapshot)?;
        if let Some(remote)=&draft.remote_id {let account=self.context.account(&snapshot,draft.account_id)?;self.context.mail.delete_remote_draft(&account,remote)?;}
        self.context.update_with_events(|store| {
            check(store)?;
            store.drafts.retain(|d| d.id != id);
            Ok((
                (),
                vec![AppEvent::DraftsChanged { ids: vec![id] }],
            ))
        })
    }
    pub fn send(&self, id: u64) -> AppResult<()> {
        let operation = super::mcp_outbox::enqueue(self.context.clone(), id, None, None, None, false, None)?;
        if operation.status == crate::domain::outbox::DeliveryStatus::Failed {
            super::mcp_outbox::retry_failed(&self.context, operation.id)?;
        }
        let delivery = super::mcp_outbox::execute(self.context.clone(), operation.id)?;
        use crate::domain::outbox::DeliveryStatus;
        match delivery.status {
            DeliveryStatus::Sent => Ok(()),
            DeliveryStatus::Unknown => Err(AppError::new(
                "delivery_unknown",
                "Delivery is uncertain. Check the provider before sending again.",
            )),
            DeliveryStatus::SentNotSaved => Err(AppError::new(
                "sent_not_saved",
                "Message was sent but its local copy could not be saved. Do not send it again.",
            )),
            _ => Err(AppError::new(
                "delivery_pending",
                "Delivery has not completed. Check the outbox before sending again.",
            )),
        }
    }
    pub(crate) fn deliver_snapshot(&self, draft: &Draft, plain:&str, html:Option<String>,sender_name:&str) -> AppResult<()> {
        let _guard = self
            .operation
            .lock()
            .map_err(|_| AppError::delivery_not_started("Draft operation lock failed"))?;
        let before_send = |error: AppError| AppError::delivery_not_started(error.message);
        let store = self.context.read_shared().map_err(before_send)?;
        let id = draft.id;
        let draft = draft.clone();
        let to = self.validate(&draft).map_err(before_send)?;
        let account = self.context.account(&store, draft.account_id).map_err(before_send)?;
        let rendered = richtext::Rendered { plain:plain.to_string(), html };
        let message_id = super::mcp_outbox::message_id(&draft, self.context.attachments.as_ref()).map_err(before_send)?;
        // Snapshot references are immutable and must be readable before sending.
        // Keep their original filenames and bytes available to local Sent/export.
        let mut attachments = Vec::new();
        let mut digests = Vec::new();
        for path in &draft.attachments {
            let name = std::path::Path::new(path).file_name().and_then(|n| n.to_str())
                .ok_or_else(|| AppError::delivery_not_started("Attachment filename is invalid"))?.to_string();
            let mut attachment = crate::model::Attachment { name: name.clone(), mime: "application/octet-stream".into(), path: Some(path.clone()), ..Default::default() };
            let bytes = self.context.attachments.read_bytes(&attachment).map_err(before_send)?;
            attachment.size = bytes.len() as u64;
            use sha2::{Digest, Sha256};
            digests.push((name, format!("{:x}", Sha256::digest(&bytes))));
            attachments.push(attachment);
        }
        let mut sent = Email {
            account_id: account.id, folder: "Sent".into(), message_id: message_id.clone(), local_sent: true,
            from_name: sender_name.to_string(), from_addr: account.email.clone(), to: to.clone(), cc: addresses(&draft.cc),
            subject: draft.subject.clone(), body: rendered.plain.clone(), date: self.context.clock.now(),
            read: true, classified: true, attachments, ..Default::default()
        };
        sent.outgoing_fingerprint = crate::domain::mail_identity::outgoing_fingerprint(&sent, &digests);
        let message = OutgoingMessage {
            message_id:Some(message_id),
            to: to.clone(),
            cc: addresses(&draft.cc),
            bcc: addresses(&draft.bcc),
            subject: draft.subject.clone(),
            plain: rendered.plain.clone(),
            html: rendered.html,
            in_reply_to: draft
                .reply_to
                .and_then(|id| store.email(id))
                .map(|e| e.message_id.clone()),
            attachments: draft.attachments.clone(),
        };
        if let Some(remote)=&draft.remote_id {
            self.context.mail.send_remote_draft(&account,sender_name,message,remote,draft.remote_revision.as_deref().unwrap_or_default())?;
        } else {
            self.context.mail.send(&account, sender_name, message)?;
        }
        self.context.update_with_events(|current| {
            // Preserve an external edit made while SMTP was running.
            let had_draft = current.drafts.iter().any(|d| d.id == id);
            current.drafts.retain(|d| d.id != id || d.updated != draft.updated);
            let draft_removed = had_draft && !current.drafts.iter().any(|d| d.id == id);
            let sent_id = current.new_id();
            sent.id = sent_id;
            sent.thread_id = draft.reply_to.and_then(|r| current.email(r)).map(|e| e.thread_id.clone()).unwrap_or_else(|| sent.message_id.trim_matches(['<', '>']).to_string());
            // thread_id copies the parent's thread, which is one level too high for a reply to a
            // reply, so the direct parent is recorded on its own (ADR-0010).
            sent.in_reply_to = draft.reply_to.and_then(|r| current.email(r)).map(|e| e.message_id.trim_matches(['<', '>']).to_string()).unwrap_or_default();
            for (index, attachment) in sent.attachments.iter_mut().enumerate() { attachment.id = sent_id.saturating_mul(1000).saturating_add(index as u64); }
            current.emails.push(sent);
            let mut touched = vec![sent_id];
            if let Some(e) = draft.reply_to.and_then(|r| current.email_mut(r)) {
                e.replied = true;
                touched.push(e.id);
            }
            if let Some(a) = current.accounts.iter_mut().find(|a| a.id == account.id) { a.last_signature = draft.signature_id; }
            let mut events = Vec::new();
            if draft_removed {
                events.push(AppEvent::DraftsChanged { ids: vec![id] });
            }
            events.push(AppEvent::EmailChanged { account_id: account.id, ids: touched });
            events.push(AppEvent::CountsChanged { account_id: account.id });
            Ok(((), events))
        }).map_err(|e| AppError::new("sent_not_saved", format!("Message was sent but the local copy could not be saved. Do not send it again. {}", e.message)))
    }
    pub fn validate_send(&self, id: u64) -> AppResult<()> {
        let store = self.context.read_shared()?;
        let draft = store
            .drafts
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| AppError::missing("Draft"))?;
        self.validate(draft)?;
        self.context.account(&store, draft.account_id)?;
        Ok(())
    }
    fn validate(&self, draft: &Draft) -> AppResult<Vec<String>> {
        let to = addresses(&draft.to);
        if to.is_empty() && addresses(&draft.cc).is_empty() && addresses(&draft.bcc).is_empty() {
            return Err(AppError::validation("Add at least one recipient"));
        }
        if draft.subject.contains(['\r','\n']){return Err(AppError::validation("Subject must not contain line breaks"));}
        if to
            .iter()
            .chain(addresses(&draft.cc).iter())
            .chain(addresses(&draft.bcc).iter())
            .any(|a| !a.contains('@') || a.contains(['\r', '\n']))
        {
            return Err(AppError::validation("Check the recipient addresses"));
        }
        let size = self.context.attachments.total_size(&draft.attachments)?;
        if size > rules::ATTACHMENT_CAP_BYTES {
            return Err(AppError::validation("Attachments exceed the 25 MB limit"));
        }
        Ok(to)
    }
}
