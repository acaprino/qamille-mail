use qamille::{
    application::{AppResult, ServiceContext, mail::MailService, ports::*},
    domain::outbox::ProviderCapabilities,
    infrastructure::{provider_registry::ProviderRegistry, repository::MemoryRepository},
    model::*,
};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex, atomic::AtomicU64},
};

struct Adapter {
    provider: &'static str,
    search: bool,
    calls: Arc<Mutex<Vec<(String, u32)>>>,
    sent: AtomicU64,
}
impl MailGateway for Adapter {
    fn capabilities(&self, _: &Account) -> ProviderCapabilities {
        ProviderCapabilities {
            provider_id: self.provider.into(),
            sync: true,
            remote_actions: true,
            remote_search: self.search,
            send: true,
            ..Default::default()
        }
    }
    fn sync(&self, a: &Account) -> AppResult<SyncBatch> {
        self.calls
            .lock()
            .unwrap()
            .push((self.provider.into(), a.id));
        Ok(SyncBatch {
            emails: vec![Email {
                id: u64::from(a.id) * 100,
                account_id: a.id,
                subject: format!("{} message", self.provider),
                ..Default::default()
            }],
            archive_folder: None,
            trash_folder: None,
        })
    }
    fn apply(&self, a: &Account, _: u32, _: RemoteAction) -> AppResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push((format!("{}:apply", self.provider), a.id));
        Ok(())
    }
    fn send(&self, a: &Account, _: &str, _: OutgoingMessage) -> AppResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push((format!("{}:send", self.provider), a.id));
        self.sent.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn check_login(&self, a: &Account) -> AppResult<()> {
        self.calls
            .lock()
            .unwrap()
            .push((format!("{}:login", self.provider), a.id));
        Ok(())
    }
}
struct Secrets(HashMap<String, String>);
impl SecretStore for Secrets {
    fn get(&self, k: &str) -> AppResult<String> {
        Ok(self.0.get(k).cloned().unwrap_or_default())
    }
    fn set(&self, _: &str, _: &str) -> AppResult<()> {
        Ok(())
    }
}
struct ClockNow;
impl Clock for ClockNow {
    fn now(&self) -> i64 {
        2_000_000_000
    }
}
struct Files;
impl AttachmentGateway for Files {
    fn total_size(&self, _: &[String]) -> AppResult<u64> {
        Ok(0)
    }
    fn read_text(&self, _: &Attachment) -> AppResult<String> {
        Ok(String::new())
    }
}

#[test]
fn one_context_routes_accounts_to_distinct_adapters_and_capabilities() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    for (provider, search) in [("alpha", true), ("beta", false)] {
        registry
            .register(
                provider,
                Arc::new(Adapter {
                    provider,
                    search,
                    calls: calls.clone(),
                    sent: AtomicU64::new(0),
                }),
            )
            .unwrap();
    }
    let store = Store {
        accounts: vec![
            Account {
                id: 1,
                provider_id: "alpha".into(),
                email: "same@example.test".into(),
                ..Default::default()
            },
            Account {
                id: 2,
                provider_id: "beta".into(),
                email: "same@example.test".into(),
                ..Default::default()
            },
        ],
        ..Default::default()
    };
    let context = Arc::new(ServiceContext { compute: std::sync::Arc::new(qamille::infrastructure::fake_compute::FakeCompute::default()),
        events: Arc::new(NullSink),
        repository: Arc::new(MemoryRepository::new(store)),
        mail: Arc::new(registry),
        secrets: Arc::new(Secrets(HashMap::from([
            (provider_account_key("alpha", 1), "alpha-secret".into()),
            (provider_account_key("beta", 2), "beta-secret".into()),
        ]))),
        clock: Arc::new(ClockNow),
        attachments: Arc::new(Files),
    });
    let current = context.read().unwrap();
    let alpha = context.account(&current, 1).unwrap();
    let beta = context.account(&current, 2).unwrap();
    assert_eq!(alpha.password, "alpha-secret");
    assert_eq!(beta.password, "beta-secret");
    assert!(context.mail.capabilities(&alpha).remote_search);
    assert!(!context.mail.capabilities(&beta).remote_search);
    let result = MailService::new(context.clone()).sync().unwrap();
    assert!(result.errors.is_empty());
    let current = context.read().unwrap();
    assert!(
        current
            .emails
            .iter()
            .any(|e| e.account_id == 1 && e.subject == "alpha message")
    );
    assert!(
        current
            .emails
            .iter()
            .any(|e| e.account_id == 2 && e.subject == "beta message")
    );
    context.mail.check_login(&alpha).unwrap();
    context
        .mail
        .apply(&beta, 10, RemoteAction::Seen(true))
        .unwrap();
    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[
            ("alpha".into(), 1),
            ("beta".into(), 2),
            ("alpha:login".into(), 1),
            ("beta:apply".into(), 2)
        ]
    );
    assert_eq!(
        context.mail.list_remote_drafts(&beta).unwrap_err().code,
        "unsupported_capability"
    );
}

#[test]
fn unknown_provider_never_uses_a_registered_fallback() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mut registry = ProviderRegistry::new();
    registry
        .register(
            "gmail",
            Arc::new(Adapter {
                provider: "gmail",
                search: true,
                calls: calls.clone(),
                sent: AtomicU64::new(0),
            }),
        )
        .unwrap();
    let account = Account {
        provider_id: "not-installed".into(),
        kind: AccountKind::Gmail,
        ..Default::default()
    };
    let c = registry.capabilities(&account);
    assert!(!c.send && !c.sync && !c.remote_search && !c.remote_actions);
    assert_eq!(
        registry.sync(&account).err().unwrap().code,
        "unsupported_provider"
    );
    assert!(calls.lock().unwrap().is_empty());
}

#[test]
fn provider_identity_survives_serialization_and_legacy_defaults() {
    let legacy = Account {
        id: 1,
        kind: AccountKind::Gmail,
        ..Default::default()
    };
    assert_eq!(legacy.resolved_provider_id(), "gmail");
    assert_eq!(Account::default().resolved_provider_id(), "sample");
    let future = Account {
        provider_id: "beta".into(),
        kind: AccountKind::Gmail,
        ..Default::default()
    };
    let roundtrip: Account = serde_json::from_slice(&serde_json::to_vec(&future).unwrap()).unwrap();
    assert_eq!(roundtrip.resolved_provider_id(), "beta");
    let mut value = serde_json::to_value(legacy).unwrap();
    value.as_object_mut().unwrap().remove("provider_id");
    let old: Account = serde_json::from_value(value).unwrap();
    assert_eq!(old.resolved_provider_id(), "gmail");
    assert_eq!(account_key("A@EXAMPLE.TEST"), "gmail:a@example.test");
    assert_ne!(
        provider_account_key("alpha", 1),
        provider_account_key("beta", 1)
    );
}
