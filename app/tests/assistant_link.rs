//! The link of the assistant of the server (gateway LLM, wave 5): session, consent of the
//! address and consent to the provider, read from the vault and the store at every call.
use qamille::application::AppResult;
use qamille::application::ports::{SecretStore, StoreRepository};
use qamille::application::server_account::SERVER_SESSION;
use qamille::domain::model::{ComputeMode, LlmConsent, Store};
use qamille::infrastructure::compute_factory::ComputeFactory;
use qamille::infrastructure::repository::MemoryRepository;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Vault(Mutex<HashMap<String, String>>);
impl SecretStore for Vault {
    fn get(&self, key: &str) -> AppResult<String> {
        Ok(self.0.lock().unwrap().get(key).cloned().unwrap_or_default())
    }
    fn set(&self, key: &str, value: &str) -> AppResult<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
}

const URL: &str = "https://server.example";
const KEY: &str = "spk_dev_secret_value";

fn session(url: &str) -> String {
    format!(r#"{{"url":"{url}","user_id":"u","username":"m","device_id":"d","device_key":"{KEY}"}}"#)
}
fn consent(url: &str) -> Option<LlmConsent> {
    Some(LlmConsent { url: url.into(), label: "OpenAI".into(), external: true })
}
fn factory(with_session: bool, address_consent: bool, llm: Option<LlmConsent>, mode: ComputeMode) -> ComputeFactory {
    let mut store = Store::default();
    store.settings.compute_mode = mode;
    if address_consent {
        store.settings.compute_consented_urls.push(URL.into());
    }
    store.settings.llm_consent = llm;
    let vault = Arc::new(Vault::default());
    if with_session {
        vault.set(SERVER_SESSION, &session(URL)).unwrap();
    }
    let repository: Arc<dyn StoreRepository> = Arc::new(MemoryRepository::new(store));
    ComputeFactory::with(repository, vault)
}

#[test]
fn assistant_link_needs_session_address_consent_and_llm_consent() {
    assert!(factory(false, true, consent(URL), ComputeMode::Server).assistant_link().is_none());
    assert!(factory(true, false, consent(URL), ComputeMode::Server).assistant_link().is_none());
    assert!(factory(true, true, None, ComputeMode::Server).assistant_link().is_none());
    let link = factory(true, true, consent(URL), ComputeMode::Server).assistant_link().unwrap();
    assert_eq!((link.url.as_str(), link.key.as_str(), link.consent.as_str()), (URL, KEY, "true;OpenAI"));
}

#[test]
fn assistant_link_is_none_for_another_address_consent() {
    assert!(factory(true, true, consent("https://other.example"), ComputeMode::Server).assistant_link().is_none());
}

#[test]
fn assistant_link_does_not_need_server_compute_mode() {
    assert!(factory(true, true, consent(URL), ComputeMode::Off).assistant_link().is_some());
}

#[test]
fn assistant_link_debug_hides_the_key() {
    let link = factory(true, true, consent(URL), ComputeMode::Server).assistant_link().unwrap();
    assert!(!format!("{link:?}").contains(KEY));
}
