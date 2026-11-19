//! La configurazione della sincronizzazione: `QAMILLE_SYNC_KEY` solo dall'ambiente, i limiti.
#![allow(dead_code)]

mod support;

use std::collections::HashMap;

use qamille_server::config::{Config, ConfigError};
use support::TempDir;

const KEY: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const OLD: &str = "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210";

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> =
        pairs.iter().map(|(k, v)| ((*k).to_owned(), (*v).to_owned())).collect();
    move |name| map.get(name).cloned()
}

fn load(dir: &TempDir, extra: &[(&str, &str)]) -> Result<Config, ConfigError> {
    let data = dir.path().to_str().unwrap().to_owned();
    let mut pairs = vec![("QAMILLE_DATA_DIR", data.as_str())];
    pairs.extend_from_slice(extra);
    Config::load_with(None, env_of(&pairs))
}

#[test]
fn sync_is_off_without_the_key_and_the_limits_default() {
    let dir = TempDir::new();
    let c = load(&dir, &[]).unwrap();
    assert!(c.sync.key.is_none() && c.sync.old_key.is_none());
    assert!(!c.sync.enabled());
    assert_eq!((c.sync.max_items, c.sync.max_item_bytes), (200, 65_536));
}

#[test]
fn the_key_comes_only_from_the_environment() {
    let dir = TempDir::new();
    let c = load(&dir, &[("QAMILLE_SYNC_KEY", KEY)]).unwrap();
    assert!(c.sync.enabled());
    // Una variabile vuota (come la lascia un `.env`) non conta.
    assert!(!load(&dir, &[("QAMILLE_SYNC_KEY", "  ")]).unwrap().sync.enabled());
    let file = dir.path().join("server.toml");
    std::fs::write(&file, format!("sync_key = \"{KEY}\"\n")).unwrap();
    let error = Config::load_with(Some(&file), env_of(&[])).unwrap_err();
    assert!(error.to_string().contains("unknown field `sync_key`"), "{error}");
    assert!(!error.to_string().contains(KEY));
}

#[test]
fn a_key_of_the_wrong_format_stops_the_loading_without_echoing_it() {
    let dir = TempDir::new();
    let short = &KEY[..63];
    let weak = "ab".repeat(32);
    let junk = "zz".repeat(32);
    for bad in [short, weak.as_str(), junk.as_str()] {
        let error = load(&dir, &[("QAMILLE_SYNC_KEY", bad)]).unwrap_err();
        let text = error.to_string();
        assert!(text.contains("QAMILLE_SYNC_KEY"), "{text}");
        assert!(!text.contains(&bad[..8]) || bad == junk, "{text}");
        assert!(!text.contains(short), "{text}");
    }
}

#[test]
fn the_old_key_is_read_and_validated_like_the_new_one() {
    let dir = TempDir::new();
    let c = load(&dir, &[("QAMILLE_SYNC_KEY", KEY), ("QAMILLE_SYNC_KEY_OLD", OLD)]).unwrap();
    assert!(c.sync.old_key.is_some());
    let error = load(&dir, &[("QAMILLE_SYNC_KEY", KEY), ("QAMILLE_SYNC_KEY_OLD", "abc")]).unwrap_err();
    assert!(error.to_string().contains("QAMILLE_SYNC_KEY_OLD"), "{error}");
}

#[test]
fn the_debug_of_the_configuration_hides_both_keys() {
    let dir = TempDir::new();
    let c = load(&dir, &[("QAMILLE_SYNC_KEY", KEY), ("QAMILLE_SYNC_KEY_OLD", OLD)]).unwrap();
    for text in [format!("{c:?}"), format!("{c:#?}"), format!("{:?}", c.sync)] {
        assert!(!text.contains(KEY) && !text.contains(&KEY[..16]), "{text}");
        assert!(!text.contains(OLD) && !text.contains(&OLD[..16]), "{text}");
        assert!(text.contains("<nascosta>"), "{text}");
    }
}

#[test]
fn the_limits_come_from_file_and_environment_and_are_bounded() {
    let dir = TempDir::new();
    let file = dir.path().join("server.toml");
    std::fs::write(&file, "sync_max_items = 5\nsync_max_item_bytes = 2048\n").unwrap();
    let c = Config::load_with(Some(&file), env_of(&[])).unwrap();
    assert_eq!((c.sync.max_items, c.sync.max_item_bytes), (5, 2048));
    let c = Config::load_with(
        Some(&file),
        env_of(&[("QAMILLE_SYNC_MAX_ITEMS", "7"), ("QAMILLE_SYNC_MAX_ITEM_BYTES", "4096")]),
    )
    .unwrap();
    assert_eq!((c.sync.max_items, c.sync.max_item_bytes), (7, 4096));
    for (name, value, key) in [
        ("QAMILLE_SYNC_MAX_ITEMS", "0", "sync_max_items"),
        ("QAMILLE_SYNC_MAX_ITEMS", "10001", "sync_max_items"),
        ("QAMILLE_SYNC_MAX_ITEM_BYTES", "1023", "sync_max_item_bytes"),
        ("QAMILLE_SYNC_MAX_ITEM_BYTES", "1048577", "sync_max_item_bytes"),
    ] {
        let error = load(&dir, &[(name, value)]).unwrap_err();
        assert!(error.to_string().contains(key), "{name}={value}: {error}");
    }
    assert!(load(&dir, &[("QAMILLE_SYNC_MAX_ITEMS", "10000"), ("QAMILLE_SYNC_MAX_ITEM_BYTES", "6000")]).is_ok());
    assert!(load(&dir, &[("QAMILLE_SYNC_MAX_ITEMS", "60"), ("QAMILLE_SYNC_MAX_ITEM_BYTES", "1048576")]).is_ok());
}

#[test]
fn the_core_keys_list_has_the_sync_limits() {
    let dir = TempDir::new();
    let file = dir.path().join("server.toml");
    std::fs::write(&file, "sync_max_items = 10\nsync_max_item_bytes = 4096\n").unwrap();
    // `load_split` restituisce la tabella delle chiavi che non sono del core: le due non ci sono.
    let (_, rest) = Config::load_split(Some(&file), env_of(&[])).unwrap();
    assert!(rest.is_empty(), "{rest:?}");
}

#[test]
fn the_total_a_user_may_store_is_bounded() {
    // `GET` apre e manda tutti gli elementi dell'utente: la somma ha un tetto di 64 MiB.
    let dir = TempDir::new();
    let ok = [("QAMILLE_SYNC_MAX_ITEMS", "1024"), ("QAMILLE_SYNC_MAX_ITEM_BYTES", "65536")];
    assert!(load(&dir, &ok).is_ok());
    let too_much = [("QAMILLE_SYNC_MAX_ITEMS", "10000"), ("QAMILLE_SYNC_MAX_ITEM_BYTES", "1048576")];
    let error = load(&dir, &too_much).unwrap_err().to_string();
    assert!(error.contains("sync_max_items") && error.contains("64 MiB"), "{error}");
    // Il predefinito ci sta largo.
    assert!(load(&dir, &[]).is_ok());
}
