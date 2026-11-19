//! Developer-owned Google Desktop OAuth configuration. Credentials are a pair:
//! an app secret must never be sent for a different, user-configured client ID.

use crate::application::{AppError, AppResult, ports::GOOGLE_NOT_CONFIGURED};

struct Client {
    id: String,
    secret: String,
}

#[derive(Default)]
struct Configuration {
    runtime: Option<Client>,
    build: Option<Client>,
}

impl Client {
    fn new(id: &str, secret: &str) -> Option<Self> {
        let id = id.trim();
        (!id.is_empty()).then(|| Self {
            id: id.into(),
            secret: secret.trim().into(),
        })
    }
}

fn configured() -> Configuration {
    // Runtime configuration is useful for local developer builds. An explicit
    // runtime ID selects its own secret, never one from a different build client.
    let runtime_id = std::env::var("QAMILLE_GOOGLE_CLIENT_ID").unwrap_or_default();
    let runtime_secret = std::env::var("QAMILLE_GOOGLE_CLIENT_SECRET").unwrap_or_default();
    select_config(
        &runtime_id,
        &runtime_secret,
        option_env!("QAMILLE_GOOGLE_CLIENT_ID").unwrap_or(""),
        option_env!("QAMILLE_GOOGLE_CLIENT_SECRET").unwrap_or(""),
    )
}

fn select_config(
    runtime_id: &str,
    runtime_secret: &str,
    build_id: &str,
    build_secret: &str,
) -> Configuration {
    Configuration {
        runtime: Client::new(runtime_id, runtime_secret),
        build: Client::new(build_id, build_secret),
    }
}

fn resolve(
    app: Configuration,
    legacy_id: &str,
    legacy_secret: &str,
) -> AppResult<(String, String)> {
    let legacy_id = legacy_id.trim();
    if legacy_id.is_empty() {
        return app
            .runtime
            .or(app.build)
            .map(|client| (client.id, client.secret))
            .ok_or_else(|| AppError::new("google_not_configured", GOOGLE_NOT_CONFIGURED));
    }
    let secret = matching_secret(&app, legacy_id, legacy_secret);
    Ok((legacy_id.into(), secret))
}

fn matching_secret(app: &Configuration, client_id: &str, legacy_secret: &str) -> String {
    let client = app
        .runtime
        .as_ref()
        .filter(|client| client.id == client_id.trim())
        .or_else(|| {
            app.build
                .as_ref()
                .filter(|client| client.id == client_id.trim())
        });
    match client {
        Some(client) => client.secret.clone(),
        None => legacy_secret.trim().into(),
    }
}

pub fn credentials(legacy_id: &str, legacy_secret: &str) -> AppResult<(String, String)> {
    resolve(configured(), legacy_id, legacy_secret)
}

/// Whether `client_id` is the client of this build (or the one set for a developer run): its
/// secret belongs to the app, so it is never copied to another computer.
pub fn is_app_client(client_id: &str) -> bool {
    let app = configured();
    let id = client_id.trim();
    !id.is_empty() && [app.runtime.as_ref(), app.build.as_ref()].into_iter().flatten().any(|client| client.id == id)
}

/// Refresh and calendar consent use the client that originally authorized the account.
pub fn secret_for_client(client_id: &str, legacy_secret: &str) -> String {
    matching_secret(&configured(), client_id, legacy_secret)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> Configuration {
        select_config("", "", "app.apps.googleusercontent.com", "app-secret")
    }

    #[test]
    fn app_configuration_is_used_without_user_settings() {
        let (id, secret) = resolve(app(), "", "old-unrelated-secret").unwrap();
        assert_eq!(id, "app.apps.googleusercontent.com");
        assert_eq!(secret, "app-secret");
    }

    #[test]
    fn runtime_override_selects_a_complete_pair_without_mixing_build_secret() {
        let client = resolve(
            select_config(" runtime-client ", "", "build-client", "build-secret"),
            "",
            "",
        )
        .unwrap();
        assert_eq!(client, ("runtime-client".into(), "".into()));
        let client = resolve(
            select_config(
                "",
                "unrelated-runtime-secret",
                "build-client",
                "build-secret",
            ),
            "",
            "",
        )
        .unwrap();
        assert_eq!(client, ("build-client".into(), "build-secret".into()));
        assert!(resolve(select_config("", "orphan-secret", "", ""), "", "").is_err());
    }

    #[test]
    fn explicit_legacy_client_keeps_its_own_secret() {
        assert_eq!(
            resolve(app(), " custom-client ", " custom-secret ").unwrap(),
            ("custom-client".into(), "custom-secret".into())
        );
        assert_eq!(matching_secret(&app(), "custom-client", ""), "");
    }

    #[test]
    fn matching_client_uses_app_secret_for_sign_in_and_refresh() {
        assert_eq!(
            resolve(app(), "app.apps.googleusercontent.com", "unrelated")
                .unwrap()
                .1,
            "app-secret"
        );
        assert_eq!(
            matching_secret(&app(), "app.apps.googleusercontent.com", "unrelated"),
            "app-secret"
        );
    }

    #[test]
    fn compiled_client_remains_available_to_accounts_after_a_runtime_override() {
        let selected = select_config(
            "runtime-client",
            "runtime-secret",
            "build-client",
            "build-secret",
        );
        assert_eq!(
            matching_secret(&selected, "build-client", "unrelated"),
            "build-secret"
        );
        assert_eq!(
            matching_secret(&selected, "runtime-client", "unrelated"),
            "runtime-secret"
        );
        assert_eq!(
            matching_secret(&selected, "another-client", "legacy-secret"),
            "legacy-secret"
        );
        assert_eq!(
            resolve(selected, "build-client", "").unwrap().1,
            "build-secret"
        );
    }

    #[test]
    fn legacy_configuration_works_without_an_app_client() {
        assert_eq!(
            resolve(Configuration::default(), "custom-client", "custom-secret").unwrap(),
            ("custom-client".into(), "custom-secret".into())
        );
    }

    #[test]
    fn missing_configuration_is_an_app_issue() {
        let error = resolve(Configuration::default(), " \t", "orphan-secret").unwrap_err();
        assert_eq!(error.code, "google_not_configured");
        assert_eq!(error.message, GOOGLE_NOT_CONFIGURED);
        assert!(Client::new(" \t", "secret").is_none());
    }
}
