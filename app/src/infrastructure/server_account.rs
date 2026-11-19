//! The account routes of a Qamille server over HTTP (spec A2) with `ureq`. Error bodies are
//! `{"error":{"code","message","retry_after_s"}}`; the request bodies (with passwords) never
//! appear in an error or a log.
use super::tls;
use crate::application::{
    AppError, AppResult,
    server_account::{BrowserTicket, Credentials, LoginGrant, LoginOutcome, RemoteEndpoint, ServerAccount, ServerAddress, SignUp},
};
use crate::domain::server_account::{AccountInfo, BillingPage, CheckoutStatus, DeviceInfo, ServerInfo};
use std::sync::atomic::AtomicBool;
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::Duration;

const INFO_TIMEOUT: Duration = Duration::from_secs(5);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
/// Larger answers are not a Qamille account route.
const MAX_BODY: u64 = 1 << 20;

pub struct HttpServerAccount;

#[derive(Deserialize)]
struct Grant {
    user_id: String,
    username: String,
    device_id: String,
    device_key: String,
}

fn agent(timeout: Duration, ca_pem: Option<&str>) -> AppResult<ureq::Agent> {
    Ok(ureq::Agent::config_builder()
        .timeout_global(Some(timeout))
        // A redirect would carry the request (passwords, the device key) to another address.
        .max_redirects(0)
        .http_status_as_error(false)
        .tls_config(tls::config(ca_pem)?)
        .build()
        .into())
}

fn unreachable() -> AppError {
    AppError::new("unreachable", "The Qamille server does not answer")
}

/// The server answered but its certificate (or the TLS handshake) was refused: not the same as a
/// server that does not answer, and the fix is different (the organization's certificate authority).
fn untrusted() -> AppError {
    AppError::new("tls_untrusted", "The server's certificate is not trusted")
}

/// Does this transport error come from the server's certificate rather than the network? The
/// platform verifier and rustls report a refused certificate as an I/O error that names it
/// (`invalid peer certificate: UnknownIssuer`); an `https` address that points at a plain-text port
/// ("corrupt message") is still just a server that does not answer as expected.
fn is_tls_failure(error: &ureq::Error) -> bool {
    if matches!(error, ureq::Error::Tls(_)) {
        return true;
    }
    let text = error.to_string().to_lowercase();
    ["certificate", "unknownissuer", "unknown issuer"].iter().any(|word| text.contains(word))
}

/// The application error for a transport failure.
fn transport_error(error: &ureq::Error) -> AppError {
    if is_tls_failure(error) { untrusted() } else { unreachable() }
}

/// The status and JSON body of an answer; a body that is not JSON reads as null.
fn read(response: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> AppResult<(u16, Value)> {
    let mut response = response.map_err(|error| transport_error(&error))?;
    let status = response.status().as_u16();
    let text = response.body_mut().with_config().limit(MAX_BODY).read_to_string().unwrap_or_default();
    Ok((status, serde_json::from_str(&text).unwrap_or(Value::Null)))
}

/// The application error for a server error body, with fixed English texts the interface translates.
fn server_error(status: u16, body: &Value) -> AppError {
    let error = &body["error"];
    let retry = error["retry_after_s"].as_u64();
    match error["code"].as_str().unwrap_or_default() {
        "invalid_credentials" => AppError::new("invalid_credentials", "Wrong username, email or password"),
        "rate_limited" => AppError::new(
            "rate_limited",
            match retry {
                Some(seconds) => format!("Too many attempts. Try again in {seconds} seconds"),
                None => "Too many attempts. Try again later".into(),
            },
        ),
        "server_busy" => AppError::new("server_busy", "The server is busy. Try again in a moment"),
        "tls_required" => AppError::new("tls_required", "The server accepts passwords only over HTTPS"),
        "unauthorized" => AppError::new("unauthorized", "This device is no longer connected to the server"),
        "password_change_required" => AppError::new("password_change_required", "Change the temporary password to continue"),
        "device_limit" => AppError::new("device_limit", "The account already has its maximum of devices"),
        "payload_too_large" => AppError::new("payload_too_large", "The request is too large for the server"),
        "invalid_request" => AppError::new("invalid_request", "The server refused the request"),
        "not_found" => AppError::new("not_found", "The server does not know this device or account"),
        "invalid_code" => AppError::new("invalid_code", "The code is wrong or expired. Ask for a new one"),
        "invalid_password" | "password_too_short" | "password_too_long" => AppError::new("invalid_password", "The password must be 10 to 256 characters long"),
        "password_too_common" => AppError::new("password_too_common", "This password is among the most used ones. Choose another one"),
        "username_taken" => AppError::new("username_taken", "This username is taken. Choose another one"),
        "subscription_required" => AppError::new("subscription_required", "An active Qamille subscription is required for compute on the server"),
        "terms_required" => AppError::new("terms_required", "The Terms and the privacy notice must be accepted to create the account"),
        "already_subscribed" => AppError::new("already_subscribed", "This account already has an active subscription"),
        "no_subscription" => AppError::new("no_subscription", "This account has no subscription to manage"),
        _ if status == 401 => AppError::new("unauthorized", "This device is no longer connected to the server"),
        _ => AppError::new("server_error", "The Qamille server answered with an error"),
    }
}

fn devices_of(body: &Value) -> Vec<DeviceInfo> {
    let list = if body["devices"].is_array() { &body["devices"] } else { &body["error"]["devices"] };
    serde_json::from_value(list.clone()).unwrap_or_default()
}

/// The outcome of the two sign-in routes.
fn login_outcome(status: u16, body: Value) -> AppResult<LoginOutcome> {
    match (status, body["error"]["code"].as_str()) {
        (200 | 201, _) => {
            let grant: Grant = serde_json::from_value(body)
                .map_err(|_| AppError::new("server_error", "The Qamille server answered with an error"))?;
            Ok(LoginOutcome::Signed(LoginGrant {
                user_id: grant.user_id,
                username: grant.username,
                device_id: grant.device_id,
                device_key: grant.device_key,
            }))
        }
        (_, Some("password_change_required")) => Ok(LoginOutcome::PasswordChangeRequired),
        (_, Some("device_limit")) => Ok(LoginOutcome::DeviceLimit(devices_of(&body))),
        _ => Err(server_error(status, &body)),
    }
}

fn ok(status: u16, body: &Value) -> AppResult<()> {
    if (200..300).contains(&status) { Ok(()) } else { Err(server_error(status, body)) }
}

fn bearer(endpoint: &RemoteEndpoint) -> String {
    format!("Bearer {}", endpoint.device_key)
}

impl ServerAccount for HttpServerAccount {
    fn info(&self, address: &ServerAddress) -> AppResult<ServerInfo> {
        let agent = agent(INFO_TIMEOUT, address.ca_pem.as_deref())?;
        let (status, body) = read(agent.get(format!("{}/v1/info", address.url)).call())?;
        if status != 200 {
            return Err(AppError::new("not_qamille", "This address is not a Qamille server"));
        }
        serde_json::from_value::<ServerInfo>(body)
            .ok()
            .filter(|info| !info.version.is_empty())
            .ok_or_else(|| AppError::new("not_qamille", "This address is not a Qamille server"))
    }
    fn login(&self, address: &ServerAddress, credentials: &Credentials<'_>) -> AppResult<LoginOutcome> {
        let agent = agent(REQUEST_TIMEOUT, address.ca_pem.as_deref())?;
        let body = json!({ "identifier": credentials.identifier, "password": credentials.password, "device_name": credentials.device_name });
        let (status, body) = read(agent.post(format!("{}/v1/login", address.url)).send_json(&body))?;
        login_outcome(status, body)
    }
    fn change_temporary_password(&self, address: &ServerAddress, credentials: &Credentials<'_>, new_password: &str) -> AppResult<LoginOutcome> {
        let agent = agent(REQUEST_TIMEOUT, address.ca_pem.as_deref())?;
        let body = json!({
            "identifier": credentials.identifier,
            "temporary_password": credentials.password,
            "new_password": new_password,
            "device_name": credentials.device_name,
        });
        let (status, body) = read(agent.post(format!("{}/v1/login/password", address.url)).send_json(&body))?;
        login_outcome(status, body)
    }
    fn disable_device_with_password(&self, address: &ServerAddress, identifier: &str, password: &str, device_id: &str) -> AppResult<()> {
        let agent = agent(REQUEST_TIMEOUT, address.ca_pem.as_deref())?;
        let body = json!({ "identifier": identifier, "password": password, "device_id": device_id });
        let (status, body) = read(agent.post(format!("{}/v1/login/disable-device", address.url)).send_json(&body))?;
        ok(status, &body)
    }
    fn devices(&self, endpoint: &RemoteEndpoint) -> AppResult<Vec<DeviceInfo>> {
        let agent = agent(endpoint.timeout, endpoint.ca_pem.as_deref())?;
        let (status, body) = read(agent.get(format!("{}/v1/account/devices", endpoint.url)).header("Authorization", bearer(endpoint)).call())?;
        ok(status, &body)?;
        Ok(devices_of(&body))
    }
    fn revoke_device(&self, endpoint: &RemoteEndpoint, device_id: &str) -> AppResult<()> {
        if device_id.is_empty() || !device_id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(AppError::validation("Unknown device"));
        }
        let agent = agent(endpoint.timeout, endpoint.ca_pem.as_deref())?;
        let (status, body) = read(agent.delete(format!("{}/v1/account/devices/{device_id}", endpoint.url)).header("Authorization", bearer(endpoint)).call())?;
        ok(status, &body)
    }
    fn change_password(&self, endpoint: &RemoteEndpoint, current: &str, new: &str, sign_out_other_devices: bool) -> AppResult<()> {
        let agent = agent(endpoint.timeout, endpoint.ca_pem.as_deref())?;
        let body = json!({ "current_password": current, "new_password": new, "sign_out_other_devices": sign_out_other_devices });
        let (status, body) = read(agent.put(format!("{}/v1/account/password", endpoint.url)).header("Authorization", bearer(endpoint)).send_json(&body))?;
        ok(status, &body)
    }
    fn sign_out(&self, endpoint: &RemoteEndpoint) -> AppResult<()> {
        let agent = agent(endpoint.timeout, endpoint.ca_pem.as_deref())?;
        let (status, body) = read(agent.delete(format!("{}/v1/devices/self", endpoint.url)).header("Authorization", bearer(endpoint)).call())?;
        ok(status, &body)
    }
    fn delete_account(&self, endpoint: &RemoteEndpoint, password: &str) -> AppResult<()> {
        let agent = agent(endpoint.timeout, endpoint.ca_pem.as_deref())?;
        let body = json!({ "password": password });
        let (status, body) = read(
            agent.delete(format!("{}/v1/account", endpoint.url)).header("Authorization", bearer(endpoint)).force_send_body().send_json(&body),
        )?;
        ok(status, &body)
    }
    fn signup_start(&self, address: &ServerAddress, email: &str) -> AppResult<()> {
        let agent = agent(REQUEST_TIMEOUT, address.ca_pem.as_deref())?;
        let (status, body) = read(agent.post(format!("{}/v1/signup", address.url)).send_json(json!({ "email": email })))?;
        if status == 404 {
            return Err(AppError::new("not_available", "This server does not offer sign-up"));
        }
        ok(status, &body)
    }
    fn signup_confirm(&self, address: &ServerAddress, signup: &SignUp<'_>) -> AppResult<LoginGrant> {
        let agent = agent(REQUEST_TIMEOUT, address.ca_pem.as_deref())?;
        let mut request = json!({ "email": signup.email, "code": signup.code, "password": signup.password, "device_name": signup.device_name });
        // Only the version the server declared in `/v1/info`, and only when the box was ticked.
        if let Some(version) = signup.accepted_terms.map(str::trim).filter(|v| !v.is_empty()) {
            request["accepted_terms"] = json!(version);
        }
        if let Some(username) = signup.username.map(str::trim).filter(|u| !u.is_empty()) {
            request["username"] = json!(username);
        }
        let (status, body) = read(agent.post(format!("{}/v1/signup/confirm", address.url)).send_json(&request))?;
        match login_outcome(status, body)? {
            LoginOutcome::Signed(grant) => Ok(grant),
            _ => Err(AppError::new("server_error", "The Qamille server answered with an error")),
        }
    }
    fn reset_start(&self, address: &ServerAddress, email: &str) -> AppResult<()> {
        let agent = agent(REQUEST_TIMEOUT, address.ca_pem.as_deref())?;
        let (status, body) = read(agent.post(format!("{}/v1/password-reset", address.url)).send_json(json!({ "email": email })))?;
        if status == 404 {
            return Err(AppError::new("not_available", "This server does not offer password recovery"));
        }
        ok(status, &body)
    }
    fn reset_confirm(&self, address: &ServerAddress, email: &str, code: &str, new_password: &str) -> AppResult<()> {
        let agent = agent(REQUEST_TIMEOUT, address.ca_pem.as_deref())?;
        let request = json!({ "email": email, "code": code, "new_password": new_password });
        let (status, body) = read(agent.post(format!("{}/v1/password-reset/confirm", address.url)).send_json(&request))?;
        ok(status, &body)
    }
    fn browser_sign_in(&self, address: &ServerAddress, provider: &str, reauth: bool, terms: Option<&str>, cancel: &AtomicBool) -> AppResult<BrowserTicket> {
        super::browser_sign_in::sign_in(address, provider, reauth, terms, cancel)
    }
    fn redeem_browser(&self, address: &ServerAddress, ticket: &BrowserTicket, device_name: &str) -> AppResult<LoginOutcome> {
        let agent = agent(REQUEST_TIMEOUT, address.ca_pem.as_deref())?;
        let body = json!({ "code": ticket.code, "code_verifier": ticket.verifier, "device_name": device_name });
        let (status, body) = read(agent.post(format!("{}/v1/login/browser", address.url)).send_json(&body))?;
        if status == 404 {
            return Err(AppError::new("not_available", "This server does not offer browser sign-in"));
        }
        login_outcome(status, body)
    }
    fn browser_disable_device(&self, address: &ServerAddress, ticket: &BrowserTicket, device_id: &str) -> AppResult<()> {
        let agent = agent(REQUEST_TIMEOUT, address.ca_pem.as_deref())?;
        let body = json!({ "code": ticket.code, "code_verifier": ticket.verifier, "device_id": device_id });
        let (status, body) = read(agent.post(format!("{}/v1/login/browser/disable-device", address.url)).send_json(&body))?;
        ok(status, &body)
    }
    fn account(&self, endpoint: &RemoteEndpoint) -> AppResult<AccountInfo> {
        let agent = agent(endpoint.timeout, endpoint.ca_pem.as_deref())?;
        let (status, body) = read(agent.get(format!("{}/v1/account", endpoint.url)).header("Authorization", bearer(endpoint)).call())?;
        if status == 405 || (status == 404 && body["error"]["code"].as_str() == Some("not_found")) {
            // A server older than the account route.
            return Err(AppError::new("not_available", "This server does not describe the account"));
        }
        ok(status, &body)?;
        serde_json::from_value(body).map_err(|_| AppError::new("server_error", "The Qamille server answered with an error"))
    }
    fn checkout(&self, endpoint: &RemoteEndpoint, plan_id: &str) -> AppResult<BillingPage> {
        let agent = agent(endpoint.timeout, endpoint.ca_pem.as_deref())?;
        let (status, body) = read(
            agent.post(format!("{}/v1/billing/checkout", endpoint.url)).header("Authorization", bearer(endpoint)).send_json(json!({ "plan_id": plan_id })),
        )?;
        if status == 404 && body["error"]["code"].as_str() == Some("not_found") {
            return Err(AppError::new("not_available", "This server does not sell subscriptions"));
        }
        ok(status, &body)?;
        serde_json::from_value(body).map_err(|_| AppError::new("server_error", "The Qamille server answered with an error"))
    }
    fn checkout_status(&self, endpoint: &RemoteEndpoint, id: &str) -> AppResult<CheckoutStatus> {
        let agent = agent(endpoint.timeout, endpoint.ca_pem.as_deref())?;
        let (status, body) = read(agent.get(format!("{}/v1/billing/checkout/{id}", endpoint.url)).header("Authorization", bearer(endpoint)).call())?;
        ok(status, &body)?;
        serde_json::from_value(body).map_err(|_| AppError::new("server_error", "The Qamille server answered with an error"))
    }
    fn billing_portal(&self, endpoint: &RemoteEndpoint) -> AppResult<BillingPage> {
        let agent = agent(endpoint.timeout, endpoint.ca_pem.as_deref())?;
        let (status, body) = read(agent.post(format!("{}/v1/billing/portal", endpoint.url)).header("Authorization", bearer(endpoint)).send_empty())?;
        ok(status, &body)?;
        serde_json::from_value(body).map_err(|_| AppError::new("server_error", "The Qamille server answered with an error"))
    }
    fn delete_account_reauthenticated(&self, endpoint: &RemoteEndpoint, ticket: &BrowserTicket) -> AppResult<()> {
        let agent = agent(endpoint.timeout, endpoint.ca_pem.as_deref())?;
        let body = json!({ "login_code": ticket.code, "code_verifier": ticket.verifier });
        let (status, body) = read(
            agent.delete(format!("{}/v1/account", endpoint.url)).header("Authorization", bearer(endpoint)).force_send_body().send_json(&body),
        )?;
        ok(status, &body)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn io(text: &str) -> ureq::Error {
        ureq::Error::Io(std::io::Error::new(std::io::ErrorKind::InvalidData, text.to_owned()))
    }

    #[test]
    fn a_refused_certificate_is_told_apart_from_a_network_failure() {
        for text in ["invalid peer certificate: UnknownIssuer", "invalid peer certificate: Expired", "invalid peer certificate: NotValidForName"] {
            assert_eq!(transport_error(&io(text)).code, "tls_untrusted", "{text}");
        }
        assert_eq!(transport_error(&ureq::Error::Tls("tls handshake failed")).code, "tls_untrusted");
        for error in [
            io("received corrupt message of type InvalidContentType"),
            ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::ConnectionRefused)),
            ureq::Error::Io(std::io::Error::from(std::io::ErrorKind::TimedOut)),
            ureq::Error::HostNotFound,
            ureq::Error::Timeout(ureq::Timeout::Global),
        ] {
            assert_eq!(transport_error(&error).code, "unreachable", "{error}");
        }
        assert_eq!(untrusted().message, "The server's certificate is not trusted");
    }
}
