use super::*;
use qamille::{
    application::storage::AccountStorageService,
    domain::storage::{AccountStoragePolicy, StorageLimit},
};

fn policy(bytes: Option<u64>, offline: bool) -> AccountStoragePolicy {
    AccountStoragePolicy {
        limit: bytes.map_or(StorageLimit::Unlimited, |max_bytes| StorageLimit::Limited {
            max_bytes,
        }),
        keep_all_offline: offline,
        include_spam_trash: false,
    }
}

fn storage_fixture() -> Store {
    let mut store = fixture();
    let mut email = store.emails[0].clone();
    email.body = "Synthetic storage test. ".repeat(2_000);
    email.attachments.clear();
    email.pinned = true; // Its task identity must survive local content eviction.
    email.message_id = "quota-test@example.com".into();
    store.emails = vec![email];
    store.drafts.clear();
    store.undo_actions.clear();
    store
}

#[test]
fn lowering_account_limit_requires_confirmation_and_rolls_back_preview() {
    let initial = storage_fixture();
    let account_id = initial.emails[0].account_id;
    let email_id = initial.emails[0].id;
    let ctx = context(initial.clone(), Arc::new(TestMail::default()));
    let service = AccountStorageService::new(ctx.clone());
    let error = service
        .update(account_id, policy(Some(8_000), false), false)
        .unwrap_err();
    assert_eq!(error.code, "cleanup_confirmation_required");
    let after = ctx.read().unwrap();
    assert_eq!(
        after.account(account_id).unwrap().storage_policy,
        initial.account(account_id).unwrap().storage_policy
    );
    assert_eq!(
        after.email(email_id).unwrap().body,
        initial.email(email_id).unwrap().body
    );
    let result = service
        .update(account_id, policy(Some(8_000), false), true)
        .unwrap();
    assert!(!result.over_limit);
    assert!(ctx.read().unwrap().email(email_id).unwrap().remote_only);
    assert!(ctx.read().unwrap().email(email_id).unwrap().pinned);
}

#[test]
fn unlimited_and_protected_offline_account_never_evict_on_settings_change() {
    let initial = storage_fixture();
    let id = initial.emails[0].account_id;
    let ctx = context(initial.clone(), Arc::new(TestMail::default()));
    let service = AccountStorageService::new(ctx.clone());
    let result = service.update(id, policy(None, true), false).unwrap();
    assert!(
        !result.offline_complete,
        "Selecting offline is not proof that Gmail has been fully downloaded"
    );
    assert_eq!(ctx.read().unwrap().emails[0].body, initial.emails[0].body);
    let error = service
        .update(id, policy(Some(8_000), true), true)
        .unwrap_err();
    assert_eq!(error.code, "storage_protected");
    assert_eq!(
        service.get(id).unwrap().policy.limit,
        StorageLimit::Unlimited
    );
}

#[test]
fn changing_one_account_does_not_change_other_account_policy_or_content() {
    let mut initial = storage_fixture();
    let first = initial.emails[0].account_id;
    let mut other = initial.accounts[0].clone();
    other.id = 99;
    other.email = "other@example.com".into();
    other.storage_policy = policy(None, false);
    initial.accounts.push(other.clone());
    let mut other_mail = initial.emails[0].clone();
    other_mail.id = 99_999;
    other_mail.account_id = 99;
    other_mail.date = 1;
    initial.emails.push(other_mail.clone());
    let ctx = context(initial, Arc::new(TestMail::default()));
    let service = AccountStorageService::new(ctx.clone());
    service
        .update(first, policy(Some(8_000), false), true)
        .unwrap();
    let after = ctx.read().unwrap();
    assert_eq!(
        after.account(99).unwrap().storage_policy,
        other.storage_policy
    );
    assert_eq!(after.email(other_mail.id).unwrap().body, other_mail.body);
}

#[test]
fn invalid_limit_and_unknown_account_leave_store_unchanged() {
    let ctx = context(storage_fixture(), Arc::new(TestMail::default()));
    let service = AccountStorageService::new(ctx.clone());
    let id = ctx.read().unwrap().accounts[0].id;
    let before = serde_json::to_value(ctx.read().unwrap()).unwrap();
    assert_eq!(
        service
            .update(id, policy(Some(0), false), true)
            .unwrap_err()
            .code,
        "validation"
    );
    assert_eq!(
        service
            .update(u32::MAX, policy(None, false), true)
            .unwrap_err()
            .code,
        "not_found"
    );
    assert_eq!(serde_json::to_value(ctx.read().unwrap()).unwrap(), before);
}

#[test]
fn workspaces_saved_with_removed_offline_limits_open_and_keep_their_mail() {
    // ADR-0005 EX-002/EX-003: the removed global "Offline mail" fields are
    // ignored on load, evict nothing and disappear on the next save.
    let initial = storage_fixture();
    let mut json = serde_json::to_value(&initial).unwrap();
    json["settings"]["offline_days"] = 1.into();
    json["settings"]["offline_max_mb"] = 1.into();
    let legacy: Store = serde_json::from_value(json).unwrap();
    assert_eq!(legacy.emails[0].body, initial.emails[0].body);
    let ctx = context(legacy, Arc::new(TestMail::default()));
    let settings = ctx.read().unwrap().settings.clone();
    WorkspaceService::new(ctx.clone())
        .save_settings(qamille::application::workspace::SettingsUpdate {
            settings,
            ai_key: None,
            google_client_secret: None,
        })
        .unwrap();
    let saved = serde_json::to_value(ctx.read().unwrap()).unwrap();
    assert!(saved["settings"].get("offline_days").is_none());
    assert!(saved["settings"].get("offline_max_mb").is_none());
    assert_eq!(ctx.read().unwrap().emails[0].body, initial.emails[0].body);
}
