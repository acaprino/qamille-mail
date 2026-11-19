//! Source scanning and content extraction can be replaced independently from
//! the resumable workflow. All content objects below are temporary, non-serialized.
use super::AppResult;
use crate::domain::{
    indexing::FolderProgress,
    model::{Account, Email},
};
use std::sync::atomic::AtomicBool;

pub struct IndexMedia {
    pub attachment_id: u64,
    pub mime: String,
    pub bytes: Vec<u8>,
}
pub struct IndexContent {
    pub email: Email,
    pub media: Vec<IndexMedia>,
    pub warnings: Vec<String>,
}
pub struct IndexPage {
    pub progress: FolderProgress,
    pub email: Option<Email>,
}
/// Up to `max` new messages of one folder and the folder's moved cursor.
pub struct IndexBatch {
    pub progress: FolderProgress,
    pub emails: Vec<Email>,
}
/// One provider connection reused for many batches (W1.2). It ends when it
/// `expired`, after an error, or when dropped.
pub trait IndexSession: Send {
    /// Metadata of up to `max` messages of `folder`, in walk order.
    fn next_batch(
        &mut self,
        folder: &FolderProgress,
        max: usize,
        cancel: &AtomicBool,
    ) -> AppResult<IndexBatch>;
    /// Temporary contents of `emails`: one result per message, in the same order.
    /// One failing message never fails the others.
    fn contents(&mut self, emails: &[Email], cancel: &AtomicBool) -> Vec<AppResult<IndexContent>>;
    /// Time to close it and open a new one (age, messages read, an error).
    fn expired(&self) -> bool;
}
pub trait IndexSource: Send + Sync {
    fn inventory_total(
        &self,
        _account: &Account,
        _folders: &[FolderProgress],
        _cancel: &AtomicBool,
    ) -> AppResult<Option<u64>> {
        Ok(None)
    }
    fn folders(&self, account: &Account) -> AppResult<Vec<FolderProgress>>;
    /// A batch session. `None`: this source reads one message at a time
    /// (`next`, `content`) and the indexer adapts it (`PerMessageSession`).
    fn open(&self, _account: &Account) -> AppResult<Option<Box<dyn IndexSession>>> {
        Ok(None)
    }
    fn next(
        &self,
        _account: &Account,
        folder: &FolderProgress,
        _cancel: &AtomicBool,
    ) -> AppResult<IndexPage> {
        Ok(IndexPage { progress: folder.clone(), email: None })
    }
    fn content(
        &self,
        _account: &Account,
        email: &Email,
        _cancel: &AtomicBool,
    ) -> AppResult<IndexContent> {
        Ok(IndexContent { email: email.clone(), media: vec![], warnings: vec![] })
    }
}

/// A batch session over a source that reads one message at a time.
pub struct PerMessageSession {
    pub source: std::sync::Arc<dyn IndexSource>,
    pub account: Account,
}
impl IndexSession for PerMessageSession {
    fn next_batch(
        &mut self,
        folder: &FolderProgress,
        max: usize,
        cancel: &AtomicBool,
    ) -> AppResult<IndexBatch> {
        let mut progress = folder.clone();
        let mut emails = Vec::new();
        // Empty pages (stretches of deleted mail) are bounded like the windows
        // of a real batch, so one batch always ends quickly.
        let mut empty = 0;
        while emails.len() < max.max(1) && empty < 16 {
            let page = self.source.next(&self.account, &progress, cancel)?;
            let moved = page.progress != progress;
            progress = page.progress;
            let found = page.email.is_some();
            empty += usize::from(!found);
            emails.extend(page.email);
            if progress.complete || (!found && !moved) {
                break;
            }
        }
        Ok(IndexBatch { progress, emails })
    }
    fn contents(&mut self, emails: &[Email], cancel: &AtomicBool) -> Vec<AppResult<IndexContent>> {
        emails
            .iter()
            .map(|email| {
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    self.source.content(&self.account, email, cancel)
                }))
                .unwrap_or_else(|_| Err(super::AppError::new("index_panic", "Indexing stopped on this message")))
            })
            .collect()
    }
    fn expired(&self) -> bool {
        false
    }
}
#[derive(Default)]
pub struct DocumentIndexReport {
    pub indexed: bool,
    pub warnings: Vec<String>,
}
