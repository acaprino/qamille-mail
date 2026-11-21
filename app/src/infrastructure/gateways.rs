use super::{ai, mail, oauth, rag, secret};
use crate::application::{AppError, AppResult, ports::*};
use crate::domain::model::{Account, Category, Email, Settings, Store};
use std::sync::{Arc, atomic::AtomicBool};

pub struct SystemClock;
impl Clock for SystemClock {
    fn regional(&self, settings: &Settings) -> RegionalSettings {
        RegionalSettings { time_zone: super::timezone::resolved_timezone(settings), locale: super::timezone::resolved_locale(settings), warning: super::timezone::timezone_warning(settings) }
    }
    fn now_ms(&self) -> i64 {
        chrono::Utc::now().timestamp_millis()
    }
    fn now(&self) -> i64 {
        crate::domain::model::now_ts()
    }
}

pub struct LocalAttachments;
impl AttachmentGateway for LocalAttachments {
    fn persistent_paths(&self, paths: &[String]) -> AppResult<Vec<String>> {
        let root = Store::data_dir();
        paths.iter().map(|path| super::portable_files::persistent_path(path, &root)).collect()
    }
    fn snapshot_outgoing(&self, paths:&[String], key:&str)->AppResult<Vec<String>> {
        snapshot_outgoing_in(&Store::data_dir().join("outbox-files"), paths, key)
    }
    fn cleanup_cache(&self, store: &Store) -> AppResult<()> {
        super::mail_cache::cleanup(&Store::data_dir().join("attachments"), store)
    }
    fn read_bytes(&self, attachment: &crate::model::Attachment) -> AppResult<Vec<u8>> {
        if let Some(path) = &attachment.path {
            return std::fs::read(path).map_err(|_| AppError::storage(format!("Attachment unavailable: {}", attachment.name)));
        }
        attachment.inline_text.as_ref().map(|text| text.as_bytes().to_vec())
            .ok_or_else(|| AppError::storage(format!("Attachment unavailable: {}", attachment.name)))
    }
    fn total_size(&self, paths: &[String]) -> AppResult<u64> {
        let mut size = 0u64;
        for path in paths {
            let metadata = std::fs::metadata(path).map_err(|_| {
                AppError::validation(
                    "An attachment is no longer available. Remove it or attach it again.",
                )
            })?;
            if !metadata.is_file() {
                return Err(AppError::validation("Attachments must be files"));
            }
            size = size.saturating_add(metadata.len());
        }
        Ok(size)
    }
    fn text_from_bytes(&self, name: &str, bytes: &[u8]) -> AppResult<String> {
        super::search_support::bytes_attachment_text(name, bytes).map_err(AppError::validation)
    }
    fn read_text(&self, attachment: &crate::model::Attachment) -> AppResult<String> {
        super::search_support::cached_attachment_text(attachment).ok_or_else(|| AppError::external(format!("Attachment text unavailable: {}", attachment.name)))
    }
}

fn snapshot_outgoing_in(root: &std::path::Path, paths: &[String], key: &str) -> AppResult<Vec<String>> {
    use sha2::{Digest, Sha256};
    use std::io::Write;
    let folder = root.join(format!("{:x}", Sha256::digest(key.as_bytes())));
    let mut output = Vec::new();
    for (index, path) in paths.iter().enumerate() {
        let bytes = std::fs::read(path)
            .map_err(|_| AppError::validation("An outgoing attachment is unavailable"))?;
        // Keep disambiguation in the directory: mail providers use the basename
        // as the visible attachment filename.
        let name = std::path::Path::new(path).file_name()
            .ok_or_else(|| AppError::validation("Invalid attachment name"))?;
        let target_folder = folder.join(format!("{index}-{:x}", Sha256::digest(&bytes)));
        std::fs::create_dir_all(&target_folder)?;
        let target = target_folder.join(name);
        if target.exists() {
            if std::fs::read(&target)? != bytes {
                return Err(AppError::storage("An outgoing attachment snapshot has changed"));
            }
        } else {
            let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(&target)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
        }
        output.push(target.to_string_lossy().into_owned());
    }
    Ok(output)
}

#[cfg(test)]
mod outgoing_snapshot_tests {
    use super::*;

    #[test]
    fn snapshot_preserves_mime_filenames_and_duplicate_basenames() {
        let root = std::env::temp_dir().join(format!(
            "qamille-attachment-names-{}-{}", std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
        ));
        let first = root.join("first");
        let second = root.join("second");
        std::fs::create_dir_all(&first).unwrap();
        std::fs::create_dir_all(&second).unwrap();
        let a = first.join("report.pdf");
        let b = second.join("report.pdf");
        std::fs::write(&a, b"first report").unwrap();
        std::fs::write(&b, b"second report").unwrap();
        let paths = vec![a.to_string_lossy().into_owned(), b.to_string_lossy().into_owned()];
        let snapshots = snapshot_outgoing_in(&root.join("snapshots"), &paths, "draft-1").unwrap();
        assert_ne!(snapshots[0], snapshots[1]);
        assert_eq!(std::fs::read(&snapshots[0]).unwrap(), b"first report");
        assert_eq!(std::fs::read(&snapshots[1]).unwrap(), b"second report");

        let message = mail::Outgoing {
            message_id: None, to: vec!["recipient@example.test".into()], cc: vec![], bcc: vec![],
            subject: "Synthetic attachment".into(), body: "Body".into(), html: None,
            in_reply_to: None, attachments: snapshots.clone(),
        };
        let account = Account { email: "sender@example.test".into(), ..Default::default() };
        let mime = String::from_utf8(mail::build_message(&account, "Sender", &message).unwrap().formatted()).unwrap();
        assert_eq!(mime.matches("filename=\"report.pdf\"").count(), 2);

        std::fs::write(&a, b"newer report").unwrap();
        let newer = snapshot_outgoing_in(&root.join("snapshots"), &paths, "draft-1").unwrap();
        assert_ne!(snapshots[0], newer[0]);
        assert_eq!(std::fs::read(&snapshots[0]).unwrap(), b"first report");
        assert_eq!(std::fs::read(&newer[0]).unwrap(), b"newer report");
        assert_eq!(newer[1], snapshots[1]);
    }
}

pub struct KeyringSecrets;
impl SecretStore for KeyringSecrets {
    fn get(&self, key: &str) -> AppResult<String> {
        secret::get_result(key).map_err(AppError::external)
    }
    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        secret::set(key, value).map_err(AppError::external)
    }
}

pub struct GmailGateway {
    pub attachments: std::path::PathBuf,
    reader: super::mail_reader::MailReader,
}
impl GmailGateway {
    pub fn new(attachments: std::path::PathBuf) -> Self {
        Self { attachments, reader: super::mail_reader::MailReader::default() }
    }
}
impl MailGateway for GmailGateway {
    fn send_remote_draft(&self,account:&Account,sender_name:&str,message:OutgoingMessage,id:&str,revision:&str)->AppResult<()>{super::gmail_drafts::send_snapshot(account,sender_name,message,id,revision)}
    fn authorize_mcp_delivery(&self,profile:Option<&str>,account_id:u32)->AppResult<()> {
        let maximum=super::repository::JsonStoreRepository::new(Store::path()).read()?.settings.mcp_access;
        if maximum!=crate::model::Access::Send{return Err(AppError::new("permission_denied","MCP send permission was revoked"));}
        if let Some(name)=profile {let grant=super::mcp_config::profile(name,maximum)?;if grant.access!=crate::model::Access::Send||grant.account_ids.as_ref().is_some_and(|ids|!ids.contains(&account_id)){return Err(AppError::new("permission_denied","MCP profile no longer allows sending from this account"));}}
        Ok(())
    }
    fn list_labels(&self,account:&Account)->AppResult<serde_json::Value>{super::gmail_drafts::list_labels(account)}
    fn manage_label(&self,account:&Account,action:&str,name:&str,new_name:Option<&str>)->AppResult<serde_json::Value>{super::gmail_drafts::manage_label(account,action,name,new_name)}
    fn capabilities(&self, account: &Account) -> crate::domain::outbox::ProviderCapabilities {
        let gmail=account.resolved_provider_id()=="gmail";
        crate::domain::outbox::ProviderCapabilities { provider_id: account.resolved_provider_id().into(), sync:gmail, remote_actions:gmail, remote_search:gmail, remote_drafts:gmail && !account.oauth_client_id.is_empty(), native_threads:gmail, labels:gmail, send: true, max_attachment_bytes: crate::rules::ATTACHMENT_CAP_BYTES }
    }
    fn list_remote_drafts(&self, account: &Account) -> AppResult<Vec<crate::domain::outbox::RemoteDraft>> { super::gmail_drafts::list(account) }
    fn save_remote_draft(&self, account: &Account, draft: &crate::domain::outbox::RemoteDraft) -> AppResult<crate::domain::outbox::RemoteDraft> { super::gmail_drafts::save(account, draft) }
    fn delete_remote_draft(&self, account: &Account, id: &str) -> AppResult<()> { super::gmail_drafts::delete(account, id) }
    fn sync_with_policy(&self, account: &Account, request: &SyncRequest<'_>) -> AppResult<PolicySyncBatch> {
        // Google's refusal of the sign-in keeps its code (`sign_in_expired`) through the IMAP error.
        super::mail_cache_download::sync_account_policy(account, &self.attachments, request)
            .map_err(oauth::provider_error)
    }
    fn load_body(&self, account: &Account, email: &Email) -> AppResult<Email> {
        super::mail_cache_download::download_body(account, email).map_err(AppError::external)
    }
    fn load_body_preview(&self, account: &Account, email: &Email) -> AppResult<Email> {
        self.reader.read(account, email).map_err(AppError::external)
    }
    fn search(&self, account: &Account, request: &crate::application::ports::RemoteSearchRequest, cancel: &std::sync::atomic::AtomicBool) -> AppResult<crate::application::ports::RemoteSearchPage> {
        mail::search_account(account, request, cancel).map_err(AppError::external)
    }
    fn history_batch(&self, account: &Account, request: &crate::application::ports::HistoryRequest) -> AppResult<crate::application::ports::HistoryBatch> {
        mail::history_batch(account, request).map_err(AppError::external)
    }
    fn apply_message(&self, account: &Account, email: &Email, action: RemoteAction) -> AppResult<()> {
        let action = match action {
            RemoteAction::Seen(v) => mail::Remote::Seen(v),
            RemoteAction::Flag(v) => mail::Remote::Flag(v),
            RemoteAction::Move(v) => mail::Remote::Move(v),
            RemoteAction::Archive => mail::Remote::Archive,
            RemoteAction::Label(v) => mail::Remote::Label(v),
            RemoteAction::RemoveLabel(v)=>mail::Remote::RemoveLabel(v),
            RemoteAction::Spam=>mail::Remote::Spam,
        };
        mail::push_message_action(account, email, action).map_err(AppError::external)
    }
    fn resolve_identity(&self, account: &Account, email: &Email) -> AppResult<Email> {
        mail::resolve_identity(account, email).map_err(AppError::external)
    }
    fn watch(&self, account: &Account) -> AppResult<Box<dyn MailWatch>> {
        super::idle::watch(account)
    }
    fn originals(&self, account: &Account, emails: &[Email]) -> AppResult<Vec<Vec<u8>>> {
        mail::download_originals(account, emails).map_err(AppError::external)
    }
    fn attachment_bytes(&self, account: &Account, email: &Email, attachment_id: u64) -> AppResult<Vec<u8>> {
        super::mail_cache_download::download_attachment(account, email, attachment_id).map_err(AppError::external)
    }
    fn thread(&self, account: &Account, email: &Email) -> AppResult<Vec<Email>> {
        mail::download_thread(account, email).map_err(AppError::external)
    }
    fn sync(&self, a: &Account) -> AppResult<SyncBatch> {
        if a.resolved_provider_id() == "sample" {
            return Ok(SyncBatch {
                emails: vec![],
                archive_folder: None,
                trash_folder: None,
            });
        }
        let emails = mail::sync_account(a, &self.attachments, 0).map_err(oauth::provider_error)?;
        let (archive_folder, trash_folder) = mail::gmail_folders(a).map_err(oauth::provider_error)?;
        Ok(SyncBatch { emails, archive_folder, trash_folder })
    }
    fn apply(&self, a: &Account, uid: u32, action: RemoteAction) -> AppResult<()> {
        if a.resolved_provider_id() == "sample" {
            return Ok(());
        }
        let action = match action {
            RemoteAction::Seen(v) => mail::Remote::Seen(v),
            RemoteAction::Flag(v) => mail::Remote::Flag(v),
            RemoteAction::Move(v) => mail::Remote::Move(v),
            RemoteAction::Archive => mail::Remote::Archive,
            RemoteAction::Label(v) => mail::Remote::Label(v),
            RemoteAction::RemoveLabel(v)=>mail::Remote::RemoveLabel(v),
            RemoteAction::Spam=>mail::Remote::Spam,
        };
        mail::push_action(a, uid, action).map_err(AppError::external)
    }
    fn send(&self, a: &Account, name: &str, message: OutgoingMessage) -> AppResult<()> {
        if a.resolved_provider_id() == "sample" {
            return Ok(());
        }
        mail::send(
            a,
            name,
            &mail::Outgoing {
                message_id:message.message_id,
                to: message.to,
                cc: message.cc,
                bcc: message.bcc,
                subject: message.subject,
                body: message.plain,
                html: message.html,
                in_reply_to: message.in_reply_to,
                attachments: message.attachments,
            },
        )
        .map_err(AppError::external)
    }
    fn check_login(&self, a: &Account) -> AppResult<()> {
        mail::check_login(a).map_err(AppError::external)
    }
}

pub struct GoogleOAuth;
impl GoogleIdentity for GoogleOAuth {
    fn client_credentials(&self, client_id: &str, client_secret: &str) -> AppResult<(String, String)> {
        super::google_oauth_config::credentials(client_id, client_secret)
    }
    fn sign_in(&self, client_id: &str, client_secret: &str, login_hint: &str) -> AppResult<(String, String)> {
        let (email, token) = oauth::sign_in(client_id, client_secret, &AtomicBool::new(false), login_hint)
            .map_err(AppError::external)?;
        Ok((email, token.refresh))
    }
    fn cancel_sign_in(&self) {
        oauth::cancel_sign_in();
    }
}

/// Search over the local index; vectors and passages come from the compute gateway.
pub struct LocalSearch {
    compute: Arc<dyn crate::application::compute::ComputeGateway>,
    notices: Arc<super::compute_factory::Notices>,
}
impl LocalSearch {
    pub fn new(compute: Arc<dyn crate::application::compute::ComputeGateway>) -> Self {
        Self { compute, notices: super::compute_factory::notices() }
    }
    /// With the notices of a factory of its own (tests).
    pub fn with_notices(compute: Arc<dyn crate::application::compute::ComputeGateway>, notices: Arc<super::compute_factory::Notices>) -> Self {
        Self { compute, notices }
    }
    /// The gateway of this moment: one operation keeps it to the end (D3).
    fn gateway(&self) -> Arc<dyn crate::application::compute::ComputeGateway> {
        crate::application::compute::pinned(&self.compute)
    }
    /// The hits, and why they are words only when the query vector could not be had (D8).
    pub fn search_reported(&self, store: &Store, query: &str, limit: usize) -> AppResult<(Vec<SearchHit>, Option<AppError>)> {
        let compute = self.gateway();
        let (hits, unavailable) = rag::search_disk_reported(store, query, limit, &*compute).map_err(AppError::external)?;
        let hits = hits
            .into_iter()
            .map(|h| SearchHit {
                email_id: h.email_id,
                score: h.score,
                passage: h.passage,
            })
            .collect();
        Ok((hits, unavailable))
    }
}
impl Default for LocalSearch {
    /// The gateway of this process (`compute_factory::routed`).
    fn default() -> Self {
        Self::new(super::compute_factory::routed())
    }
}
const SEMANTIC_NOTICE: &str = "semantic-unavailable";
impl SearchGateway for LocalSearch {
    fn lexical_search(&self,store:&Store,query:&str,account:Option<u32>,now:i64,sort:&str)->AppResult<Option<Vec<u64>>> {
        super::lexical_index::search(&Store::data_dir(),store,query,account,now,sort)
    }
    fn index_document(&self, store: &Store, content: &crate::application::indexing_ports::IndexContent, cancel: &AtomicBool) -> AppResult<crate::application::indexing_ports::DocumentIndexReport> {
        super::rag_segments::index_document(store, content, cancel, &*self.gateway())
    }
    fn index_lexical(&self, store: &Store, content: &crate::application::indexing_ports::IndexContent, cancel: &AtomicBool) -> AppResult<crate::application::indexing_ports::DocumentIndexReport> {
        super::rag_segments::index_lexical(store, content, cancel)
    }
    /// Words only when the query vector cannot be had, with the notice of D8 once per outage.
    fn search(&self, store: &Store, query: &str, limit: usize) -> AppResult<Vec<SearchHit>> {
        let (hits, unavailable) = self.search_reported(store, query, limit)?;
        match unavailable {
            Some(_) => self.notices.once(SEMANTIC_NOTICE, crate::application::compute::SEMANTIC_UNAVAILABLE_MESSAGE),
            None => self.notices.forget(SEMANTIC_NOTICE),
        }
        Ok(hits)
    }
    fn index(&self, store: &Store) -> AppResult<String> {
        self.index_with_content(store, &store.emails)
    }
    fn index_with_content(&self, store: &Store, content: &[crate::domain::model::Email]) -> AppResult<String> {
        let (_, report) = rag::update_index_for_store(store, content, &AtomicBool::new(false), &|_, _| {}, &*self.gateway())
            .map_err(AppError::external)?;
        if report.skipped_emails == 0 {
            Ok(format!("{} messages indexed", report.added_emails))
        } else {
            Ok(format!("{} messages indexed; {} skipped because the account storage limit was reached", report.added_emails, report.skipped_emails))
        }
    }
    fn installed(&self) -> bool {
        rag::model_installed(&*self.gateway())
    }
    fn needs_server(&self) -> bool {
        matches!(self.gateway().health(), Err(e) if e.code == crate::application::compute::codes::NOT_CONNECTED)
    }
    /// Whether vectors can be made now. A device the server refuses (`401`, A8) and a server with
    /// another model than the index's (D4) stop the indexing of the account; the second is said
    /// once per pair of models.
    fn semantic_readiness(&self) -> AppResult<bool> {
        use crate::application::compute::codes;
        let compute = self.gateway();
        match super::rag_segments::checked_identity(&*compute, &Store::data_dir()) {
            Ok(_) => Ok(rag::model_installed(&*compute)),
            Err(error) if error.code == codes::MODEL_MISMATCH => {
                let server = compute.health().map(|health| health.embedding_identity().model).unwrap_or_default();
                self.notices.once(&format!("mismatch:{}:{server}", compute.identity()), crate::application::compute::MODEL_MISMATCH_MESSAGE);
                Err(error)
            }
            Err(error) if error.code == codes::UNAUTHORIZED => Err(AppError::new(super::rag_segments::COMPUTE_UNAUTHORIZED, error.message)),
            // A pause or no session: words only for now, the vectors follow (EX-3).
            Err(_) => Ok(false),
        }
    }
    fn reset_vectors(&self) -> AppResult<()> {
        super::rag_segments::reset_vectors(&Store::data_dir())
    }
    fn install_status(&self) -> crate::application::ports::ModelInstallStatus {
        let health = self.gateway().health().ok();
        let model = health.as_ref().and_then(|health| health.model(&health.embedding_identity().model).cloned());
        match model {
            Some(model) => crate::application::ports::ModelInstallStatus {
                installed: model.ready(),
                running: model.state == "downloading",
                done_bytes: model.done_bytes,
                total_bytes: model.total_bytes,
                error: model.error,
            },
            None => crate::application::ports::ModelInstallStatus::default(),
        }
    }
}

pub struct ConfiguredAssistant;
impl AssistantGateway for ConfiguredAssistant {
    fn summarize(&self, s: &Settings, kind: &str, text: &str) -> AppResult<String> {
        Ok(ai::summarize(s, kind, text))
    }
    fn reply(&self, s: &Settings, e: &Email, instruction: &str) -> AppResult<String> {
        Ok(ai::generate_reply(s, e, instruction))
    }
    fn ask(&self, s: &Settings, q: &str, context: &str) -> AppResult<String> {
        Ok(ai::ask_assistant(s, q, context))
    }
    fn classify(
        &self,
        s: &Settings,
        e: &Email,
        labels: &[String],
    ) -> AppResult<(Category, Vec<String>)> {
        Ok(ai::classify(s, e, labels))
    }
}
