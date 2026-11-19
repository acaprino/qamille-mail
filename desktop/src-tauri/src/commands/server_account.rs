//! The link to a Qamille server (access wave 2): commands for Settings → Compute and the first
//! start of the Professional build. Passwords arrive here, go to the server and are dropped; the
//! device key never leaves the backend (`server_status` has no field for it).
use crate::runtime::{DesktopState, blocking};
use qamille::{
    application::{
        AppResult,
        AppError,
        server_account::{ConnectOutcome, Credentials, LlmStatus, Probe, ServerStatus, SignOut, SignUp},
    },
    domain::{
        model::ComputeMode,
        server_account::{AccountInfo, BillingPage, CheckoutStatus, DeviceInfo},
    },
    edition::Edition,
};
use tauri::State;

/// The name a new device gets unless the person writes another.
fn default_device_name(edition: &Edition) -> String {
    let system = if cfg!(windows) { "Windows" } else if cfg!(target_os = "macos") { "macOS" } else { "Linux" };
    match edition {
        Edition::SelfHosted => format!("Qamille su {system}"),
        Edition::Professional { .. } => format!("Qamille Professional su {system}"),
    }
}
fn device_name(state: &DesktopState, name: Option<String>) -> String {
    name.map(|n| n.trim().chars().take(80).collect::<String>())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| default_device_name(state.server.edition()))
}

#[derive(serde::Serialize)]
pub struct AppEdition {
    #[serde(flatten)]
    edition: Edition,
    default_device_name: String,
}

#[tauri::command]
pub fn app_edition(state: State<'_, DesktopState>) -> AppEdition {
    let edition = state.server.edition().clone();
    AppEdition { default_device_name: default_device_name(&edition), edition }
}
#[tauri::command]
pub async fn server_status(state: State<'_, DesktopState>) -> AppResult<ServerStatus> {
    let server = state.server.clone();
    blocking(move || server.status()).await
}
#[tauri::command]
pub async fn server_probe(state: State<'_, DesktopState>, url: String) -> AppResult<Probe> {
    let server = state.server.clone();
    blocking(move || server.probe(&url)).await
}
#[tauri::command]
pub async fn server_record_consent(state: State<'_, DesktopState>, url: String) -> AppResult<()> {
    let server = state.server.clone();
    // The consent to the address, and with it the one to the provider the server declares for its
    // assistant (INV-34). A server without an assistant is not an error.
    let sync = state.sync.clone();
    blocking(move || {
        server.record_consent(&url)?;
        let _ = server.record_llm_consent(&url);
        // The consent to keep the mailboxes on the server counts only if it declares it (S6).
        let _ = sync.record_consent(&url);
        Ok(())
    })
    .await
}
/// The person confirmed the provider the server declares now (after `409 llm_consent_required`).
#[tauri::command]
pub async fn server_record_llm_consent(state: State<'_, DesktopState>, url: String) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || server.record_llm_consent(&url)).await
}
#[tauri::command]
pub async fn server_llm_status(state: State<'_, DesktopState>) -> AppResult<LlmStatus> {
    let server = state.server.clone();
    blocking(move || server.llm_status()).await
}
#[tauri::command]
pub async fn server_connect(state: State<'_, DesktopState>, url: String, identifier: String, password: String, device_name: Option<String>) -> AppResult<ConnectOutcome> {
    let (server, name) = (state.server.clone(), device_name_of(&state, device_name));
    blocking(move || server.connect(&url, &Credentials { identifier: &identifier, password: &password, device_name: &name })).await
}
#[tauri::command]
pub async fn server_change_temporary_password(state: State<'_, DesktopState>, url: String, identifier: String, temporary_password: String, new_password: String, device_name: Option<String>, change_server: Option<bool>) -> AppResult<ConnectOutcome> {
    let (server, name) = (state.server.clone(), device_name_of(&state, device_name));
    blocking(move || {
        server.change_temporary_password(&url, &Credentials { identifier: &identifier, password: &temporary_password, device_name: &name }, &new_password, change_server.unwrap_or(false))
    })
    .await
}
fn device_name_of(state: &DesktopState, name: Option<String>) -> String {
    device_name(state, name)
}
#[tauri::command]
pub async fn server_disable_device_with_password(state: State<'_, DesktopState>, url: String, identifier: String, password: String, device_id: String) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || server.disable_device_with_password(&url, &identifier, &password, &device_id)).await
}
#[tauri::command]
pub async fn server_devices(state: State<'_, DesktopState>) -> AppResult<Vec<DeviceInfo>> {
    let server = state.server.clone();
    blocking(move || server.devices()).await
}
#[tauri::command]
pub async fn server_revoke_device(state: State<'_, DesktopState>, device_id: String) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || server.revoke_device(&device_id)).await
}
#[tauri::command]
pub async fn server_change_password(state: State<'_, DesktopState>, current_password: String, new_password: String, sign_out_other_devices: bool) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || server.change_password(&current_password, &new_password, sign_out_other_devices)).await
}
#[tauri::command]
pub async fn server_change_server(state: State<'_, DesktopState>, url: String, identifier: String, password: String, device_name: Option<String>) -> AppResult<ConnectOutcome> {
    let (server, name) = (state.server.clone(), device_name_of(&state, device_name));
    blocking(move || server.change_server(&url, &Credentials { identifier: &identifier, password: &password, device_name: &name })).await
}
#[tauri::command]
pub async fn server_sign_out(state: State<'_, DesktopState>) -> AppResult<SignOut> {
    let server = state.server.clone();
    blocking(move || server.sign_out()).await
}
#[tauri::command]
pub async fn server_delete_account(state: State<'_, DesktopState>, password: String) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || server.delete_account(&password)).await
}
#[tauri::command]
pub async fn server_startup_check(state: State<'_, DesktopState>) -> AppResult<bool> {
    let server = state.server.clone();
    blocking(move || server.startup_check()).await
}
#[tauri::command]
pub async fn server_finish_onboarding(state: State<'_, DesktopState>) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || server.finish_onboarding()).await
}
#[tauri::command]
pub async fn compute_set_ca_pem(state: State<'_, DesktopState>, pem: Option<String>) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || server.set_ca_pem(pem)).await
}
#[tauri::command]
pub async fn server_signup_start(state: State<'_, DesktopState>, url: String, email: String) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || server.signup_start(&url, &email)).await
}
#[tauri::command]
pub async fn server_signup_confirm(state: State<'_, DesktopState>, url: String, email: String, code: String, password: String, username: Option<String>, device_name: Option<String>, accepted_terms: Option<String>) -> AppResult<ServerStatus> {
    let (server, name) = (state.server.clone(), device_name_of(&state, device_name));
    blocking(move || {
        server.signup_confirm(&url, &SignUp { email: &email, code: &code, password: &password, username: username.as_deref(), device_name: &name, accepted_terms: accepted_terms.as_deref() })
    })
    .await
}
#[tauri::command]
pub async fn server_reset_start(state: State<'_, DesktopState>, url: String, email: String) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || server.reset_start(&url, &email)).await
}
#[tauri::command]
pub async fn server_reset_confirm(state: State<'_, DesktopState>, url: String, email: String, code: String, new_password: String) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || server.reset_confirm(&url, &email, &code, &new_password)).await
}
/// «Continue with Google»: the browser signs in on the server (up to 5 minutes of waiting).
#[tauri::command]
pub async fn server_connect_browser(window: tauri::WebviewWindow, state: State<'_, DesktopState>, url: String, provider: String, device_name: Option<String>, change_server: Option<bool>, accepted_terms: Option<String>) -> AppResult<ConnectOutcome> {
    let (server, name) = (state.server.clone(), device_name_of(&state, device_name));
    let result = blocking(move || server.connect_with_browser(&url, &provider, &name, change_server.unwrap_or(false), accepted_terms.as_deref())).await;
    super::accounts::back_from_browser(&window);
    result
}
#[tauri::command]
pub fn server_cancel_browser_sign_in(state: State<'_, DesktopState>) {
    state.server.cancel_browser_sign_in();
}
/// At the device limit of a browser sign-in: disables one device and finishes the sign-in.
#[tauri::command]
pub async fn server_browser_disable_device(state: State<'_, DesktopState>, device_id: String) -> AppResult<ConnectOutcome> {
    let server = state.server.clone();
    blocking(move || server.browser_disable_device(&device_id)).await
}
#[tauri::command]
pub async fn server_account_info(state: State<'_, DesktopState>) -> AppResult<AccountInfo> {
    let server = state.server.clone();
    blocking(move || server.account_info()).await
}
fn open_in_browser(url: &str) -> AppResult<()> {
    open::that(url).map_err(|e| AppError::external(e.to_string()))
}
/// Opens the payment page of a plan in the browser; the interface follows the session by id.
#[tauri::command]
pub async fn server_start_checkout(state: State<'_, DesktopState>, plan_id: String) -> AppResult<BillingPage> {
    let server = state.server.clone();
    blocking(move || {
        let page = server.start_checkout(&plan_id)?;
        open_in_browser(&page.url)?;
        Ok(page)
    })
    .await
}
#[tauri::command]
pub async fn server_checkout_status(state: State<'_, DesktopState>, id: String) -> AppResult<CheckoutStatus> {
    let server = state.server.clone();
    blocking(move || server.checkout_status(&id)).await
}
/// Opens the processor's page to manage the subscription.
#[tauri::command]
pub async fn server_open_billing_portal(state: State<'_, DesktopState>) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || open_in_browser(&server.billing_portal()?.url)).await
}
/// «Delete account» for an account without password: confirmed by signing in again.
#[tauri::command]
pub async fn server_delete_account_browser(window: tauri::WebviewWindow, state: State<'_, DesktopState>, provider: String) -> AppResult<()> {
    let server = state.server.clone();
    let result = blocking(move || server.delete_account_with_browser(&provider)).await;
    super::accounts::back_from_browser(&window);
    result
}
#[tauri::command]
pub fn compute_forget_pending_ca(state: State<'_, DesktopState>) {
    state.server.forget_pending_ca();
}
#[tauri::command]
pub async fn compute_set_mode(state: State<'_, DesktopState>, mode: ComputeMode) -> AppResult<()> {
    let server = state.server.clone();
    blocking(move || server.set_mode(mode)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_names_say_the_edition() {
        assert!(default_device_name(&Edition::SelfHosted).starts_with("Qamille su "));
        assert!(default_device_name(&Edition::Professional { server: "https://x.example".into() }).starts_with("Qamille Professional su "));
    }
    #[test]
    fn the_status_has_no_field_for_the_device_key() {
        let status = ServerStatus {
            edition: Edition::SelfHosted,
            connected: true,
            url: Some("https://x.example".into()),
            username: Some("mario".into()),
            device_id: Some("d1".into()),
            compute_mode: ComputeMode::Server,
            consented_urls: vec![],
        };
        let json = serde_json::to_value(&status).unwrap();
        let keys: Vec<_> = json.as_object().unwrap().keys().cloned().collect();
        assert!(!keys.iter().any(|k| k.contains("key")), "{keys:?}");
    }
}
