//! The synchronization of the mailboxes (wave 7): the service against an in-memory server and the
//! «apps» of `support/sync_fakes.rs`. Every test is one or two computers of the same user.
#[path = "support/sync_fakes.rs"]
mod fakes;

use fakes::*;
use qamille::{
    application::{
        ports::SecretStore,
        sync::{PendingDelete, SyncMode},
    },
    domain::{
        model::{AiProvider, SignatureBinding},
        sync::{CREDENTIAL_GOOGLE, item_id},
    },
};
use std::sync::{Arc, atomic::Ordering};

const CLIENT: &str = "personal.apps.googleusercontent.com";

fn pair() -> (Arc<FakeSync>, App, App) {
    let server = Arc::new(FakeSync::default());
    let (a, b) = (App::new(&server), App::new(&server));
    for app in [&a, &b] {
        app.sign_in("u1");
        app.consent();
    }
    (server, a, b)
}

#[test]
fn nothing_is_sent_without_consent_login_and_the_switch() {
    let server = Arc::new(FakeSync::default());
    let app = App::new(&server);
    app.add_gmail("giulia@gmail.com", "1//token", CLIENT);
    // No session.
    assert!(app.sync.sync_now().unwrap().skipped);
    // A session, no consent.
    app.sign_in("u1");
    assert!(app.sync.sync_now().unwrap().skipped);
    // Consent for another address.
    app.context.update(|s| { s.settings.mailbox_sync = true; s.settings.mailbox_sync_consent = Some("https://altro.example".into()); Ok(()) }).unwrap();
    assert!(app.sync.sync_now().unwrap().skipped);
    // The general switch off.
    app.consent();
    app.sync.set_enabled(false).unwrap();
    assert!(app.sync.sync_now().unwrap().skipped);
    assert!(server.calls().is_empty(), "{:?}", server.calls());
    app.sync.set_enabled(true).unwrap();
    assert!(!app.sync.sync_now().unwrap().skipped);
}

#[test]
fn consent_is_recorded_only_if_the_server_declares_sync() {
    let server = Arc::new(FakeSync::default());
    let app = App::new(&server);
    *app.info.sync.lock().unwrap() = false;
    app.sign_in("u1");
    app.consent();
    let settings = app.store().settings;
    assert_eq!((settings.mailbox_sync, settings.mailbox_sync_consent), (false, None));
    *app.info.sync.lock().unwrap() = true;
    app.consent();
    let settings = app.store().settings;
    assert_eq!((settings.mailbox_sync, settings.mailbox_sync_consent), (true, Some(URL.into())));
}

#[test]
fn a_first_pass_downloads_a_gmail_and_an_imap_mailbox_and_stores_their_credentials_in_the_vault() {
    let (_server, a, b) = pair();
    a.add_gmail("Giulia@Gmail.com", "1//refresh-token", CLIENT);
    a.add_imap("ufficio@example.com", "app-password");
    let sent = a.sync.sync_now().unwrap();
    assert_eq!(sent.uploaded, 2);
    let got = b.sync.sync_now().unwrap();
    assert_eq!(got.downloaded, 2);
    let store = b.store();
    assert_eq!(store.accounts.len(), 2);
    let gmail = b.account("giulia@gmail.com").unwrap();
    assert_eq!((gmail.provider_id.as_str(), gmail.oauth_client_id.as_str(), gmail.imap_host.as_str()), ("gmail", CLIENT, "imap.gmail.com"));
    assert!(gmail.password.is_empty(), "the credential is not in the store");
    assert_eq!(b.token("giulia@gmail.com"), "1//refresh-token");
    let imap = b.account("ufficio@example.com").unwrap();
    assert_eq!(imap.provider_id, "fastmail");
    assert_eq!(b.vault.get(&format!("provider:fastmail:{}", imap.id)).unwrap(), "app-password");
    assert!(b.events.0.lock().unwrap().iter().any(|e| e.contains("AccountsChanged")));
    // Nothing more to say on the next pass.
    let again = b.sync.sync_now().unwrap();
    assert_eq!((again.downloaded, again.uploaded), (0, 0));
}

#[test]
fn the_signature_text_travels_and_is_bound_to_the_new_mailbox() {
    let (_server, a, b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.context
        .update(|s| {
            s.signatures.push(qamille::domain::model::Signature { id: 7, name: "Mia".into(), body: "Giulia Rossi".into(), modified: 1, html: false });
            s.accounts.iter_mut().find(|x| x.id == id).unwrap().signature = SignatureBinding::Id(7);
            Ok(())
        })
        .unwrap();
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    let store = b.store();
    let account = store.accounts.iter().find(|x| x.email == "giulia@gmail.com").unwrap();
    let SignatureBinding::Id(sig) = account.signature else { panic!("not bound: {:?}", account.signature) };
    assert_eq!(store.signatures.iter().find(|s| s.id == sig).unwrap().body, "Giulia Rossi");
}

#[test]
fn the_credentials_go_only_to_the_session_address_with_the_device_key() {
    let (server, a, _b) = pair();
    a.add_gmail("giulia@gmail.com", "1//refresh-token", CLIENT);
    a.sync.sync_now().unwrap();
    let data = server.0.lock().unwrap();
    assert!(!data.endpoints.is_empty());
    assert!(data.endpoints.iter().all(|e| e == &format!("{URL}|spk_dev_secret-key")), "{:?}", data.endpoints);
}

#[test]
fn a_signature_two_mailboxes_share_is_not_rewritten_by_an_item_and_nothing_bounces() {
    use qamille::domain::model::Signature;
    let (server, a, b) = pair();
    let ida = a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.context
        .update(|s| {
            s.signatures.push(Signature { id: 1, name: "A".into(), body: "Firma di A".into(), modified: 1, html: false });
            s.accounts.iter_mut().find(|x| x.id == ida).unwrap().signature = SignatureBinding::Id(1);
            Ok(())
        })
        .unwrap();
    a.sync.sync_now().unwrap();
    // On the other computer two mailboxes use the signature 7.
    b.add_gmail("giulia@gmail.com", "t", CLIENT);
    b.add_gmail("altra@gmail.com", "t2", CLIENT);
    b.context
        .update(|s| {
            s.signatures.push(Signature { id: 7, name: "Comune".into(), body: "Firma comune".into(), modified: 1, html: false });
            for account in s.accounts.iter_mut() {
                account.signature = SignatureBinding::Id(7);
            }
            Ok(())
        })
        .unwrap();
    b.sync.sync_now().unwrap();
    assert_eq!(b.store().signatures.iter().find(|s| s.id == 7).unwrap().body, "Firma comune", "the shared signature stays");
    // Nothing goes back and forth: the next passes only look.
    server.clear_calls();
    b.sync.sync_now().unwrap();
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    assert!(server.calls().iter().all(|c| c.starts_with("list")), "{:?}", server.calls());
}

#[test]
fn the_custom_google_client_is_saved_only_if_missing_and_the_build_client_never_travels() {
    let (server, a, b) = pair();
    a.context.update(|s| { s.settings.google_client_id = CLIENT.into(); Ok(()) }).unwrap();
    a.vault.set("google-client-secret", "personal-secret").unwrap();
    a.add_gmail("giulia@gmail.com", "t1", CLIENT);
    a.add_gmail("work@example.com", "t2", "app.apps.googleusercontent.com");
    a.sync.sync_now().unwrap();
    let personal = server.item(&item_id("gmail", "giulia@gmail.com")).unwrap().1;
    assert_eq!(personal.credential.client_secret.as_deref(), Some("personal-secret"));
    let build = server.item(&item_id("gmail", "work@example.com")).unwrap().1;
    assert_eq!(build.credential.client_secret, None, "the secret of the build never travels");
    b.sync.sync_now().unwrap();
    assert_eq!(b.store().settings.google_client_id, CLIENT);
    assert_eq!(b.vault.get("google-client-secret").unwrap(), "personal-secret");
    // A computer that already has a client keeps it.
    let server2 = Arc::new(FakeSync::default());
    server2.write(personal.clone());
    let c = App::new(&server2);
    c.sign_in("u1");
    c.consent();
    c.context.update(|s| { s.settings.google_client_id = "mine.apps.googleusercontent.com".into(); Ok(()) }).unwrap();
    c.vault.set("google-client-secret", "mine").unwrap();
    c.sync.sync_now().unwrap();
    assert_eq!(c.store().settings.google_client_id, "mine.apps.googleusercontent.com");
    assert_eq!(c.vault.get("google-client-secret").unwrap(), "mine");
}

#[test]
fn a_local_mailbox_is_uploaded_with_base_version_zero_and_then_with_the_version() {
    let (server, a, _b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "t", CLIENT);
    let item = item_id("gmail", "giulia@gmail.com");
    a.sync.sync_now().unwrap();
    assert!(server.calls().contains(&format!("put {item} base 0")), "{:?}", server.calls());
    let version = server.item(&item).unwrap().0;
    // No change, no call.
    server.clear_calls();
    a.sync.sync_now().unwrap();
    assert!(server.calls().iter().all(|c| c.starts_with("list")), "{:?}", server.calls());
    // A change of name goes up with the version it started from.
    a.advance(10);
    a.context.update(|s| { s.accounts.iter_mut().find(|x| x.id == id).unwrap().name = "Nuovo nome".into(); Ok(()) }).unwrap();
    a.sync.sync_now().unwrap();
    assert!(server.calls().contains(&format!("put {item} base {version}")), "{:?}", server.calls());
    assert_eq!(server.item(&item).unwrap().1.account.name, "Nuovo nome");
}

#[test]
fn an_empty_credential_is_never_uploaded() {
    let (server, a, _b) = pair();
    a.add_gmail("giulia@gmail.com", "", CLIENT);
    a.sync.sync_now().unwrap();
    assert!(server.calls().iter().all(|c| c.starts_with("list")), "{:?}", server.calls());
}

#[test]
fn the_same_mailbox_on_both_computers_is_one_item() {
    let (server, a, b) = pair();
    a.add_gmail("giulia@gmail.com", "token-del-primo", CLIENT);
    b.add_gmail("giulia@gmail.com", "token-del-secondo", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    assert_eq!(b.store().accounts.len(), 1, "no double");
    assert_eq!(server.0.lock().unwrap().rows.len(), 1);
    assert_eq!(server.calls().iter().filter(|c| c.starts_with("put") && c.contains("base 0")).count(), 1, "the second computer joins the item instead of creating it: {:?}", server.calls());
    // The token that works here is not replaced by one nobody tried: it goes to the other computer.
    assert_eq!(b.token("giulia@gmail.com"), "token-del-secondo", "a working local credential stays");
    assert_eq!(server.item(&item_id("gmail", "giulia@gmail.com")).unwrap().1.credential.value, "token-del-secondo");
    a.sync.sync_now().unwrap();
    assert_eq!(a.token("giulia@gmail.com"), "token-del-secondo", "and the other computer follows");
}

#[test]
fn a_mailbox_not_changed_here_adopts_the_remote_copy_whole_even_with_an_older_clock() {
    let (server, a, b) = pair();
    a.add_gmail("giulia@gmail.com", "t1", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    let item = item_id("gmail", "giulia@gmail.com");
    // A computer whose clock is behind writes a new name and a new token.
    let mut remote = server.item(&item).unwrap().1;
    remote.account.name = "Dal server".into();
    remote.credential.value = "t2".into();
    remote.updated_at -= 500;
    remote.credential_updated_at -= 500;
    server.write(remote);
    b.sync.sync_now().unwrap();
    assert_eq!(b.account("giulia@gmail.com").unwrap().name, "Dal server");
    assert_eq!(b.token("giulia@gmail.com"), "t2");
    // It is not sent back.
    server.clear_calls();
    b.sync.sync_now().unwrap();
    assert!(server.calls().iter().all(|c| c.starts_with("list")), "{:?}", server.calls());
}

#[test]
fn a_conflicting_item_that_names_another_mailbox_is_not_applied() {
    let (server, a, _b) = pair();
    a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    let item = item_id("gmail", "giulia@gmail.com");
    // The server answers the conflict with the item of another address under this id.
    let mut foreign = server.item(&item).unwrap().1;
    foreign.account.email = "altra@gmail.com".into();
    foreign.account.username = "altra@gmail.com".into();
    foreign.credential.value = "token-altrui".into();
    {
        let mut data = server.0.lock().unwrap();
        data.counter += 1;
        let version = data.counter;
        data.rows.insert(item, Row { version, deleted: false, item: Some(foreign) });
    }
    a.advance(10);
    a.context.update(|s| { s.accounts[0].name = "Nome nuovo".into(); Ok(()) }).unwrap();
    let error = a.sync.sync_now().unwrap_err();
    assert_eq!(error.code, "server_error");
    assert_eq!(a.store().accounts.len(), 1);
    let account = a.account("giulia@gmail.com").unwrap();
    assert_eq!((account.email.as_str(), account.name.as_str()), ("giulia@gmail.com", "Nome nuovo"));
    assert_eq!(a.token("giulia@gmail.com"), "t");
}

#[test]
fn a_conflict_merges_by_parts_and_retries() {
    let (server, a, b) = pair();
    let ida = a.add_gmail("giulia@gmail.com", "t1", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    let item = item_id("gmail", "giulia@gmail.com");
    // B renews the token; before it is sent, A changes the name and sends it.
    b.advance(100);
    b.vault.set("gmail:giulia@gmail.com", "t2-renewed").unwrap();
    a.advance(50);
    a.context.update(|s| { s.accounts.iter_mut().find(|x| x.id == ida).unwrap().name = "Nome di A".into(); Ok(()) }).unwrap();
    a.sync.sync_now().unwrap();
    server.clear_calls();
    b.sync.sync_now().unwrap();
    // B had the old version: its put conflicted, the merge kept A's name and B's token.
    let (_, merged) = server.item(&item).unwrap();
    assert_eq!(merged.credential.value, "t2-renewed");
    assert_eq!(merged.account.name, "Nome di A");
    assert_eq!(b.account("giulia@gmail.com").unwrap().name, "Nome di A");
    a.sync.sync_now().unwrap();
    assert_eq!(a.token("giulia@gmail.com"), "t2-renewed", "EX-37: the new token arrives without doing anything");
}

#[test]
fn a_conflict_with_no_current_item_retries_with_base_zero() {
    let (server, a, _b) = pair();
    a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    let item = item_id("gmail", "giulia@gmail.com");
    // The server forgets the row (a base version that does not exist any more).
    server.0.lock().unwrap().rows.clear();
    a.advance(5);
    a.context.update(|s| { s.accounts[0].name = "Cambiata".into(); Ok(()) }).unwrap();
    a.sync.sync_now().unwrap();
    assert!(server.item(&item).is_some());
}

#[test]
fn a_remote_change_does_not_override_a_newer_local_change() {
    let (server, a, b) = pair();
    let ida = a.add_gmail("giulia@gmail.com", "t1", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    let item = item_id("gmail", "giulia@gmail.com");
    // B changes the colour long ago in server time, A changes the name afterwards.
    let mut stale = server.item(&item).unwrap().1;
    stale.account.color = [200, 0, 0];
    stale.updated_at -= 1000;
    server.write(stale);
    a.advance(100);
    a.context.update(|s| { s.accounts.iter_mut().find(|x| x.id == ida).unwrap().name = "Nome nuovo".into(); Ok(()) }).unwrap();
    a.sync.sync_now().unwrap();
    assert_eq!(a.account("giulia@gmail.com").unwrap().name, "Nome nuovo");
    let (_, now_on_server) = server.item(&item).unwrap();
    assert_eq!(now_on_server.account.name, "Nome nuovo");
}

#[test]
fn a_changed_epoch_reuploads_everything_without_removal_notices() {
    let (server, a, b) = pair();
    a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    server.reset();
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    assert!(a.sync.status().unwrap().removals.is_empty() && b.sync.status().unwrap().removals.is_empty());
    assert!(server.item(&item_id("gmail", "giulia@gmail.com")).is_some(), "the mailbox is back on the server");
    assert_eq!(b.store().accounts.len(), 1);
}

#[test]
fn a_402_pauses_with_the_subscription_message_and_keeps_local_mailboxes() {
    let (server, a, _b) = pair();
    a.add_gmail("giulia@gmail.com", "t", CLIENT);
    server.fail_next("subscription_required");
    let report = a.sync.sync_now().unwrap();
    assert!(report.paused);
    assert_eq!(a.sync.status().unwrap().mode, SyncMode::PausedForSubscription);
    assert_eq!(a.store().accounts.len(), 1);
    // The next pass tries again.
    a.sync.sync_now().unwrap();
    assert_eq!(a.sync.status().unwrap().mode, SyncMode::Idle);
}

#[test]
fn a_401_is_an_error_and_unreachable_keeps_the_pending_work() {
    let (server, a, _b) = pair();
    a.add_gmail("giulia@gmail.com", "t", CLIENT);
    server.fail_next("unauthorized");
    assert_eq!(a.sync.sync_now().unwrap_err().code, "unauthorized");
    assert_eq!(a.sync.status().unwrap().mode, SyncMode::Error);
    server.fail_next("unreachable");
    assert!(a.sync.sync_now().is_err());
    a.sync.sync_now().unwrap();
    assert!(server.item(&item_id("gmail", "giulia@gmail.com")).is_some());
}

#[test]
fn a_server_without_the_routes_is_unavailable_not_an_error() {
    let (server, a, _b) = pair();
    server.fail_next("not_found");
    assert!(a.sync.sync_now().unwrap().unavailable);
    assert_eq!(a.sync.status().unwrap().mode, SyncMode::Unavailable);
}

#[test]
fn tick_runs_at_startup_every_five_minutes_and_two_seconds_after_a_nudge() {
    use std::time::{Duration, Instant};
    let (server, a, _b) = pair();
    let t0 = Instant::now();
    assert!(a.sync.tick(t0).unwrap().is_some(), "at startup");
    assert!(a.sync.tick(t0 + Duration::from_secs(10)).unwrap().is_none());
    a.sync.nudge_at(t0 + Duration::from_secs(20));
    assert!(a.sync.tick(t0 + Duration::from_secs(21)).unwrap().is_none(), "grouped for two seconds");
    assert!(a.sync.tick(t0 + Duration::from_secs(22)).unwrap().is_some());
    assert!(a.sync.tick(t0 + Duration::from_secs(100)).unwrap().is_none());
    assert!(a.sync.tick(t0 + Duration::from_secs(22 + 301)).unwrap().is_some(), "every five minutes");
    assert!(!server.calls().is_empty());
}

#[test]
fn signing_out_forgets_consent_and_state_but_not_the_mailboxes() {
    let (_server, a, _b) = pair();
    a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    a.accounts.sign_out().unwrap();
    let settings = a.store().settings;
    assert_eq!((settings.mailbox_sync, settings.mailbox_sync_consent), (false, None));
    assert!(a.sync.sync_now().unwrap().skipped);
    assert_eq!(a.sync.status().unwrap().mode, SyncMode::NoServer);
    assert_eq!(a.store().accounts.len(), 1);
    assert_eq!(a.token("giulia@gmail.com"), "t");
    assert!(a.state.0.lock().unwrap().items.is_empty());
}

#[test]
fn signing_out_or_changing_user_keeps_the_switches_of_the_mailboxes() {
    let (_server, a, _b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "t", CLIENT);
    let item = item_id("gmail", "giulia@gmail.com");
    a.sync.set_mailbox(id, false).unwrap();
    a.sync.sync_now().unwrap();
    a.state.0.lock().unwrap().needs_reconnect.insert("revoked-here".into());
    a.accounts.sign_out().unwrap();
    assert!(a.sync.sync_now().unwrap().skipped);
    {
        let state = a.state.0.lock().unwrap();
        assert!(state.disabled.contains(&item) && state.needs_reconnect.contains("revoked-here"), "{state:?}");
        assert!(state.items.is_empty() && state.url.is_empty());
    }
    // Another person on the same computer: their state starts over, the mailbox stays «only here».
    a.sign_in("u2");
    a.consent();
    a.sync.sync_now().unwrap();
    let state = a.state.0.lock().unwrap();
    assert_eq!(state.user_id, "u2");
    assert!(state.disabled.contains(&item) && state.needs_reconnect.contains("revoked-here"), "{state:?}");
}

#[test]
fn a_change_made_while_a_pass_runs_is_not_lost_when_the_pass_saves() {
    let (server, a, _b) = pair();
    a.add_gmail("uno@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    let other = item_id("gmail", "altra@gmail.com");
    let (state, o) = (a.state.clone(), other.clone());
    server.0.lock().unwrap().before_list = Some(Box::new(move || {
        // What `set_mailbox`, `revoke_mailbox` and `after_local_removal` of another thread saved.
        let mut s = state.0.lock().unwrap();
        s.disabled.insert(o.clone());
        s.needs_reconnect.insert(o.clone());
        s.pending_deletes.push(PendingDelete { item_id: o.clone(), version: 3 });
    }));
    a.advance(10);
    a.sync.sync_now().unwrap();
    let s = a.state.0.lock().unwrap();
    assert!(s.disabled.contains(&other), "{s:?}");
    assert!(s.needs_reconnect.contains(&other), "{s:?}");
    assert!(s.pending_deletes.iter().any(|d| d.item_id == other), "{s:?}");
    assert!(s.items.contains_key(&item_id("gmail", "uno@gmail.com")), "the pass keeps its own result");
    assert!(s.last_sync.is_some());
}

#[test]
fn a_state_that_cannot_be_saved_is_an_error_and_not_a_silent_success() {
    let (_server, a, _b) = pair();
    let id = a.add_gmail("uno@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    a.advance(10);
    a.context.update(|s| { s.accounts[0].name = "Cambiato".into(); Ok(()) }).unwrap();
    a.state.1.store(true, Ordering::SeqCst);
    let error = a.sync.sync_now().unwrap_err();
    assert_eq!(error.code, "storage");
    assert_eq!(a.sync.status().unwrap().mode, SyncMode::Error);
    assert!(a.sync.set_mailbox(id, false).is_err(), "the switch does not pretend to be saved");
    a.state.1.store(false, Ordering::SeqCst);
    a.sync.sync_now().unwrap();
    assert_eq!(a.sync.status().unwrap().mode, SyncMode::Idle);
}

#[test]
fn save_settings_cannot_change_the_two_sync_fields() {
    let (_server, a, _b) = pair();
    let mut settings = a.store().settings;
    settings.mailbox_sync = false;
    settings.mailbox_sync_consent = None;
    settings.ai_provider = AiProvider::Offline;
    let workspace = qamille::application::workspace::WorkspaceService::new(a.context.clone());
    workspace.save_settings(qamille::application::workspace::SettingsUpdate { settings, ai_key: None, google_client_secret: None }).unwrap();
    let after = a.store().settings;
    assert!(after.mailbox_sync);
    assert_eq!(after.mailbox_sync_consent.as_deref(), Some(URL));
}

#[test]
fn the_state_and_the_status_contain_no_secret() {
    let (_server, a, _b) = pair();
    a.add_gmail("giulia@gmail.com", "1//very-secret-token", CLIENT);
    a.add_imap("ufficio@example.com", "very-secret-password");
    a.sync.sync_now().unwrap();
    let state = serde_json::to_string(&*a.state.0.lock().unwrap()).unwrap();
    let status = serde_json::to_string(&a.sync.status().unwrap()).unwrap();
    for text in [state, status, a.events.0.lock().unwrap().join("\n"), format!("{:?}", a.store().settings)] {
        assert!(!text.contains("very-secret") && !text.contains("spk_dev_"), "{text}");
    }
    let _ = CREDENTIAL_GOOGLE;
}

#[test]
fn only_one_pass_runs_at_a_time() {
    let (server, a, _b) = pair();
    a.add_gmail("giulia@gmail.com", "t", CLIENT);
    let a = Arc::new(a);
    let handles: Vec<_> = (0..4).map(|_| { let a = a.clone(); std::thread::spawn(move || a.sync.sync_now().unwrap()) }).collect();
    let reports: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert!(reports.iter().any(|r| !r.skipped));
    assert_eq!(server.0.lock().unwrap().bodies.len(), 1, "one upload only");
}

#[test]
fn a_hostile_item_from_the_server_is_not_applied() {
    let (server, a, _b) = pair();
    let mut item = {
        let (_s2, other, _) = pair();
        other.add_gmail("x@gmail.com", "t", CLIENT);
        other.sync.sync_now().unwrap();
        // Build the item from a real mailbox of another app.
        let id = item_id("gmail", "x@gmail.com");
        let (_, mut item) = (_s2.item(&id).unwrap().0, _s2.item(&id).unwrap().1);
        item.updated_at = 1;
        item
    };
    item.account.imap_host = "evil host/../x".into();
    server.write(item.clone());
    a.sync.sync_now().unwrap();
    assert!(a.store().accounts.is_empty());
    // An item whose id does not match its address is not applied either.
    item.account.imap_host = "imap.gmail.com".into();
    let mut data = server.0.lock().unwrap();
    data.counter += 1;
    let v = data.counter;
    data.rows.insert(item_id("gmail", "other@gmail.com"), fakes::Row { version: v, deleted: false, item: Some(item) });
    drop(data);
    a.sync.sync_now().unwrap();
    assert!(a.store().accounts.is_empty());
}

// ---- removals, switches and remove_account (task 3) -------------------------------------------

#[test]
fn a_tombstone_for_a_known_mailbox_asks_instead_of_deleting() {
    let (server, a, b) = pair();
    a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    let item = item_id("gmail", "giulia@gmail.com");
    server.remove(&item);
    let report = b.sync.sync_now().unwrap();
    assert_eq!(report.removal_notices, 1);
    let status = b.sync.status().unwrap();
    assert_eq!(status.removals.len(), 1);
    assert_eq!(status.removals[0].email, "giulia@gmail.com");
    assert_eq!(b.store().accounts.len(), 1, "nothing is deleted behind the person's back");
    assert_eq!(b.token("giulia@gmail.com"), "t");
    assert!(b.events.0.lock().unwrap().iter().any(|e| e.contains("removed or switched off on another computer")));
    // No second notice, and the mailbox is not recreated on the server.
    assert_eq!(b.sync.sync_now().unwrap().removal_notices, 0);
    assert!(server.item(&item).is_none());
}

#[test]
fn a_known_mailbox_missing_from_a_full_listing_counts_as_removed_but_not_from_an_incremental_one() {
    let (server, a, b) = pair();
    a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    // Incremental: nothing new, nothing missing.
    assert_eq!(b.sync.sync_now().unwrap().removal_notices, 0);
    // The tombstone was purged (90 days): only a full listing sees the item gone.
    server.0.lock().unwrap().rows.clear();
    assert_eq!(b.sync.sync_now().unwrap().removal_notices, 0);
    b.advance(3601);
    assert_eq!(b.sync.sync_now().unwrap().removal_notices, 1);
}

#[test]
fn a_tombstone_for_an_unknown_mailbox_is_ignored() {
    let (server, _a, b) = pair();
    server.remove(&item_id("gmail", "mai-vista@gmail.com"));
    assert_eq!(b.sync.sync_now().unwrap().removal_notices, 0);
    assert!(b.store().accounts.is_empty());
}

#[test]
fn keep_only_here_turns_the_mailbox_switch_off_and_stops_both_directions() {
    let (server, a, b) = pair();
    a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    let item = item_id("gmail", "giulia@gmail.com");
    server.remove(&item);
    b.sync.sync_now().unwrap();
    b.sync.resolve_removal(&item, false, &b.account_service).unwrap();
    assert!(b.sync.status().unwrap().removals.is_empty());
    assert!(!b.sync.status().unwrap().mailboxes[0].enabled);
    server.clear_calls();
    b.sync.sync_now().unwrap();
    assert!(server.calls().iter().all(|c| c.starts_with("list")), "{:?}", server.calls());
    assert!(server.item(&item).is_none());
}

#[test]
fn remove_here_too_deletes_the_mailbox_its_vault_value_mail_and_drafts_and_forgets_the_item() {
    let (server, a, b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    let item = item_id("gmail", "giulia@gmail.com");
    let bid = b.account("giulia@gmail.com").unwrap().id;
    b.context.update(|s| { s.emails.push(qamille::domain::model::Email { id: 1, account_id: bid, ..Default::default() }); Ok(()) }).unwrap();
    server.remove(&item);
    b.sync.sync_now().unwrap();
    b.sync.resolve_removal(&item, true, &b.account_service).unwrap();
    let store = b.store();
    assert!(store.accounts.is_empty() && store.emails.is_empty() && store.drafts.is_empty());
    assert_eq!(b.token("giulia@gmail.com"), "");
    assert!(b.sync.status().unwrap().removals.is_empty());
    server.clear_calls();
    b.sync.sync_now().unwrap();
    assert!(server.calls().iter().all(|c| c.starts_with("list")), "no delete is sent back: {:?}", server.calls());
    let _ = id;
}

#[test]
fn removing_a_mailbox_here_deletes_the_item_on_the_server_with_its_version() {
    let (server, a, b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    let item = item_id("gmail", "giulia@gmail.com");
    let version = server.item(&item).unwrap().0;
    let removed = a.account_service.remove_account(id).unwrap();
    a.sync.after_local_removal(&removed.provider_id, &removed.email).unwrap();
    server.fail_next("unreachable");
    assert!(a.sync.sync_now().is_err(), "the server is down: the deletion waits");
    server.clear_calls();
    a.sync.sync_now().unwrap();
    assert!(server.calls().contains(&format!("delete {item} base Some({version})")), "{:?}", server.calls());
    assert!(server.item(&item).is_none());
    // The other computer is asked.
    b.sync.sync_now().unwrap();
}

#[test]
fn a_mailbox_removed_here_does_not_come_back_from_a_full_listing_before_its_delete_is_sent() {
    let (server, a, _b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    let item = item_id("gmail", "giulia@gmail.com");
    let removed = a.account_service.remove_account(id).unwrap();
    a.sync.after_local_removal(&removed.provider_id, &removed.email).unwrap();
    // The next pass lists everything (an hour went by): the item is still there.
    a.advance(3601);
    a.sync.sync_now().unwrap();
    assert!(a.store().accounts.is_empty(), "the removed mailbox is not recreated");
    assert!(server.item(&item).is_none(), "and its item is deleted");
    assert_eq!(a.token("giulia@gmail.com"), "");
}

#[test]
fn switching_a_mailbox_off_takes_its_credential_off_the_server_and_the_others_are_told_in_true_words() {
    let (server, a, b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    let item = item_id("gmail", "giulia@gmail.com");
    a.sync.set_mailbox(id, false).unwrap();
    a.sync.sync_now().unwrap();
    assert!(server.item(&item).is_none(), "«only on this computer»: the server no longer keeps it");
    assert_eq!((a.store().accounts.len(), a.token("giulia@gmail.com").as_str()), (1, "t"));
    // The other computer is asked, and the words say «removed or switched off», not just «removed».
    b.sync.sync_now().unwrap();
    assert_eq!(b.sync.status().unwrap().removals.len(), 1);
    assert!(b.events.0.lock().unwrap().iter().any(|e| e.contains("removed or switched off")), "{:?}", b.events.0.lock().unwrap());
    assert_eq!(b.store().accounts.len(), 1, "nothing is deleted there");
    // Switched on again it goes back up, and the question of the other computer goes away.
    a.sync.set_mailbox(id, true).unwrap();
    a.sync.sync_now().unwrap();
    assert!(server.item(&item).is_some());
    b.sync.sync_now().unwrap();
    assert!(b.sync.status().unwrap().removals.is_empty());
}

#[test]
fn an_invalid_mailbox_is_skipped_with_a_reason_and_the_others_still_go_up() {
    let (server, a, _b) = pair();
    let bad = a.add_gmail("rotta@gmail.com", "t", CLIENT);
    let good = a.add_gmail("buona@gmail.com", "t", CLIENT);
    a.context.update(|s| { s.accounts.iter_mut().find(|x| x.id == bad).unwrap().imap_host = String::new(); Ok(()) }).unwrap();
    let report = a.sync.sync_now().unwrap();
    assert_eq!(report.uploaded, 1);
    assert!(server.item(&item_id("gmail", "buona@gmail.com")).is_some());
    assert!(server.item(&item_id("gmail", "rotta@gmail.com")).is_none(), "an item the server would refuse is not sent");
    let status = a.sync.status().unwrap();
    assert_eq!(status.mode, SyncMode::Idle, "one mailbox does not put the service in error");
    let problems: Vec<(u32, bool)> = status.mailboxes.iter().map(|m| (m.account_id, m.problem.is_some())).collect();
    assert_eq!(problems, [(bad, true), (good, false)]);
    // Fixed, it goes up and the problem is gone.
    a.advance(10);
    a.context.update(|s| { s.accounts.iter_mut().find(|x| x.id == bad).unwrap().imap_host = "imap.gmail.com".into(); Ok(()) }).unwrap();
    a.sync.sync_now().unwrap();
    assert!(server.item(&item_id("gmail", "rotta@gmail.com")).is_some());
    assert!(a.sync.status().unwrap().mailboxes.iter().all(|m| m.problem.is_none()));
}

#[test]
fn a_mailbox_the_server_refuses_is_a_problem_of_that_mailbox_until_its_content_changes() {
    for code in ["invalid_request", "payload_too_large", "sync_limit"] {
        let (server, a, _b) = pair();
        let big = a.add_gmail("grande@gmail.com", "t", CLIENT);
        a.add_gmail("piccola@gmail.com", "t", CLIENT);
        let big_item = item_id("gmail", "grande@gmail.com");
        server.0.lock().unwrap().fail_put.insert(big_item.clone(), code);
        let puts = |server: &FakeSync| server.calls().iter().filter(|c| c.starts_with(&format!("put {big_item}"))).count();
        let report = a.sync.sync_now().unwrap();
        assert_eq!(report.uploaded, 1, "{code}");
        let status = a.sync.status().unwrap();
        assert_eq!(status.mode, SyncMode::Idle, "{code}");
        assert!(status.mailboxes.iter().any(|m| m.account_id == big && m.problem.is_some()), "{code}");
        assert_eq!(puts(&server), 1);
        // Nothing changed: it is not asked again.
        a.advance(10);
        a.sync.sync_now().unwrap();
        assert_eq!(puts(&server), 1, "{code}");
        // Changed, it is tried again.
        server.0.lock().unwrap().fail_put.clear();
        a.advance(10);
        a.context.update(|s| { s.accounts.iter_mut().find(|x| x.id == big).unwrap().name = "Più corta".into(); Ok(()) }).unwrap();
        a.sync.sync_now().unwrap();
        assert_eq!(puts(&server), 2, "{code}");
        assert!(server.item(&big_item).is_some());
        assert!(a.sync.status().unwrap().mailboxes.iter().all(|m| m.problem.is_none()), "{code}");
    }
}

#[test]
fn a_mailbox_the_server_refused_is_asked_again_at_the_next_full_listing() {
    let (server, a, _b) = pair();
    a.add_gmail("grande@gmail.com", "t", CLIENT);
    let item = item_id("gmail", "grande@gmail.com");
    server.0.lock().unwrap().fail_put.insert(item.clone(), "sync_limit");
    a.sync.sync_now().unwrap();
    server.0.lock().unwrap().fail_put.clear();
    a.advance(3601);
    a.sync.sync_now().unwrap();
    assert!(server.item(&item).is_some(), "the limit was lifted meanwhile");
}

#[test]
fn a_mailbox_removed_without_the_service_is_noticed_at_the_next_pass() {
    let (server, a, _b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.sync_now().unwrap();
    a.account_service.remove_account(id).unwrap();
    a.sync.sync_now().unwrap();
    assert!(server.item(&item_id("gmail", "giulia@gmail.com")).is_none());
}

#[test]
fn a_disabled_mailbox_is_neither_uploaded_nor_updated_and_resumes_when_turned_on() {
    let (server, a, b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.set_mailbox(id, false).unwrap();
    a.sync.sync_now().unwrap();
    assert!(server.0.lock().unwrap().rows.is_empty(), "nothing is sent: {:?}", server.0.lock().unwrap().rows.keys().collect::<Vec<_>>());
    // A remote item for a disabled mailbox is ignored.
    let item = item_id("gmail", "giulia@gmail.com");
    a.sync.set_mailbox(id, true).unwrap();
    a.sync.sync_now().unwrap();
    assert!(server.item(&item).is_some());
    b.sync.sync_now().unwrap();
    let bid = b.account("giulia@gmail.com").unwrap().id;
    b.sync.set_mailbox(bid, false).unwrap();
    a.advance(10);
    a.vault.set("gmail:giulia@gmail.com", "t-renewed").unwrap();
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    assert_eq!(b.token("giulia@gmail.com"), "t", "a disabled mailbox does not follow");
}

#[test]
fn remove_account_rejects_an_unknown_id() {
    let (_server, a, _b) = pair();
    assert_eq!(a.account_service.remove_account(99).unwrap_err().code, "not_found");
}

// ---- revocation (task 4) ------------------------------------------------------------------------

#[test]
fn revoking_a_mailbox_clears_its_credential_and_the_old_token_does_not_come_back() {
    let (server, a, b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "1//old-token", CLIENT);
    a.sync.sync_now().unwrap();
    b.sync.sync_now().unwrap();
    let revoked = a.sync.revoke_mailbox(id).unwrap();
    assert_eq!(revoked.outcome, qamille::application::sync::RevokeOutcome::Revoked);
    assert_eq!(*a.revoker.revoked.lock().unwrap(), ["1//old-token"]);
    assert_eq!(a.token("giulia@gmail.com"), "");
    assert!(a.sync.status().unwrap().mailboxes[0].needs_reconnect);
    // The server still has the old token: it is neither taken back nor used to overwrite it.
    server.clear_calls();
    a.advance(5);
    a.sync.sync_now().unwrap();
    assert_eq!(a.token("giulia@gmail.com"), "", "a revoked token is not restored");
    assert!(server.calls().iter().all(|c| c.starts_with("list")), "{:?}", server.calls());
    // Signing in again uploads the new token; the other computer takes it.
    a.advance(5);
    a.vault.set("gmail:giulia@gmail.com", "1//new-token").unwrap();
    a.sync.sync_now().unwrap();
    assert!(!a.sync.status().unwrap().mailboxes[0].needs_reconnect);
    assert_eq!(server.item(&item_id("gmail", "giulia@gmail.com")).unwrap().1.credential.value, "1//new-token");
    b.sync.sync_now().unwrap();
    assert_eq!(b.token("giulia@gmail.com"), "1//new-token");
}

#[test]
fn a_failed_revoke_keeps_the_credential_and_says_failed_and_retry_works() {
    let (_server, a, _b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "1//token", CLIENT);
    *a.revoker.fail.lock().unwrap() = true;
    let first = a.sync.revoke_mailbox(id).unwrap();
    assert_eq!(first.outcome, qamille::application::sync::RevokeOutcome::Failed);
    assert_eq!(a.token("giulia@gmail.com"), "1//token");
    assert!(!a.sync.status().unwrap().mailboxes[0].needs_reconnect);
    *a.revoker.fail.lock().unwrap() = false;
    assert_eq!(a.sync.revoke_mailbox(id).unwrap().outcome, qamille::application::sync::RevokeOutcome::Revoked);
}

#[test]
fn an_imap_password_is_reported_as_manual_and_left_alone() {
    let (_server, a, _b) = pair();
    let id = a.add_imap("ufficio@example.com", "app-password");
    let result = a.sync.revoke_mailbox(id).unwrap();
    assert_eq!(result.outcome, qamille::application::sync::RevokeOutcome::ManualPassword);
    assert!(a.revoker.revoked.lock().unwrap().is_empty());
    assert_eq!(a.vault.get(&format!("provider:fastmail:{id}")).unwrap(), "app-password");
}

#[test]
fn revoke_all_covers_the_mailboxes_this_computer_synchronizes_and_not_the_ones_kept_only_here() {
    let (_server, a, _b) = pair();
    a.add_gmail("uno@gmail.com", "t1", CLIENT);
    a.add_gmail("due@gmail.com", "t2", CLIENT);
    a.add_imap("ufficio@example.com", "pw");
    let solo = a.add_gmail("solo@gmail.com", "t-solo", CLIENT);
    a.sync.set_mailbox(solo, false).unwrap();
    a.sync.sync_now().unwrap();
    let all = a.sync.revoke_all().unwrap();
    assert_eq!(all.len(), 3);
    assert_eq!(a.revoker.revoked.lock().unwrap().len(), 2);
    assert!(!a.revoker.revoked.lock().unwrap().iter().any(|t| t == "t-solo"), "it never left this computer: it keeps working");
    assert_eq!(a.token("solo@gmail.com"), "t-solo");
}

#[test]
fn a_mailbox_that_fails_to_revoke_does_not_stop_the_others() {
    let (_server, a, _b) = pair();
    a.add_gmail("uno@gmail.com", "t1", CLIENT);
    a.add_gmail("due@gmail.com", "t2", CLIENT);
    a.add_gmail("tre@gmail.com", "t3", CLIENT);
    a.sync.sync_now().unwrap();
    a.vault.broken.lock().unwrap().push("gmail:due@gmail.com".into());
    let all = a.sync.revoke_all().unwrap();
    let outcomes: Vec<_> = all.iter().map(|r| (r.email.as_str(), r.outcome)).collect();
    use qamille::application::sync::RevokeOutcome::{Failed, Revoked};
    assert_eq!(outcomes, [("uno@gmail.com", Revoked), ("due@gmail.com", Failed), ("tre@gmail.com", Revoked)]);
}

#[test]
fn the_reminder_to_sign_in_again_is_saved_before_the_token_leaves_the_vault() {
    let (_server, a, _b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "1//old", CLIENT);
    a.sync.sync_now().unwrap();
    a.state.1.store(true, Ordering::SeqCst);
    assert!(a.sync.revoke_mailbox(id).is_err(), "the state could not be saved");
    assert_eq!(a.token("giulia@gmail.com"), "1//old", "the token is still where the next pass expects it");
    a.state.1.store(false, Ordering::SeqCst);
}

#[test]
fn a_revoked_mailbox_is_not_uploaded_without_a_credential() {
    let (server, a, _b) = pair();
    let id = a.add_gmail("giulia@gmail.com", "t", CLIENT);
    a.sync.revoke_mailbox(id).unwrap();
    a.sync.sync_now().unwrap();
    assert!(server.0.lock().unwrap().rows.is_empty());
}

#[test]
fn switching_session_while_listing_stops_the_pass_before_upload() {
    use qamille::application::server_account::SERVER_SESSION;
    use qamille::domain::server_account::ServerSession;

    let (server, a, _) = pair();
    a.add_gmail("local@gmail.com", "local-token", CLIENT);
    let vault = Arc::clone(&a.vault);
    server.0.lock().unwrap().before_list = Some(Box::new(move || {
        let mut session: ServerSession = serde_json::from_str(&vault.get(SERVER_SESSION).unwrap()).unwrap();
        session.url = "https://other.example".into();
        session.user_id = "other".into();
        session.device_key = "other-device".into();
        vault.set(SERVER_SESSION, &serde_json::to_string(&session).unwrap()).unwrap();
    }));
    assert!(a.sync.sync_now().unwrap().skipped);
    assert!(server.0.lock().unwrap().bodies.is_empty());
    assert_eq!(a.token("local@gmail.com"), "local-token");
}

#[test]
fn a_restored_older_version_reconciles_even_if_the_epoch_was_not_rotated() {
    let (server, a, _) = pair();
    let kept = item_id("gmail", "kept@gmail.com");
    let missing = item_id("gmail", "missing@gmail.com");
    a.add_gmail("kept@gmail.com", "first", CLIENT);
    a.add_gmail("missing@gmail.com", "second", CLIENT);
    a.sync.sync_now().unwrap();
    a.sync.sync_now().unwrap();
    assert_eq!(a.state.0.lock().unwrap().since, 2);
    {
        let mut data = server.0.lock().unwrap();
        data.counter = 1;
        data.rows.remove(&missing);
        let row = data.rows.get_mut(&kept).unwrap();
        row.version = 1;
        row.item.as_mut().unwrap().account.name = "Restored".into();
    }
    a.sync.sync_now().unwrap();
    assert_eq!(a.account("kept@gmail.com").unwrap().name, "Restored");
    assert!(server.item(&missing).is_some());
    assert!(a.sync.status().unwrap().removals.is_empty());
    let mut updated = server.item(&kept).unwrap().1;
    updated.account.name = "After recovery".into();
    server.write(updated);
    a.sync.sync_now().unwrap();
    assert_eq!(a.account("kept@gmail.com").unwrap().name, "After recovery");
}

#[test]
fn a_live_unreadable_mailbox_survives_conflicts_on_old_and_new_devices() {
    let (server, a, b) = pair();
    let address = "future@gmail.com";
    let id = item_id("gmail", address);
    a.add_gmail(address, "t1", CLIENT);
    a.sync.sync_now().unwrap();
    b.add_gmail(address, "local", CLIENT);
    a.context.update(|s| { s.accounts[0].name = "Renamed".into(); Ok(()) }).unwrap();
    {
        let mut data = server.0.lock().unwrap();
        data.counter = 2;
        data.rows.insert(id.clone(), Row { version: 2, deleted: false, item: None });
    }
    for app in [&a, &b] {
        assert_eq!(app.sync.sync_now().unwrap().uploaded, 0);
        let status = app.sync.status().unwrap();
        assert!(status.removals.is_empty());
        assert!(status.mailboxes[0].problem.is_some());
    }
    let data = server.0.lock().unwrap();
    let row = data.rows.get(&id).unwrap();
    assert_eq!(row.version, 2);
    assert!(!row.deleted);
    assert!(row.item.is_none());
}
