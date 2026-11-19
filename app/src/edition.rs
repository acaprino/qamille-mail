//! Which build this is (spec A6): self-hosted, or Professional with the server fixed at build
//! time by `QAMILLE_PRESET_SERVER`. `build.rs` refuses an invalid value, so a Professional
//! build can never ship as self-hosted by mistake. The interface reads it with `app_edition`.
use serde::Serialize;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Edition {
    SelfHosted,
    Professional { server: String },
}

impl Edition {
    /// The preset server of a Professional build.
    pub fn preset(&self) -> Option<&str> {
        match self {
            Self::SelfHosted => None,
            Self::Professional { server } => Some(server),
        }
    }
}

/// The edition for a preset value: none or empty is self-hosted.
pub fn edition_from(preset: Option<&str>) -> Edition {
    match preset.map(str::trim).filter(|value| !value.is_empty()) {
        None => Edition::SelfHosted,
        Some(value) => match crate::domain::server_url::normalize_server_url(value) {
            Ok(server) => Edition::Professional { server },
            // Unreachable: build.rs fails the build first.
            Err(_) => Edition::SelfHosted,
        },
    }
}

/// The edition of this build.
pub fn edition() -> Edition {
    edition_from(option_env!("QAMILLE_PRESET_SERVER"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn the_preset_decides_the_edition() {
        assert_eq!(edition_from(None), Edition::SelfHosted);
        assert_eq!(edition_from(Some("  ")), Edition::SelfHosted);
        assert_eq!(
            edition_from(Some("HTTPS://Qamille.Example/")),
            Edition::Professional { server: "https://qamille.example".into() }
        );
        assert_eq!(serde_json::to_string(&Edition::SelfHosted).unwrap(), r#"{"kind":"self-hosted"}"#);
    }
}
