//! I comandi `sync keygen`, `sync rekey` e `sync reset --yes`, con il binario vero.
#![allow(dead_code)]

mod support;

use std::process::{Command, Output};

use qamille_server::sync::SyncKey;
use qamille_server::sync::store::{self, Probe};
use support::sync_client::{SyncClient, account_item, item_id};
use support::{TEST_SYNC_KEY, TempDir, TestServer, sync_config, user_with_device};

const NEW_KEY: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";
const THIRD_KEY: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

fn run(dir: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qamille-server"));
    cmd.args(args).env_clear().env("QAMILLE_DATA_DIR", dir).current_dir(dir);
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("avvio del binario")
}

fn out(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

async fn seeded(items: u32) -> TestServer {
    let ts = TestServer::new(sync_config());
    let mario = user_with_device(&ts, "mario", "una password lunga").await;
    let client = SyncClient::new(&ts.public, &mario.device_key);
    for n in 1..=items {
        assert_eq!(client.put(&item_id(n), 0, account_item("MARKER")).await.status, 200);
    }
    ts
}

async fn probe(ts: &TestServer, key: &str) -> Probe {
    let key = SyncKey::parse(key).unwrap();
    ts.accounts().db().blocking(move |c| Ok(store::probe(c, &key).unwrap())).await.unwrap()
}

#[test]
fn keygen_prints_one_valid_key_and_nothing_else() {
    let dir = TempDir::new();
    let missing = dir.path().join("non-esiste");
    let first = run(dir.path(), &["sync", "keygen"], &[("QAMILLE_DATA_DIR", missing.to_str().unwrap())]);
    assert_eq!(first.status.code(), Some(0), "{}", err(&first));
    let text = out(&first);
    assert!(text.ends_with('\n') && text.matches('\n').count() == 1, "{text:?}");
    let key = text.trim_end();
    assert_eq!(key.len(), 64);
    assert!(SyncKey::parse(key).is_ok());
    assert!(!err(&first).contains(key));
    assert!(err(&first).contains("QAMILLE_SYNC_KEY"));
    let second = run(dir.path(), &["sync", "keygen"], &[]);
    assert_ne!(out(&second).trim_end(), key);
    assert!(!missing.exists() && !dir.path().join("server.db").exists(), "keygen non tocca il database");
}

#[tokio::test]
async fn rekey_reencrypts_with_the_new_key_and_leaves_the_old_one_useless() {
    let ts = seeded(2).await;
    assert_eq!(probe(&ts, TEST_SYNC_KEY).await, Probe::Opens);
    let o = run(ts.dir.path(), &["sync", "rekey"], &[("QAMILLE_SYNC_KEY", NEW_KEY), ("QAMILLE_SYNC_KEY_OLD", TEST_SYNC_KEY)]);
    assert_eq!(o.status.code(), Some(0), "{}", err(&o));
    assert!(out(&o).is_empty());
    assert!(err(&o).contains("2 elementi"), "{}", err(&o));
    for key in [NEW_KEY, TEST_SYNC_KEY] {
        assert!(!err(&o).contains(key) && !out(&o).contains(key));
    }
    assert_eq!(probe(&ts, NEW_KEY).await, Probe::Opens);
    assert_eq!(probe(&ts, TEST_SYNC_KEY).await, Probe::DoesNotOpen);
}

#[tokio::test]
async fn rekey_needs_both_keys_and_different_ones() {
    let ts = seeded(1).await;
    for env in [
        vec![("QAMILLE_SYNC_KEY", NEW_KEY)],
        vec![("QAMILLE_SYNC_KEY_OLD", TEST_SYNC_KEY)],
        vec![("QAMILLE_SYNC_KEY", TEST_SYNC_KEY), ("QAMILLE_SYNC_KEY_OLD", TEST_SYNC_KEY)],
        vec![],
    ] {
        let o = run(ts.dir.path(), &["sync", "rekey"], &env);
        assert_eq!(o.status.code(), Some(1), "{env:?}: {}", err(&o));
        assert!(err(&o).contains("QAMILLE_SYNC_KEY"), "{}", err(&o));
    }
    assert_eq!(probe(&ts, TEST_SYNC_KEY).await, Probe::Opens);
}

#[tokio::test]
async fn rekey_with_a_wrong_old_key_changes_nothing() {
    let ts = seeded(2).await;
    let o = run(ts.dir.path(), &["sync", "rekey"], &[("QAMILLE_SYNC_KEY", NEW_KEY), ("QAMILLE_SYNC_KEY_OLD", THIRD_KEY)]);
    assert_eq!(o.status.code(), Some(1), "{}", err(&o));
    assert!(!err(&o).contains(THIRD_KEY) && !err(&o).contains(NEW_KEY));
    assert_eq!(probe(&ts, TEST_SYNC_KEY).await, Probe::Opens);
}

#[tokio::test]
async fn reset_needs_yes_and_then_deletes_everything_without_the_key() {
    let ts = seeded(3).await;
    let refused = run(ts.dir.path(), &["sync", "reset"], &[]);
    assert_eq!(refused.status.code(), Some(2), "{}", err(&refused));
    assert_eq!(probe(&ts, TEST_SYNC_KEY).await, Probe::Opens);
    // Senza QAMILLE_SYNC_KEY funziona: è proprio il rimedio a una chiave persa.
    let done = run(ts.dir.path(), &["sync", "reset", "--yes"], &[]);
    assert_eq!(done.status.code(), Some(0), "{}", err(&done));
    assert!(err(&done).contains("3 righe"), "{}", err(&done));
    assert_eq!(probe(&ts, TEST_SYNC_KEY).await, Probe::Empty);
    let (version, epoch): (i64, i64) = ts
        .accounts()
        .db()
        .blocking(|c| Ok(c.query_row("SELECT version, epoch FROM sync_state", [], |r| Ok((r.get(0)?, r.get(1)?)))?))
        .await
        .unwrap();
    assert_eq!((version, epoch), (3, 1));
}

#[test]
fn the_help_lists_the_sync_commands() {
    let dir = TempDir::new();
    let o = run(dir.path(), &["--help"], &[]);
    let text = out(&o);
    for line in ["sync keygen", "sync rekey", "sync reset --yes"] {
        assert!(text.contains(line), "{text}");
    }
}

#[test]
fn an_unknown_sync_command_is_a_usage_error() {
    let dir = TempDir::new();
    for args in [&["sync"][..], &["sync", "boh"], &["sync", "reset", "--si"], &["sync", "keygen", "x"]] {
        let o = run(dir.path(), args, &[]);
        assert_eq!(o.status.code(), Some(2), "{args:?}: {}", err(&o));
    }
}

#[test]
fn a_key_of_the_wrong_format_stops_the_commands_without_echoing_it() {
    let dir = TempDir::new();
    let bad = "chiave-sbagliata-di-prova";
    let o = run(dir.path(), &["user", "list"], &[("QAMILLE_SYNC_KEY", bad)]);
    assert_eq!(o.status.code(), Some(1));
    assert!(err(&o).contains("QAMILLE_SYNC_KEY"), "{}", err(&o));
    assert!(!err(&o).contains(bad), "{}", err(&o));
}
