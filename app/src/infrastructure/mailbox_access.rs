//! Detach account-scoped permissions and pending writes outside the mail database.
use crate::application::{AppError, AppResult, contact_ports::ContactStore};
use crate::model::Store;

pub fn revoke(id: u32, detach: &mut dyn FnMut() -> AppResult<()>) -> AppResult<()> {
    revoke_in(&Store::data_dir(), id, detach)
}

fn revoke_in(root: &std::path::Path, id: u32, detach: &mut dyn FnMut() -> AppResult<()>) -> AppResult<()> {
    let contacts = super::contact_repository::FileContactStore::new(root.to_path_buf());
    let _flush = contacts.lock_flush()?;
    super::mcp_config::revoke_mailbox(root, id)?;
    contacts.revoke_mailbox_grants(id)?;
    detach()?;
    if let Err(error) = contacts.discard_detached_writes(id) {
        // The committed mailbox can no longer send or regain this identifier. Keep reporting
        // its removal so the caller also publishes the deletion to the person's other devices.
        eprintln!("Could not clear the detached mailbox's queued contact writes: {error}");
    }
    Ok(())
}

/// The legacy editor must persist its updated snapshot before discarding contact writes.
pub fn retire_legacy(store: &mut Store, id: u32) -> Result<(), String> {
    let account = store.account(id).cloned().ok_or_else(|| "Mailbox no longer exists".to_string())?;
    revoke(id, &mut || {
        if account.resolved_provider_id() != "sample" {
            let primary = crate::application::sync::credential_key(&account);
            super::secret::get_result(&primary).map_err(AppError::storage)?;
            for key in [format!("calendar:{id}"), format!("contacts:{id}"), primary] {
                super::secret::remove(&key).map_err(AppError::storage)?;
            }
            super::oauth::forget_access(&account.email);
        }
        let mut detached = store.clone();
        detached.detach_mailbox(id, crate::model::now_ts());
        detached.save()?;
        *store = detached;
        Ok(())
    }).map_err(|error| error.message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::contacts::{BookId, QueuedWrite, SourceContact, WriteOp};
    use crate::infrastructure::{contact_repository::FileContactStore, mcp_config::Config};

    #[test]
    fn detaching_only_revokes_the_selected_book_and_grants() {
        let root = std::env::temp_dir().join(format!("qamille-detach-grants-{}-{}", std::process::id(), crate::model::now_ts()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("mcp-config.json");
        std::fs::write(&path, r#"{"profiles":{"assistant":{"account_ids":[12,13]}}}"#).unwrap();
        let contacts = FileContactStore::new(root.clone());
        contacts.transaction(&mut |state| {
            for id in [12, 13] {
                let key = format!("card-{id}");
                let book = BookId::google(id);
                state.sources.push(SourceContact { key: key.clone(), book: book.clone(), ..Default::default() });
                state.queue.push(QueuedWrite { key, op: WriteOp::Create });
                state.settings.write_granted.push(book.0);
            }
            Ok(())
        }).unwrap();
        let failed = revoke_in(&root, 12, &mut || Err(AppError::storage("injected detach failure")));
        assert!(failed.is_err());
        let state = contacts.read().unwrap();
        assert_eq!(state.queue.len(), 2, "a refused vault operation or mail repository commit preserves pending writes");
        revoke_in(&root, 12, &mut || {
            assert_eq!(contacts.read()?.queue.len(), 2, "contact cleanup must follow the detach commit");
            Ok(())
        }).unwrap();
        let config: Config = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(config.profiles["assistant"].account_ids.as_deref(), Some(&[13][..]));
        let state = contacts.read().unwrap();
        assert_eq!(state.queue.len(), 1);
        assert_eq!(state.queue[0].key, "card-13");
        assert_eq!(state.settings.write_granted, ["google:13"]);
        assert_eq!(state.sources.len(), 2, "historical contact data stays available for recovery");
        std::fs::remove_dir_all(root).unwrap();
    }
}
