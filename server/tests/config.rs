#![allow(dead_code)]

mod support;

use std::collections::HashMap;
use std::net::{IpAddr, SocketAddr};

use qamille_server::config::{AdminKey, Config, ConfigError, Edition};
use support::TempDir;

fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
    let map: HashMap<String, String> = pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
        .collect();
    move |name| map.get(name).cloned()
}

#[test]
fn defaults_without_file_or_environment() {
    let dir = TempDir::new();
    let data = dir.path().to_str().unwrap().to_owned();
    let config = Config::load_with(None, env_of(&[("QAMILLE_DATA_DIR", &data)])).unwrap();
    assert_eq!(config.bind, "0.0.0.0:8787".parse::<SocketAddr>().unwrap());
    assert_eq!(config.admin_bind, None);
    assert_eq!(config.default_max_devices, 3);
    assert!(config.trusted_proxies.is_empty());
    assert_eq!(config.login_max_failures_per_origin, 30);
    assert_eq!(config.login_failure_window_secs, 600);
    assert_eq!(config.admin_max_failures, 300);
    assert_eq!(config.admin_failure_window_secs, 60);
    assert_eq!(config.temporary_password_days, 7);
    assert_eq!(config.device_inactive_days, 180);
    assert!(config.admin_key.is_none());
    assert_eq!(config.edition, Edition::SelfHosted);
    assert_eq!(config.data_dir, dir.path());
}

#[test]
fn environment_wins_over_file() {
    let dir = TempDir::new();
    let file = dir.path().join("custom.toml");
    std::fs::write(
        &file,
        r#"
bind = "127.0.0.1:9000"
admin_bind = "127.0.0.1:9001"
default_max_devices = 5
trusted_proxies = ["10.0.0.2"]
temporary_password_days = 3
"#,
    )
    .unwrap();
    let config = Config::load_with(
        Some(&file),
        env_of(&[
            ("QAMILLE_DEFAULT_MAX_DEVICES", "7"),
            ("QAMILLE_TRUSTED_PROXIES", "10.0.0.3, 10.0.0.4"),
            ("QAMILLE_ADMIN_BIND", ""),
            ("QAMILLE_EDITION", "professional"),
        ]),
    )
    .unwrap();
    assert_eq!(config.bind, "127.0.0.1:9000".parse::<SocketAddr>().unwrap());
    assert_eq!(config.default_max_devices, 7);
    assert_eq!(
        config.trusted_proxies,
        vec![
            "10.0.0.3".parse::<IpAddr>().unwrap(),
            "10.0.0.4".parse::<IpAddr>().unwrap()
        ]
    );
    // Variabile vuota: nessun ascolto amministrativo, anche se il file lo indica.
    assert_eq!(config.admin_bind, None);
    assert_eq!(config.temporary_password_days, 3);
    assert_eq!(config.edition, Edition::Professional);
}

#[test]
fn file_in_data_dir_is_read_without_config_flag() {
    let dir = TempDir::new();
    std::fs::write(dir.path().join("server.toml"), "default_max_devices = 0\n").unwrap();
    let data = dir.path().to_str().unwrap().to_owned();
    let config = Config::load_with(None, env_of(&[("QAMILLE_DATA_DIR", &data)])).unwrap();
    assert_eq!(config.default_max_devices, 0);
}

#[test]
fn unknown_file_keys_are_rejected() {
    let dir = TempDir::new();
    let file = dir.path().join("server.toml");
    std::fs::write(&file, "admin_key = \"no\"\n").unwrap();
    let err = Config::load_with(Some(&file), env_of(&[])).unwrap_err();
    assert!(matches!(err, ConfigError::Parse { .. }), "{err:?}");
}

#[test]
fn short_or_repetitive_admin_key_is_rejected() {
    let short = "a1b2c3d4e5f6a7b8c9d0";
    let repetitive = "abababababababababababababababababababab";
    for key in [short, repetitive] {
        let err = Config::load_with(
            None,
            env_of(&[
                ("QAMILLE_DATA_DIR", "/nonexistent"),
                ("QAMILLE_ADMIN_KEY", key),
            ]),
        )
        .unwrap_err();
        assert!(matches!(err, ConfigError::WeakAdminKey), "{err:?}");
        // Il messaggio non ripete la chiave.
        assert!(!err.to_string().contains(key));
    }
    assert!(AdminKey::new("0123456").is_err());
    // 31 caratteri non bastano, 32 sì.
    assert!(AdminKey::new("0123456789abcdef0123456789abcde").is_err());
    assert!(AdminKey::new("0123456789abcdef0123456789abcdef").is_ok());
}

#[test]
fn valid_admin_key_is_kept_and_hidden_from_debug() {
    let key = "f3a9c1d7e5b2048f6a1c3e5d7b9f0a2c4e6d8b1a3c5e7f9d0b2a4c6e8f1a3b5d";
    let config = Config::load_with(
        None,
        env_of(&[
            ("QAMILLE_DATA_DIR", "/nonexistent"),
            ("QAMILLE_ADMIN_KEY", key),
            ("QAMILLE_ADMIN_BIND", "127.0.0.1:8788"),
        ]),
    )
    .unwrap();
    assert_eq!(config.admin_key.as_ref().unwrap().expose(), key);
    assert!(config.admin_listener().is_some());
    assert!(!format!("{config:?}").contains(key));
}

#[test]
fn empty_admin_key_means_no_admin_listener() {
    let config = Config::load_with(
        None,
        env_of(&[
            ("QAMILLE_DATA_DIR", "/nonexistent"),
            ("QAMILLE_ADMIN_KEY", ""),
            ("QAMILLE_ADMIN_BIND", "127.0.0.1:8788"),
        ]),
    )
    .unwrap();
    assert!(config.admin_key.is_none());
    assert!(config.admin_listener().is_none());
}

#[test]
fn invalid_trusted_proxies_are_rejected() {
    let err = Config::load_with(
        None,
        env_of(&[
            ("QAMILLE_DATA_DIR", "/nonexistent"),
            ("QAMILLE_TRUSTED_PROXIES", "10.0.0.2, proxy.example"),
        ]),
    )
    .unwrap_err();
    assert!(
        matches!(err, ConfigError::InvalidTrustedProxy(ref v) if v == "proxy.example"),
        "{err:?}"
    );

    let dir = TempDir::new();
    let file = dir.path().join("server.toml");
    std::fs::write(&file, "trusted_proxies = [\"10.0.0.0/8\"]\n").unwrap();
    let err = Config::load_with(Some(&file), env_of(&[])).unwrap_err();
    assert!(
        matches!(err, ConfigError::InvalidTrustedProxy(_)),
        "{err:?}"
    );
}

// --- Posta e registrazione (wave 3) ---------------------------------------------

fn data_env<'a>(dir: &'a TempDir, extra: &[(&'a str, &'a str)]) -> Vec<(&'a str, String)> {
    let mut pairs = vec![(
        "QAMILLE_DATA_DIR",
        dir.path().to_str().unwrap().to_owned(),
    )];
    pairs.extend(extra.iter().map(|(k, v)| (*k, (*v).to_owned())));
    pairs
}

fn load(pairs: &[(&str, String)]) -> Result<Config, ConfigError> {
    let borrowed: Vec<(&str, &str)> = pairs.iter().map(|(k, v)| (*k, v.as_str())).collect();
    Config::load_with(None, env_of(&borrowed))
}

#[test]
fn compute_keys_of_the_remote_mode() {
    let dir = TempDir::new();
    let data = dir.path().to_str().unwrap().to_owned();
    let config = Config::load_with(None, env_of(&[("QAMILLE_DATA_DIR", &data)])).unwrap();
    assert_eq!(config.compute_max_in_flight_per_user, 2);
    assert_eq!(config.compute_max_auth_failures_per_origin, 120);
    assert_eq!(config.models_dir(), dir.path().join("models"));
    assert_eq!(config.speech_models_dir(), dir.path().join("models").join("speech"));
    assert_eq!(config.transcription_parallelism, None);
    let file = dir.path().join("server.toml");
    std::fs::write(&file, "models_dir = \"/srv/models\"\ncompute_max_in_flight_per_user = 4\n").unwrap();
    let config = Config::load_with(Some(&file), env_of(&[("QAMILLE_TRANSCRIPTION_PARALLELISM", "3")])).unwrap();
    assert_eq!(config.models_dir(), std::path::PathBuf::from("/srv/models"));
    assert_eq!(config.compute_max_in_flight_per_user, 4);
    assert_eq!(config.transcription_parallelism, Some(3));
    let config = Config::load_with(Some(&file), env_of(&[("QAMILLE_MODELS_DIR", "/data/m"), ("QAMILLE_SPEECH_MODELS_DIR", "/data/s")])).unwrap();
    assert_eq!(config.models_dir(), std::path::PathBuf::from("/data/m"));
    assert_eq!(config.speech_models_dir(), std::path::PathBuf::from("/data/s"));
    for (key, value) in [("QAMILLE_COMPUTE_MAX_IN_FLIGHT_PER_USER", "0"), ("QAMILLE_COMPUTE_MAX_AUTH_FAILURES_PER_ORIGIN", "0"), ("QAMILLE_TRANSCRIPTION_PARALLELISM", "0")] {
        assert!(Config::load_with(None, env_of(&[("QAMILLE_DATA_DIR", &data), (key, value)])).is_err(), "{key}");
    }
}

// --- Vendita: accesso con Google e abbonamenti --------------------------------------

// --- Core ed enterprise (wave 1, compito 6) ----------------------------------------

fn file_with(dir: &TempDir, text: &str) -> std::path::PathBuf {
    let path = dir.path().join("server.toml");
    std::fs::write(&path, text).unwrap();
    path
}

#[test]
fn the_core_refuses_an_enterprise_key_in_the_file() {
    let dir = TempDir::new();
    let path = file_with(&dir, "smtp_host = \"mail.example\"
");
    let error = Config::load_with(Some(&path), env_of(&[])).unwrap_err();
    assert!(error.to_string().contains("smtp_host"), "{error}");
}

#[test]
fn load_split_returns_the_keys_it_does_not_know() {
    let dir = TempDir::new();
    let path = file_with(&dir, "default_max_devices = 5
smtp_host = \"mail.example\"
");
    let (core, rest) = Config::load_split(Some(&path), env_of(&[])).unwrap();
    assert_eq!(core.default_max_devices, 5);
    assert_eq!(rest.keys().collect::<Vec<_>>(), ["smtp_host"]);
}

#[test]
fn leftover_enterprise_variables_are_reported() {
    let env = |name: &str| (name == "QAMILLE_SIGNUP").then(|| "1".to_owned());
    assert_eq!(Config::ignored_env(env), ["QAMILLE_SIGNUP"]);
    assert!(Config::ignored_env(|_| None).is_empty());
}

#[test]
fn the_core_ignores_enterprise_variables_when_loading() {
    let dir = TempDir::new();
    let data = dir.path().to_str().unwrap().to_owned();
    let pairs = [("QAMILLE_DATA_DIR", data.as_str()), ("QAMILLE_SIGNUP", "1"), ("QAMILLE_SMTP_PORT", "zz")];
    Config::load_with(None, env_of(&pairs)).unwrap();
}

// --- plaintext_from (wave 2) -----------------------------------------------------

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

#[test]
fn plaintext_from_is_read_from_file_and_environment() {
    let dir = TempDir::new();
    let path = file_with(&dir, "plaintext_from = [\"172.30.88.1\"]\n");
    let config = Config::load_with(Some(&path), env_of(&[("QAMILLE_DATA_DIR", "/nonexistent")])).unwrap();
    assert_eq!(config.plaintext_from, [ip("172.30.88.1")]);
    let config = Config::load_with(
        Some(&path),
        env_of(&[
            ("QAMILLE_DATA_DIR", "/nonexistent"),
            ("QAMILLE_PLAINTEXT_FROM", "10.0.0.9,::1"),
        ]),
    )
    .unwrap();
    assert_eq!(config.plaintext_from, [ip("10.0.0.9"), ip("::1")]);
}

#[test]
fn plaintext_from_accepts_loopback_and_private_addresses_without_a_private_bind() {
    let mut config = Config::with_data_dir("unused");
    config.bind = "0.0.0.0:8787".parse().unwrap();
    config.admin_bind = None;
    config.plaintext_from = vec![ip("172.30.88.1"), ip("127.0.0.1"), ip("fd00::1")];
    assert!(config.validate().is_ok());
}

#[test]
fn a_public_address_in_plaintext_from_stops_the_start() {
    let mut config = Config::with_data_dir("unused");
    config.plaintext_from = vec![ip("203.0.113.9")];
    let error = config.validate().unwrap_err();
    assert!(error.to_string().contains("plaintext_from"), "{error}");
    let dir = TempDir::new();
    config.data_dir = dir.path().to_path_buf();
    let error = qamille_server::Server::open(config).err().unwrap();
    assert!(error.to_string().contains("plaintext_from"), "{error}");
}

#[test]
fn the_core_knows_the_plaintext_from_key() {
    let dir = TempDir::new();
    let path = file_with(&dir, "plaintext_from = [\"10.0.0.9\"]\n");
    let (_core, rest) = Config::load_split(Some(&path), env_of(&[])).unwrap();
    assert!(rest.is_empty(), "{rest:?}");
}

#[test]
fn plaintext_from_is_announced_at_startup_only_when_set() {
    let mut config = Config::with_data_dir("x");
    assert!(config.plaintext_notice().is_none());
    config.plaintext_from = vec!["172.18.0.1".parse().unwrap()];
    let notice = config.plaintext_notice().expect("notice");
    assert!(notice.contains("plaintext_from") && notice.contains("loopback"), "{notice}");
    assert!(!notice.contains('\n'));
}

// --- Gateway LLM (wave 3 del gateway) ---------------------------------------------

mod llm_config {
    use super::*;
    use qamille_server::llm::{LlmProvider, parse_base_url};

    fn llm_load(extra: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let dir = TempDir::new();
        let pairs = data_env(&dir, extra);
        load(&pairs)
    }

    fn with_provider(base: &str) -> Result<Config, ConfigError> {
        llm_load(&[
            ("QAMILLE_LLM_PROVIDER", "openai-compatible"),
            ("QAMILLE_LLM_BASE_URL", base),
            ("QAMILLE_LLM_MODEL", "m"),
        ])
    }

    fn valid(base: &str) -> bool {
        with_provider(base).is_ok()
    }

    fn label_of(label: &str) -> Result<Config, ConfigError> {
        let dir = TempDir::new();
        let file = dir.path().join("server.toml");
        let text = format!(
            "llm_provider = \"openai-compatible\"\nllm_base_url = \"https://api.openai.com/v1\"\nllm_model = \"m\"\nllm_label = {}\n",
            toml_string(label)
        );
        std::fs::write(&file, text).unwrap();
        Config::load_with(Some(&file), env_of(&[]))
    }

    fn toml_string(value: &str) -> String {
        format!("\"{}\"", value.replace('\\', "\\\\").replace('"', "\\\"").replace('\n', "\\n"))
    }

    #[test]
    fn llm_defaults_are_off_and_safe() {
        let config = llm_load(&[]).unwrap();
        assert_eq!(config.llm.provider, LlmProvider::None);
        assert!(!config.llm.enabled());
        assert_eq!(config.llm.max_output_tokens, 4096);
        assert_eq!(config.llm.max_request_bytes, 2 * 1024 * 1024);
        assert_eq!(config.llm.timeout_secs, 120);
        assert_eq!(config.llm.max_in_flight_per_user, 2);
        assert_eq!(config.llm.max_in_flight_total, 16);
        assert_eq!(config.llm.max_tokens_per_day_per_user, 0);
        assert!(config.llm.api_key.is_none());
    }

    #[test]
    fn llm_keys_are_read_from_file_and_environment() {
        let dir = TempDir::new();
        let file = dir.path().join("server.toml");
        std::fs::write(
            &file,
            "llm_provider = \"openai-compatible\"\nllm_base_url = \"http://ollama:11434/v1\"\nllm_model = \"file-model\"\nllm_allowed_models = [\"x\"]\nllm_max_output_tokens = 100\nllm_context_tokens = 8000\nllm_max_request_bytes = 5000\nllm_max_tokens_per_day_per_user = 7\nllm_timeout_secs = 9\nllm_max_in_flight_per_user = 3\nllm_max_in_flight_total = 5\nllm_label = \"Interno\"\nllm_external = false\n",
        )
        .unwrap();
        let config = Config::load_with(Some(&file), env_of(&[])).unwrap();
        assert_eq!(config.llm.model.as_deref(), Some("file-model"));
        assert_eq!(config.llm.allowed_models, vec!["x"]);
        assert_eq!(config.llm.max_output_tokens, 100);
        assert_eq!(config.llm.context_tokens, Some(8000));
        assert_eq!(config.llm.max_request_bytes, 5000);
        assert_eq!(config.llm.max_tokens_per_day_per_user, 7);
        assert_eq!(config.llm.timeout_secs, 9);
        assert_eq!(config.llm.max_in_flight_per_user, 3);
        assert_eq!(config.llm.max_in_flight_total, 5);
        assert_eq!(config.llm.display_label(), "Interno");
        assert!(!config.llm.is_external());
        let config = Config::load_with(
            Some(&file),
            env_of(&[
                ("QAMILLE_LLM_MODEL", "env-model"),
                ("QAMILLE_LLM_ALLOWED_MODELS", "a, b"),
                ("QAMILLE_LLM_MAX_OUTPUT_TOKENS", "50"),
                ("QAMILLE_LLM_CONTEXT_TOKENS", "4000"),
                ("QAMILLE_LLM_MAX_REQUEST_BYTES", "6000"),
                ("QAMILLE_LLM_MAX_TOKENS_PER_DAY_PER_USER", "8"),
                ("QAMILLE_LLM_TIMEOUT_SECS", "11"),
                ("QAMILLE_LLM_MAX_IN_FLIGHT_PER_USER", "4"),
                ("QAMILLE_LLM_MAX_IN_FLIGHT_TOTAL", "7"),
                ("QAMILLE_LLM_LABEL", "Env"),
                ("QAMILLE_LLM_EXTERNAL", "1"),
            ]),
        )
        .unwrap();
        assert_eq!(config.llm.model.as_deref(), Some("env-model"));
        assert_eq!(config.llm.allowed_models, vec!["a", "b"]);
        assert_eq!(config.llm.models(), vec!["env-model", "a", "b"]);
        assert_eq!(config.llm.max_output_tokens, 50);
        assert_eq!(config.llm.context_tokens, Some(4000));
        assert_eq!(config.llm.max_request_bytes, 6000);
        assert_eq!(config.llm.max_tokens_per_day_per_user, 8);
        assert_eq!(config.llm.timeout_secs, 11);
        assert_eq!(config.llm.max_in_flight_per_user, 4);
        assert_eq!(config.llm.max_in_flight_total, 7);
        assert_eq!(config.llm.display_label(), "Env");
        assert!(config.llm.is_external());
        // Doppioni tolti, predefinito per primo.
        let config = llm_load(&[
            ("QAMILLE_LLM_PROVIDER", "openai-compatible"),
            ("QAMILLE_LLM_BASE_URL", "http://ollama:11434/v1"),
            ("QAMILLE_LLM_MODEL", "m"),
            ("QAMILLE_LLM_ALLOWED_MODELS", "b,m,b"),
        ])
        .unwrap();
        assert_eq!(config.llm.models(), vec!["m", "b"]);
    }

    #[test]
    fn an_unknown_llm_provider_stops_the_start() {
        match llm_load(&[("QAMILLE_LLM_PROVIDER", "ollama")]) {
            Err(ConfigError::Invalid { key: "llm_provider", .. }) => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn openai_compatible_needs_base_url_and_model() {
        let p = ("QAMILLE_LLM_PROVIDER", "openai-compatible");
        assert!(llm_load(&[p, ("QAMILLE_LLM_MODEL", "m")]).is_err());
        assert!(llm_load(&[p, ("QAMILLE_LLM_BASE_URL", "http://ollama:11434/v1")]).is_err());
        assert!(
            llm_load(&[p, ("QAMILLE_LLM_BASE_URL", "http://ollama:11434/v1"), ("QAMILLE_LLM_MODEL", "m")])
                .is_ok()
        );
    }

    #[test]
    fn http_is_only_for_internal_hosts() {
        for ok in [
            "http://localhost:11434/v1",
            "http://127.0.0.1:8000/v1",
            "http://192.168.1.5/v1",
            "http://10.0.0.2/v1",
            "http://[fd00::1]:8000/v1",
            "http://ollama:11434/v1",
            "https://api.openai.com/v1",
        ] {
            assert!(valid(ok), "{ok}");
        }
        for bad in [
            "http://api.openai.com/v1",
            "http://8.8.8.8/v1",
            "http://ollama.example.com/v1",
            "http://[2001:db8::1]/v1",
            "http://134744072/v1",
            "http://0x08080808/v1",
            "http://0X08080808/v1",
        ] {
            assert!(!valid(bad), "{bad}");
        }
    }

    #[test]
    fn a_base_url_with_credentials_query_or_fragment_is_refused() {
        for bad in [
            "https://u:p@api.example.com/v1",
            "https://api.example.com/v1?a=b",
            "https://api.example.com/v1#x",
            "https://api.example.com/v 1",
            "ftp://api.example.com/v1",
            "file:///etc/passwd",
            "api.example.com/v1",
        ] {
            assert!(parse_base_url(bad).is_err(), "{bad}");
            assert!(!valid(bad), "{bad}");
        }
    }

    #[test]
    fn external_and_label_are_deduced_and_can_be_forced() {
        let c = with_provider("http://ollama:11434/v1").unwrap();
        assert!(!c.llm.is_external());
        assert_eq!(c.llm.display_label(), "ollama");
        let c = with_provider("https://api.openai.com/v1").unwrap();
        assert!(c.llm.is_external());
        assert_eq!(c.llm.display_label(), "OpenAI");
        let c = with_provider("https://x.openai.azure.com/openai/v1").unwrap();
        assert!(c.llm.is_external());
        assert_eq!(c.llm.display_label(), "Azure OpenAI");
        let c = llm_load(&[
            ("QAMILLE_LLM_PROVIDER", "openai-compatible"),
            ("QAMILLE_LLM_BASE_URL", "http://ollama:11434/v1"),
            ("QAMILLE_LLM_MODEL", "m"),
            ("QAMILLE_LLM_EXTERNAL", "true"),
            ("QAMILLE_LLM_LABEL", "Acme AI"),
        ])
        .unwrap();
        assert!(c.llm.is_external());
        assert_eq!(c.llm.display_label(), "Acme AI");
        let base = parse_base_url("http://[fd00::1]:8000/v1/").unwrap();
        assert_eq!(base.chat_url(), "http://[fd00::1]:8000/v1/chat/completions");
        assert!(parse_base_url("https://api.openai.com/v1").unwrap().wants_completion_tokens());
        assert!(!parse_base_url("http://ollama/v1").unwrap().wants_completion_tokens());
    }

    #[test]
    fn a_forced_label_must_be_plain_ascii() {
        assert!(label_of("Acme AI").is_ok());
        assert!(label_of(&"a".repeat(64)).is_ok());
        for bad in ["a;b", "", &"a".repeat(65), "a\nb", "Società"] {
            assert!(label_of(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn numeric_limits_must_be_positive() {
        for key in [
            "QAMILLE_LLM_MAX_OUTPUT_TOKENS",
            "QAMILLE_LLM_MAX_REQUEST_BYTES",
            "QAMILLE_LLM_TIMEOUT_SECS",
            "QAMILLE_LLM_MAX_IN_FLIGHT_PER_USER",
            "QAMILLE_LLM_MAX_IN_FLIGHT_TOTAL",
        ] {
            assert!(llm_load(&[(key, "0")]).is_err(), "{key}");
        }
        assert!(llm_load(&[("QAMILLE_LLM_MAX_TOKENS_PER_DAY_PER_USER", "0")]).is_ok());
    }

    #[test]
    fn the_provider_key_comes_only_from_the_environment() {
        let c = llm_load(&[("QAMILLE_LLM_API_KEY", "  sk-segreto  ")]).unwrap();
        assert_eq!(c.llm.api_key.as_ref().unwrap().expose(), "sk-segreto");
        assert!(llm_load(&[("QAMILLE_LLM_API_KEY", "")]).unwrap().llm.api_key.is_none());
        let dir = TempDir::new();
        let file = dir.path().join("server.toml");
        std::fs::write(&file, "llm_api_key = \"sk-nel-file\"\n").unwrap();
        let error = Config::load_with(Some(&file), env_of(&[])).unwrap_err();
        assert!(error.to_string().contains("unknown field `llm_api_key`"), "{error}");
    }

    #[test]
    fn the_debug_of_the_configuration_hides_the_provider_key() {
        let c = llm_load(&[("QAMILLE_LLM_API_KEY", "sk-segreto-9999")]).unwrap();
        for text in [format!("{c:?}"), format!("{c:#?}"), format!("{:?}", c.llm)] {
            assert!(!text.contains("sk-segreto-9999"), "{text}");
        }
        assert!(format!("{c:?}").contains("<nascosta>"));
    }

    // --- Modelli predefiniti (wave 4) ---

    /// An absolute path on every platform.
    fn absolute(name: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(name)
    }

    fn builtin(extra: &[(&str, &str)]) -> Result<Config, ConfigError> {
        let mut pairs = vec![("QAMILLE_LLM_PROVIDER", "builtin"), ("QAMILLE_LLM_BUILTIN_MODEL", "llm-medio")];
        pairs.extend_from_slice(extra);
        llm_load(&pairs)
    }

    fn invalid_key(result: Result<Config, ConfigError>) -> &'static str {
        match result {
            Err(ConfigError::Invalid { key, .. }) => key,
            other => panic!("expected an Invalid error, got {other:?}"),
        }
    }

    #[test]
    fn builtin_is_a_known_provider_and_needs_a_catalog_model() {
        assert_eq!(invalid_key(llm_load(&[("QAMILLE_LLM_PROVIDER", "builtin")])), "llm_builtin_model");
        assert_eq!(
            invalid_key(llm_load(&[("QAMILLE_LLM_PROVIDER", "builtin"), ("QAMILLE_LLM_BUILTIN_MODEL", "llm-inesistente")])),
            "llm_builtin_model"
        );
        let config = builtin(&[]).unwrap();
        assert_eq!(config.llm.provider, LlmProvider::Builtin);
        assert_eq!(config.llm.provider.as_str(), "builtin");
        let message = llm_load(&[("QAMILLE_LLM_PROVIDER", "ollama")]).unwrap_err().to_string();
        assert!(message.contains("none") && message.contains("builtin") && message.contains("openai-compatible"), "{message}");
    }

    #[test]
    fn builtin_defaults_are_context_8192_idle_600_one_slot_and_timeout_600() {
        let config = builtin(&[]).unwrap();
        assert_eq!(config.llm.builtin_context, 8192);
        assert_eq!(config.llm.builtin_idle_secs, 600);
        assert_eq!(config.llm.builtin_slots, 1);
        assert_eq!(config.llm.timeout_secs, 600);
        assert!(config.llm.builtin_program.is_none());
        assert_eq!(llm_load(&[]).unwrap().llm.timeout_secs, 120, "other providers keep their default");
    }

    #[test]
    fn the_builtin_keys_are_read_from_file_and_environment() {
        let dir = TempDir::new();
        let file = dir.path().join("server.toml");
        let from_file = absolute("llama-from-file");
        let from_env = absolute("llama-from-env");
        std::fs::write(
            &file,
            format!("llm_provider = \"builtin\"\nllm_builtin_model = \"llm-piccolo\"\nllm_builtin_context = 2048\nllm_builtin_idle_secs = 20\nllm_builtin_slots = 3\nllm_builtin_program = '{}'\nmodels_mirror_url = \"https://mirror.example.com\"\n", from_file.display()),
        )
        .unwrap();
        let config = Config::load_with(Some(&file), env_of(&[])).unwrap();
        assert_eq!(config.llm.builtin_model.as_deref(), Some("llm-piccolo"));
        assert_eq!((config.llm.builtin_context, config.llm.builtin_idle_secs, config.llm.builtin_slots), (2048, 20, 3));
        assert_eq!(config.llm.builtin_program.as_deref(), Some(from_file.as_path()));
        assert_eq!(config.models_mirror_url.as_deref(), Some("https://mirror.example.com"));
        let env_program = from_env.to_str().unwrap().to_owned();
        let config = Config::load_with(
            Some(&file),
            env_of(&[
                ("QAMILLE_LLM_BUILTIN_MODEL", "llm-grande"),
                ("QAMILLE_LLM_BUILTIN_CONTEXT", "16384"),
                ("QAMILLE_LLM_BUILTIN_IDLE_SECS", "90"),
                ("QAMILLE_LLM_BUILTIN_SLOTS", "4"),
                ("QAMILLE_LLM_BUILTIN_PROGRAM", env_program.as_str()),
                ("QAMILLE_MODELS_MIRROR_URL", "https://other.example.com/base"),
            ]),
        )
        .unwrap();
        assert_eq!(config.llm.builtin_model.as_deref(), Some("llm-grande"));
        assert_eq!((config.llm.builtin_context, config.llm.builtin_idle_secs, config.llm.builtin_slots), (16384, 90, 4));
        assert_eq!(config.llm.builtin_program.as_deref(), Some(from_env.as_path()));
        assert_eq!(config.models_mirror_url.as_deref(), Some("https://other.example.com/base"));
    }

    #[test]
    fn builtin_bounds_are_checked() {
        for (name, value, key) in [
            ("QAMILLE_LLM_BUILTIN_CONTEXT", "1000", "llm_builtin_context"),
            ("QAMILLE_LLM_BUILTIN_CONTEXT", "300000", "llm_builtin_context"),
            ("QAMILLE_LLM_BUILTIN_IDLE_SECS", "9", "llm_builtin_idle_secs"),
            ("QAMILLE_LLM_BUILTIN_SLOTS", "0", "llm_builtin_slots"),
            ("QAMILLE_LLM_BUILTIN_SLOTS", "5", "llm_builtin_slots"),
            ("QAMILLE_LLM_BUILTIN_PROGRAM", "bin/llama-server", "llm_builtin_program"),
        ] {
            assert_eq!(invalid_key(builtin(&[(name, value)])), key, "{name}={value}");
        }
        for (name, value) in [
            ("QAMILLE_LLM_BUILTIN_CONTEXT", "1024"),
            ("QAMILLE_LLM_BUILTIN_CONTEXT", "262144"),
            ("QAMILLE_LLM_BUILTIN_IDLE_SECS", "10"),
            ("QAMILLE_LLM_BUILTIN_SLOTS", "1"),
            ("QAMILLE_LLM_BUILTIN_SLOTS", "4"),
        ] {
            assert!(builtin(&[(name, value)]).is_ok(), "{name}={value}");
        }
    }

    #[test]
    fn builtin_refuses_a_base_url_a_context_override_and_a_forced_external() {
        assert_eq!(invalid_key(builtin(&[("QAMILLE_LLM_BASE_URL", "http://ollama:11434/v1")])), "llm_base_url");
        assert_eq!(invalid_key(builtin(&[("QAMILLE_LLM_CONTEXT_TOKENS", "4000")])), "llm_context_tokens");
        assert_eq!(invalid_key(builtin(&[("QAMILLE_LLM_EXTERNAL", "1")])), "llm_external");
        assert!(builtin(&[("QAMILLE_LLM_EXTERNAL", "0")]).is_ok());
        assert_eq!(invalid_key(builtin(&[("QAMILLE_LLM_MODEL", "llm-piccolo")])), "llm_model");
        assert!(builtin(&[("QAMILLE_LLM_MODEL", "llm-medio")]).is_ok());
    }

    #[test]
    fn builtin_allows_only_its_own_model_in_the_list() {
        assert_eq!(invalid_key(builtin(&[("QAMILLE_LLM_ALLOWED_MODELS", "altro")])), "llm_allowed_models");
        assert_eq!(invalid_key(builtin(&[("QAMILLE_LLM_ALLOWED_MODELS", "llm-medio, altro")])), "llm_allowed_models");
        assert_eq!(builtin(&[("QAMILLE_LLM_ALLOWED_MODELS", "llm-medio")]).unwrap().llm.models(), vec!["llm-medio"]);
    }

    #[test]
    fn builtin_is_internal_with_the_fixed_label_and_context_in_info() {
        let config = builtin(&[("QAMILLE_LLM_LABEL", "Altro nome")]).unwrap();
        assert!(!config.llm.is_external());
        assert_eq!(config.llm.display_label(), "questo server", "the consent label is fixed");
        assert_eq!(config.llm.models(), vec!["llm-medio"]);
        let info = qamille_server::llm::LlmInfo::from_config(&config.llm);
        assert_eq!(info.context_tokens, Some(8192));
        assert_eq!(info.model, "llm-medio");
        assert!(!info.external);
        assert_eq!(info.label, "questo server");
    }

    #[test]
    fn an_explicit_timeout_wins_over_the_builtin_default() {
        assert_eq!(builtin(&[("QAMILLE_LLM_TIMEOUT_SECS", "30")]).unwrap().llm.timeout_secs, 30);
        let dir = TempDir::new();
        let file = dir.path().join("server.toml");
        std::fs::write(&file, "llm_provider = \"builtin\"\nllm_builtin_model = \"llm-medio\"\nllm_timeout_secs = 45\n").unwrap();
        assert_eq!(Config::load_with(Some(&file), env_of(&[])).unwrap().llm.timeout_secs, 45);
    }

    #[test]
    fn the_mirror_url_is_validated_and_empty_means_none() {
        let mirror = |value: &str| llm_load(&[("QAMILLE_MODELS_MIRROR_URL", value)]);
        assert!(mirror("https://mirror.example.com/base").unwrap().models_mirror().is_some());
        assert!(mirror("http://mirror:8080").unwrap().models_mirror().is_some(), "http to a name without dots");
        assert_eq!(invalid_key(mirror("http://mirror.example.com")), "models_mirror_url");
        assert_eq!(invalid_key(mirror("https://user:pw@mirror.example.com")), "models_mirror_url");
        assert_eq!(invalid_key(mirror("https://mirror.example.com/?a=b")), "models_mirror_url");
        let empty = mirror("").unwrap();
        assert!(empty.models_mirror_url.is_none() && empty.models_mirror().is_none());
    }

    #[test]
    fn the_hf_token_comes_only_from_the_environment() {
        assert!(llm_load(&[]).unwrap().hf_token.is_none());
        assert!(llm_load(&[("QAMILLE_HF_TOKEN", "hf_abc")]).unwrap().hf_token.is_some());
        assert!(llm_load(&[("QAMILLE_HF_TOKEN", "")]).unwrap().hf_token.is_none(), "empty is ignored");
        let dir = TempDir::new();
        let file = dir.path().join("server.toml");
        std::fs::write(&file, "hf_token = \"hf_nel_file\"\n").unwrap();
        let error = Config::load_with(Some(&file), env_of(&[])).unwrap_err();
        assert!(!error.to_string().contains("hf_nel_file"), "{error}");
    }

    #[test]
    fn the_debug_of_the_configuration_hides_the_hf_token() {
        let config = llm_load(&[("QAMILLE_HF_TOKEN", "hf_valore_segreto_9911")]).unwrap();
        for text in [format!("{config:?}"), format!("{config:#?}")] {
            assert!(!text.contains("hf_valore_segreto_9911"), "{text}");
            assert!(text.contains("HfToken(<nascosto>)"));
        }
    }

    #[test]
    fn the_license_acknowledgement_is_read_from_the_environment_and_the_file() {
        assert_eq!(builtin(&[]).unwrap().llm.accept_license, None, "not accepted by default");
        assert_eq!(builtin(&[("QAMILLE_LLM_ACCEPT_LICENSE", " gemma ")]).unwrap().llm.accept_license.as_deref(), Some("gemma"));
        assert_eq!(builtin(&[("QAMILLE_LLM_ACCEPT_LICENSE", "")]).unwrap().llm.accept_license, None);
        let dir = TempDir::new();
        let file = dir.path().join("server.toml");
        std::fs::write(&file, "llm_provider = \"builtin\"
llm_builtin_model = \"llm-piccolo\"
llm_accept_license = \"gemma\"
").unwrap();
        let config = Config::load_with(Some(&file), env_of(&[])).unwrap();
        assert_eq!(config.llm.accept_license.as_deref(), Some("gemma"));
    }

    #[test]
    fn the_builtin_program_is_kept_as_a_path_and_not_searched() {
        let program = absolute("my-llama-server");
        let config = builtin(&[("QAMILLE_LLM_BUILTIN_PROGRAM", program.to_str().unwrap())]).unwrap();
        assert_eq!(config.llm.builtin_program, Some(program));
        assert_eq!(builtin(&[]).unwrap().llm.builtin_program, None, "no default: the fixed path is the supervisor's");
    }

}

#[test]
fn device_inactive_days_comes_from_the_file_and_the_environment() {
    let dir = TempDir::new();
    let data = dir.path().to_str().unwrap().to_owned();
    let file = dir.path().join("custom.toml");
    std::fs::write(&file, "device_inactive_days = 30
").unwrap();
    let config = Config::load_with(Some(&file), env_of(&[("QAMILLE_DATA_DIR", &data)])).unwrap();
    assert_eq!(config.device_inactive_days, 30);
    let config = Config::load_with(
        Some(&file),
        env_of(&[("QAMILLE_DATA_DIR", &data), ("QAMILLE_DEVICE_INACTIVE_DAYS", "0")]),
    )
    .unwrap();
    assert_eq!(config.device_inactive_days, 0);
}

#[test]
fn an_empty_trusted_proxies_list_is_announced_at_startup() {
    let mut config = Config::with_data_dir("x");
    // Public bind, no proxy: every request behind a gateway would share one origin.
    config.bind = "0.0.0.0:8787".parse().unwrap();
    let notice = config.origin_notice().expect("notice");
    assert!(notice.contains("trusted_proxies") && notice.contains("proxy o un gateway"), "{notice}");
    assert!(!notice.contains('\n'));
    // Loopback bind, no proxy: a local proxy would make every request come from 127.0.0.1.
    config.bind = "127.0.0.1:8787".parse().unwrap();
    let notice = config.origin_notice().expect("notice");
    assert!(notice.contains("127.0.0.1"), "{notice}");
    // With a trusted proxy there is nothing to say.
    config.trusted_proxies = vec!["10.0.0.2".parse().unwrap()];
    assert!(config.origin_notice().is_none());
}

#[test]
fn guessable_admin_keys_are_rejected_and_generated_ones_accepted() {
    for weak in [
        // Eight different characters, but a block of four repeated.
        "abcdefghabcdefghabcdefghabcdefgh",
        // A run of the same character.
        "kJ7xQ2mZ9wR4tY8uPaaaaaaaaaaaaaaa",
        // One character is more than a third of the key.
        "0a0b0c0d0e0f0g0h0i0j0k0l0m0n0o0p0",
        // Few different characters, low entropy.
        "aaaabbbbaaaabbbbccccddddeeeeffffaaaabbbb",
        "1212121212121212121212121212121212",
    ] {
        assert!(AdminKey::new(weak).is_err(), "{weak}");
    }
    for generated in [
        "f3a9c1d7e5b2048f6a1c3e5d7b9f0a2c4e6d8b1a3c5e7f9d0b2a4c6e8f1a3b5d",
        "9fB2xQ7LmZp0RtYw3KdVn8CsHe5UaJ1o",
        "0123456789abcdef0123456789abcdef",
        "c0ffee15a9b8d7e6f5a4b3c2d1e0f9a8",
    ] {
        assert!(AdminKey::new(generated).is_ok(), "{generated}");
    }
}
