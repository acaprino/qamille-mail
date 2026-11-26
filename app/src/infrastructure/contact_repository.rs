//! Contacts storage: `contacts.json` with a process lock and atomic replacement.
use crate::application::{AppError, AppResult, contact_ports::ContactStore};
use crate::domain::contacts::ContactsState;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

pub struct FileContactStore {
    root: PathBuf,
}
impl FileContactStore {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
    pub(super) fn revoke_mailbox_grants(&self, id: u32) -> AppResult<()> {
        if !self.data_path().try_exists()? { return Ok(()); }
        self.transaction(&mut |state| {
            let belongs = |book: &str| crate::domain::contacts::BookId(book.to_string()).account_id() == Some(id);
            state.settings.write_granted.retain(|book| !belongs(book));
            state.sync_tokens.retain(|book, _| !belongs(book));
            Ok(())
        })
    }

    pub(super) fn discard_detached_writes(&self, id: u32) -> AppResult<()> {
        if !self.data_path().try_exists()? { return Ok(()); }
        self.transaction(&mut |state| {
            let keys: std::collections::HashSet<_> = state.sources.iter()
                .filter(|card| card.book.account_id() == Some(id)).map(|card| card.key.clone()).collect();
            state.queue.retain(|write| !keys.contains(&write.key));
            Ok(())
        })
    }

    fn data_path(&self) -> PathBuf {
        self.root.join("contacts.json")
    }
    fn lock_file(&self) -> AppResult<File> {
        std::fs::create_dir_all(&self.root)?;
        Ok(OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(self.root.join("contacts.lock"))?)
    }
    /// A missing file is an empty book; an unreadable one is never replaced.
    fn load(&self) -> AppResult<ContactsState> {
        match std::fs::read(self.data_path()) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .map_err(|_| AppError::new("contacts_damaged", "Contacts data is damaged; original file preserved")),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(ContactsState::default()),
            Err(e) => Err(e.into()),
        }
    }
}
impl ContactStore for FileContactStore {
    fn lock_flush(&self) -> AppResult<Box<dyn Send>> {
        std::fs::create_dir_all(&self.root)?;
        let guard = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(self.root.join("contacts-flush.lock"))?;
        guard.lock()?;
        Ok(Box::new(guard))
    }
    fn read(&self) -> AppResult<ContactsState> {
        let guard = self.lock_file()?;
        guard.lock_shared()?;
        self.load()
    }
    fn transaction(&self, operation: &mut dyn FnMut(&mut ContactsState) -> AppResult<()>) -> AppResult<()> {
        let guard = self.lock_file()?;
        guard.lock()?;
        let mut state = self.load()?;
        operation(&mut state)?;
        let path = self.data_path();
        let temporary = path.with_extension(format!("{}.tmp", std::process::id()));
        let mut file = File::create(&temporary)?;
        file.write_all(&serde_json::to_vec_pretty(&state)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(temporary, path)?;
        Ok(())
    }
}

#[derive(Default)]
pub struct MemoryContactStore(Mutex<ContactsState>);
impl ContactStore for MemoryContactStore {
    fn read(&self) -> AppResult<ContactsState> {
        Ok(self.0.lock().map_err(|_| AppError::storage("Contacts lock failed"))?.clone())
    }
    fn transaction(&self, operation: &mut dyn FnMut(&mut ContactsState) -> AppResult<()>) -> AppResult<()> {
        let mut guard = self.0.lock().map_err(|_| AppError::storage("Contacts lock failed"))?;
        let mut working = guard.clone();
        operation(&mut working)?;
        *guard = working;
        Ok(())
    }
}
