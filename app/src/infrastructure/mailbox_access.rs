//! Detach account-scoped permissions and pending writes outside the mail database.
use crate::application::AppResult;
use crate::model::Store;

pub fn revoke(id: u32) -> AppResult<()> {
    revoke_in(&Store::data_dir(), id)
}

fn revoke_in(root: &std::path::Path, id: u32) -> AppResult<()> {
    super::mcp_config::revoke_mailbox(root, id)?;
    super::contact_repository::FileContactStore::new(root.to_path_buf()).revoke_mailbox(id)
}

/// The legacy native client uses the same sidecars and vault as the desktop.
pub fn retire_legacy(account: &crate::model::Account) -> Result<(), String> {
    revoke(account.id).map_err(|e| e.message)?;
    if account.resolved_provider_id() != "sample" {
        let primary = crate::application::sync::credential_key(account);
        super::secret::get_result(&primary)?;
        for key in [format!("calendar:{}", account.id), format!("contacts:{}", account.id), primary] {
            super::secret::remove(&key)?;
        }
        super::oauth::forget_access(&account.email);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::contact_ports::ContactStore;
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
        revoke_in(&root, 12).unwrap();
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
