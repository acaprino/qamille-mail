//! Detach account-scoped permissions and pending writes outside the mail database.
use crate::application::{AppError, AppResult, contact_ports::ContactStore};
use crate::model::Store;

pub fn revoke(id: u32, detach: &mut dyn FnMut() -> AppResult<()>) -> AppResult<()> {
    revoke_in(&Store::data_dir(), id, detach)
}

fn revoke_in(root: &std::path::Path, id: u32, detach: &mut dyn FnMut() -> AppResult<()>) -> AppResult<()> {
    let contacts = super::contact_repository::FileContactStore::new(root.to_path_buf());
    let _flush = contacts.lock_flush()?;
    detach()?;
    super::mcp_config::revoke_mailbox(root, id)?;
    contacts.revoke_mailbox_grants(id)?;
    contacts.discard_detached_writes(id)
}

fn clear_legacy_credentials(id: u32) -> AppResult<()> {
    Store::with_persisted_store(&mut |store| {
        crate::application::accounts::clear_retired_credentials(store, id, &super::gateways::KeyringSecrets)
    })
}

fn settle_legacy(store: &mut Store, id: u32, result: AppResult<()>) {
    let result = result.and_then(|()| Store::with_persisted_store(&mut |current| {
        current.settings.pending_mailbox_cleanup.remove(&id);
        Ok(())
    }));
    match result {
        Ok(()) => { store.settings.pending_mailbox_cleanup.remove(&id); }
        Err(error) => eprintln!("Mailbox {id} cleanup will resume after restart: {error}"),
    }
}

pub fn retry_legacy(store: &mut Store) {
    let ids: Vec<_> = store.settings.pending_mailbox_cleanup.keys().copied().collect();
    for id in ids {
        let result = revoke(id, &mut || clear_legacy_credentials(id));
        settle_legacy(store, id, result);
    }
}

pub fn retire_legacy(store: &mut Store, id: u32) -> Result<(), String> {
    let account = store.account(id).cloned().ok_or_else(|| "Mailbox no longer exists".to_string())?;
    let primary = crate::application::accounts::retirement_key(&account);
    let mut committed = false;
    let result = revoke(id, &mut || {
        if !primary.is_empty() { super::secret::get_result(&primary).map_err(AppError::storage)?; }
        let mut detached = store.clone();
        detached.detach_mailbox(id, crate::model::now_ts());
        detached.save_snapshot(Some(&account))?;
        detached.settings.pending_mailbox_cleanup.insert(id, primary.clone());
        *store = detached;
        committed = true;
        if let Some(email) = primary.strip_prefix("gmail:") { super::oauth::forget_access(email); }
        clear_legacy_credentials(id)
    });
    if !committed { return result.map_err(|error| error.message); }
    settle_legacy(store, id, result);
    Ok(())
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
        assert_eq!(state.settings.write_granted, ["google:12", "google:13"]);
        let config: Config = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(config.profiles["assistant"].account_ids.as_deref(), Some(&[12,13][..]));
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
