//! The link of this computer to a Qamille server (spec A1, A2, A7): the session kept in the
//! vault, what `GET /v1/info` says and the devices of the account. The device key goes only to
//! the session's `url`, and never into `Debug`, logs or events.
use serde::{Deserialize, Serialize};

/// The single vault value `server-session`, written with one `set`.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerSession {
    pub url: String,
    pub user_id: String,
    pub username: String,
    pub device_id: String,
    pub device_key: String,
}

impl std::fmt::Debug for ServerSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServerSession")
            .field("url", &self.url)
            .field("user_id", &self.user_id)
            .field("username", &self.username)
            .field("device_id", &self.device_id)
            .field("device_key", &"<hidden>")
            .finish()
    }
}

/// `GET /v1/info`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: String,
    pub api: u32,
    #[serde(default)]
    pub signup: bool,
    #[serde(default)]
    pub password_reset: bool,
    #[serde(default)]
    pub edition: String,
    /// Browser sign-ins the server offers (`google`); empty on servers without them.
    #[serde(default)]
    pub login_providers: Vec<String>,
    /// A Google sign-in may create an account on this server.
    #[serde(default)]
    pub google_signup: bool,
    /// Subscriptions, when the server sells them.
    #[serde(default)]
    pub billing: Option<BillingInfo>,
    /// The assistant of the server (gateway LLM), when it has one.
    #[serde(default)]
    pub llm: Option<LlmInfo>,
    /// Mailbox synchronization (wave 7), when the server has its key.
    #[serde(default)]
    pub sync: Option<SyncInfo>,
    /// The documents a new account accepts (wave 9): absent on servers that do not ask for them.
    #[serde(default)]
    pub legal: Option<LegalInfo>,
}

/// Where the Terms and the privacy notice of the server are, and the version of the terms a new
/// account accepts. The app only links the addresses and sends back the version it read here:
/// the texts are the owner's, never the app's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegalInfo {
    #[serde(default)]
    pub terms_url: Option<String>,
    #[serde(default)]
    pub privacy_url: Option<String>,
    #[serde(default)]
    pub terms_version: Option<String>,
}

/// What `GET /v1/info` declares of the mailbox synchronization.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SyncInfo {
    pub available: bool,
}

/// What `GET /v1/info` declares of the assistant: never a key or an address.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LlmInfo {
    pub model: String,
    #[serde(default)]
    pub models: Vec<String>,
    /// The server forwards to a service outside its own network.
    pub external: bool,
    pub label: String,
    #[serde(default)]
    pub context_tokens: Option<u32>,
}

/// What a server that sells subscriptions declares in `GET /v1/info`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillingInfo {
    pub provider: String,
    /// A fake payment processor: no money moves.
    #[serde(default)]
    pub test_mode: bool,
    pub currency: String,
    #[serde(default)]
    pub trial_days: u32,
    #[serde(default)]
    pub plans: Vec<PlanInfo>,
}

/// One plan on sale.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanInfo {
    pub id: String,
    pub name: String,
    /// `month` or `year`.
    pub interval: String,
    pub price_cents: u32,
    pub currency: String,
    /// What the plan includes each month, when the server limits compute (wave 9).
    #[serde(default)]
    pub quotas: Option<QuotaInfo>,
}

/// The monthly limits of a plan: minutes of audio, tokens of semantic search, tokens of the assistant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaInfo {
    pub audio_minutes: u64,
    pub embedded_tokens: u64,
    pub llm_tokens: u64,
}

/// One meter of the month: what the account used and the limit of its plan.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meter {
    pub used: u64,
    pub limit: u64,
}

/// What `GET /v1/account` says of the month: calendar month in UTC, back to zero on `renews_at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageInfo {
    pub period_start: String,
    pub renews_at: String,
    pub audio_minutes: Meter,
    pub embedded_tokens: Meter,
    pub llm_tokens: Meter,
}

/// The subscription of the account, as `GET /v1/account` gives it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Subscription {
    /// `none`, `trialing`, `active`, `past_due`, `canceled` or `expired`.
    pub status: String,
    /// Remote compute is open for this account.
    pub entitled: bool,
    #[serde(default)]
    pub plan_id: Option<String>,
    #[serde(default)]
    pub plan_name: Option<String>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub current_period_end: Option<String>,
    #[serde(default)]
    pub cancel_at_period_end: bool,
    /// The processor has a management page for it.
    #[serde(default)]
    pub manageable: bool,
    /// The consumption of the month, on servers that limit compute (wave 9).
    #[serde(default)]
    pub usage: Option<UsageInfo>,
}

/// `GET /v1/account`: who is connected and, where the server sells them, the subscription.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountInfo {
    pub user_id: String,
    pub username: String,
    #[serde(default)]
    pub email: Option<String>,
    /// Accounts born from a Google sign-in may have none.
    #[serde(default = "yes")]
    pub has_password: bool,
    #[serde(default)]
    pub identities: Vec<String>,
    #[serde(default)]
    pub billing_enabled: bool,
    #[serde(default)]
    pub subscription: Option<Subscription>,
}

fn yes() -> bool {
    true
}

/// A page of the payment processor opened for this account (checkout or management).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BillingPage {
    pub id: String,
    pub url: String,
    pub expires_at: String,
}

/// Where a checkout stands: `open`, `completed`, `failed`, `canceled` or `expired`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckoutStatus {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub plan_id: Option<String>,
}

/// The API version this app speaks.
pub const SERVER_API: u32 = 1;

/// One device of the account, as `GET /v1/account/devices` lists it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub created_at: String,
    #[serde(default)]
    pub last_seen_at: Option<String>,
    #[serde(default)]
    pub current: bool,
}

#[cfg(test)]
mod tests {
    #[test]
    fn server_info_reads_the_sync_block() {
        let with: ServerInfo = serde_json::from_str(r#"{"version":"1","api":1,"sync":{"available":true}}"#).unwrap();
        assert_eq!(with.sync, Some(SyncInfo { available: true }));
        let without: ServerInfo = serde_json::from_str(r#"{"version":"1","api":1}"#).unwrap();
        assert!(without.sync.is_none());
    }

    #[test]
    fn server_info_reads_the_llm_block() {
        let with = r#"{"version":"1","api":1,"llm":{"model":"llm-medio","models":["llm-medio"],"external":false,"label":"Server","context_tokens":8192}}"#;
        let info: ServerInfo = serde_json::from_str(with).unwrap();
        assert_eq!(info.llm.unwrap().context_tokens, Some(8192));
        let without: ServerInfo = serde_json::from_str(r#"{"version":"1","api":1}"#).unwrap();
        assert!(without.llm.is_none());
    }

    use super::*;

    #[test]
    fn a_subscription_without_usage_still_reads() {
        let sub: Subscription = serde_json::from_str(r#"{"status":"active","entitled":true,"plan_id":"pro","manageable":true}"#).unwrap();
        assert!(sub.usage.is_none());
        let info: PlanInfo = serde_json::from_str(r#"{"id":"pro","name":"Pro","interval":"month","price_cents":900,"currency":"EUR"}"#).unwrap();
        assert!(info.quotas.is_none());
    }

    #[test]
    fn usage_and_plan_quotas_read() {
        let sub: Subscription = serde_json::from_str(
            r#"{"status":"active","entitled":true,"usage":{"period_start":"2026-11-01","renews_at":"2026-12-01T00:00:00.000Z",
               "audio_minutes":{"used":12,"limit":600},"embedded_tokens":{"used":1000,"limit":2000000},"llm_tokens":{"used":5,"limit":100}}}"#,
        )
        .unwrap();
        let usage = sub.usage.unwrap();
        assert_eq!(usage.renews_at, "2026-12-01T00:00:00.000Z");
        assert_eq!(usage.audio_minutes, Meter { used: 12, limit: 600 });
        assert_eq!(usage.llm_tokens.limit, 100);
        let plan: PlanInfo = serde_json::from_str(
            r#"{"id":"pro","name":"Pro","interval":"month","price_cents":900,"currency":"EUR","quotas":{"audio_minutes":600,"embedded_tokens":2000000,"llm_tokens":100}}"#,
        )
        .unwrap();
        assert_eq!(plan.quotas, Some(QuotaInfo { audio_minutes: 600, embedded_tokens: 2_000_000, llm_tokens: 100 }));
    }

    #[test]
    fn info_reads_legal() {
        let with: ServerInfo = serde_json::from_str(
            r#"{"version":"1","api":1,"legal":{"terms_url":"https://q.example/terms","privacy_url":"https://q.example/privacy","terms_version":"2026-12-01"}}"#,
        )
        .unwrap();
        let legal = with.legal.unwrap();
        assert_eq!(legal.terms_version.as_deref(), Some("2026-12-01"));
        assert_eq!(legal.privacy_url.as_deref(), Some("https://q.example/privacy"));
        let without: ServerInfo = serde_json::from_str(r#"{"version":"1","api":1}"#).unwrap();
        assert!(without.legal.is_none());
    }

    #[test]
    fn older_servers_still_read() {
        let info: ServerInfo = serde_json::from_str(r#"{"version":"0.1.0","api":1,"signup":true,"password_reset":true,"edition":"professional"}"#).unwrap();
        assert!(info.login_providers.is_empty() && info.billing.is_none() && !info.google_signup);
        let info: ServerInfo = serde_json::from_str(
            r#"{"version":"0.2.0","api":1,"login_providers":["google"],"billing":{"provider":"fake","test_mode":true,"currency":"EUR","trial_days":14,
               "plans":[{"id":"pro-monthly","name":"Pro","interval":"month","price_cents":900,"currency":"EUR"}]}}"#,
        )
        .unwrap();
        assert_eq!(info.login_providers, ["google"]);
        assert_eq!(info.billing.unwrap().plans[0].price_cents, 900);
    }

    #[test]
    fn the_debug_form_of_a_session_never_shows_the_device_key() {
        let session = ServerSession {
            url: "https://calcolo.ufficio.it".into(),
            user_id: "u1".into(),
            username: "anna".into(),
            device_id: "d1".into(),
            device_key: "spk_dev_0123456789abcdef".into(),
        };
        let text = format!("{session:?}");
        assert!(!text.contains("spk_dev_") && text.contains("anna"), "{text}");
    }
}
