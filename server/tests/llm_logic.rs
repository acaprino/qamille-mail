//! Logica pura del gateway LLM: validazione della richiesta, filtro della risposta, traduzione
//! degli errori del provider. Nessuna rete e nessun database.
use qamille_server::compute::ComputeError;
use qamille_server::llm::{
    LlmConfig, LlmError, LlmProvider, MAX_ERROR_BODY_BYTES, MAX_RESPONSE_BYTES, ROUTE_EXTRA_SECS,
    UNAVAILABLE_RETRY_S, classify_status, estimate_tokens, filter_response, user_hash,
    validate_request,
};
use serde_json::{Value, json};

fn config_with(base: &str, model: &str, allowed: &[&str]) -> LlmConfig {
    LlmConfig {
        provider: LlmProvider::OpenAiCompatible,
        base_url: Some(base.to_owned()),
        model: Some(model.to_owned()),
        allowed_models: allowed.iter().map(|m| (*m).to_owned()).collect(),
        ..LlmConfig::default()
    }
}

fn internal() -> LlmConfig {
    config_with("http://ollama:11434/v1", "default-model", &[])
}

fn run(config: &LlmConfig, body: Value) -> Result<Value, LlmError> {
    let out = validate_request(config, &serde_json::to_vec(&body).unwrap(), "user-1")?;
    Ok(serde_json::from_slice(&out.body).unwrap())
}

fn refused(config: &LlmConfig, body: Value) -> bool {
    matches!(run(config, body), Err(LlmError::InvalidRequest(_)))
}

fn hello() -> Value {
    json!({"messages": [{"role": "user", "content": "ciao"}]})
}

fn with(extra: Value) -> Value {
    let mut body = hello();
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    body
}

// --- validate_request ---

#[test]
fn the_default_model_is_used_when_none_is_asked() {
    let config = internal();
    let out = validate_request(&config, &serde_json::to_vec(&hello()).unwrap(), "u").unwrap();
    assert_eq!(out.model, "default-model");
    let body: Value = serde_json::from_slice(&out.body).unwrap();
    assert_eq!(body["model"], "default-model");
    assert_eq!(out.prompt_chars, 4);
    // Un `model` nullo vale come assente.
    assert_eq!(run(&config, with(json!({"model": null}))).unwrap()["model"], "default-model");
}

#[test]
fn a_model_outside_the_list_is_refused() {
    let config = config_with("http://ollama:11434/v1", "a", &["b"]);
    assert_eq!(run(&config, with(json!({"model": "b"}))).unwrap()["model"], "b");
    assert_eq!(run(&config, with(json!({"model": "a"}))).unwrap()["model"], "a");
    assert!(refused(&config, with(json!({"model": "c"}))));
    assert!(refused(&config, with(json!({"model": 3}))));
}

#[test]
fn only_the_whitelisted_fields_are_forwarded() {
    let body = json!({
        "messages": [{"role": "user", "content": "ciao", "name": "mario", "extra": 1}],
        "logit_bias": {"1": 5}, "metadata": {"a": 1}, "store": true, "web_search_options": {},
        "stream_options": {"include_usage": true}, "parallel_tool_calls": true,
        "temperature": 0.5, "seed": 7
    });
    let out = run(&internal(), body).unwrap();
    let mut keys: Vec<&str> = out.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["max_tokens", "messages", "model", "seed", "temperature", "user"]);
    assert_eq!(out["messages"], json!([{"role": "user", "content": "ciao"}]));
}

#[test]
fn stream_tools_functions_and_n_above_one_are_refused() {
    let config = internal();
    for extra in [
        json!({"stream": true}),
        json!({"tools": []}),
        json!({"functions": []}),
        json!({"tool_choice": "auto"}),
        json!({"function_call": "auto"}),
        json!({"n": 2}),
    ] {
        assert!(refused(&config, with(extra.clone())), "{extra}");
    }
    assert!(run(&config, with(json!({"stream": false}))).is_ok());
    assert!(run(&config, with(json!({"n": 1}))).is_ok());
}

#[test]
fn images_audio_and_other_roles_are_refused() {
    let config = internal();
    let image = json!({"messages": [{"role": "user", "content": [{"type": "image_url", "image_url": {"url": "data:x"}}]}]});
    let audio = json!({"messages": [{"role": "user", "content": [{"type": "input_audio", "input_audio": {}}]}]});
    let tool = json!({"messages": [{"role": "tool", "content": "x"}]});
    let developer = json!({"messages": [{"role": "developer", "content": "x"}]});
    for body in [image, audio, tool, developer] {
        assert!(refused(&config, body.clone()), "{body}");
    }
    for role in ["system", "user", "assistant"] {
        assert!(run(&config, json!({"messages": [{"role": role, "content": "x"}]})).is_ok(), "{role}");
    }
}

#[test]
fn text_parts_become_one_string() {
    let body = json!({"messages": [{"role": "user", "content": [
        {"type": "text", "text": "a"}, {"type": "text", "text": "b"}
    ]}]});
    let out = run(&internal(), body).unwrap();
    assert_eq!(out["messages"][0]["content"], "a\nb");
}

#[test]
fn max_tokens_is_cut_to_the_cap_and_defaults_to_it() {
    let mut config = internal();
    config.max_output_tokens = 4096;
    assert_eq!(run(&config, with(json!({"max_tokens": 99999}))).unwrap()["max_tokens"], 4096);
    assert_eq!(run(&config, with(json!({"max_tokens": 100}))).unwrap()["max_tokens"], 100);
    assert_eq!(run(&config, hello()).unwrap()["max_tokens"], 4096);
    assert_eq!(run(&config, with(json!({"max_completion_tokens": 50}))).unwrap()["max_tokens"], 50);
    assert_eq!(run(&config, with(json!({"max_completion_tokens": 99999}))).unwrap()["max_tokens"], 4096);
    assert!(refused(&config, with(json!({"max_tokens": 0}))));
    assert!(refused(&config, with(json!({"max_tokens": -5}))));
    assert!(refused(&config, with(json!({"max_tokens": "10"}))));
}

#[test]
fn the_ceiling_field_follows_the_provider_family() {
    let internal = run(&internal(), hello()).unwrap();
    assert!(internal.get("max_tokens").is_some() && internal.get("max_completion_tokens").is_none());
    for base in ["https://api.openai.com/v1", "https://x.openai.azure.com/openai/v1"] {
        let out = run(&config_with(base, "m", &[]), with(json!({"max_tokens": 10}))).unwrap();
        assert_eq!(out["max_completion_tokens"], 10, "{base}");
        assert!(out.get("max_tokens").is_none(), "{base}");
    }
    let other = run(&config_with("https://llm.example.com/v1", "m", &[]), hello()).unwrap();
    assert!(other.get("max_tokens").is_some());
}

#[test]
fn user_is_the_sha256_of_the_user_id() {
    let id = "7f3c-secret-internal-id";
    let out = validate_request(
        &internal(),
        &serde_json::to_vec(&with(json!({"user": "mario@example.com"}))).unwrap(),
        id,
    )
    .unwrap();
    let body: Value = serde_json::from_slice(&out.body).unwrap();
    let user = body["user"].as_str().unwrap();
    assert_eq!(user, user_hash(id));
    assert_eq!(user.len(), 64);
    assert!(user.bytes().all(|b| b.is_ascii_hexdigit()));
    assert!(!String::from_utf8_lossy(&out.body).contains(id));
    assert!(!String::from_utf8_lossy(&out.body).contains("mario@example.com"));
    // SHA-256 noto.
    assert_eq!(user_hash("abc"), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
}

#[test]
fn response_format_stop_seed_temperature_and_top_p_are_checked() {
    let config = internal();
    for bad in [
        json!({"response_format": {"type": "xml"}}),
        json!({"response_format": "json"}),
        json!({"stop": ["a", "b", "c", "d", "e"]}),
        json!({"stop": [1]}),
        json!({"seed": 1.5}),
        json!({"seed": "1"}),
        json!({"temperature": 3}),
        json!({"temperature": -0.1}),
        json!({"temperature": "hot"}),
        json!({"top_p": 2}),
    ] {
        assert!(refused(&config, with(bad.clone())), "{bad}");
    }
    let ok = run(
        &config,
        with(json!({
            "response_format": {"type": "json_schema", "json_schema": {"name": "x", "schema": {"type": "object"}}},
            "stop": ["a", "b", "c", "d"], "seed": 4, "temperature": 2, "top_p": 1
        })),
    )
    .unwrap();
    assert_eq!(ok["response_format"]["type"], "json_schema");
    assert_eq!(ok["response_format"]["json_schema"]["name"], "x");
    assert_eq!(ok["stop"], json!(["a", "b", "c", "d"]));
    assert_eq!(ok["seed"], 4);
    assert_eq!(run(&config, with(json!({"stop": "fine"}))).unwrap()["stop"], "fine");
    assert_eq!(run(&config, with(json!({"response_format": {"type": "text"}}))).unwrap()["response_format"], json!({"type": "text"}));
    assert!(run(&config, with(json!({"response_format": {"type": "json_object", "extra": 1}}))).unwrap()["response_format"].get("extra").is_none());
}

#[test]
fn a_malformed_body_is_invalid_request_with_a_fixed_message() {
    let config = internal();
    let secret = "SEGRETO-NEL-CORPO";
    let bodies: Vec<Vec<u8>> = vec![
        format!("not json {secret}").into_bytes(),
        format!("[\"{secret}\"]").into_bytes(),
        format!("{{\"model\": \"{secret}\"}}").into_bytes(),
        format!("{{\"messages\": [], \"x\": \"{secret}\"}}").into_bytes(),
        format!("{{\"messages\": \"{secret}\"}}").into_bytes(),
        format!("{{\"messages\": [\"{secret}\"]}}").into_bytes(),
        Vec::new(),
    ];
    for body in bodies {
        match validate_request(&config, &body, "u") {
            Err(error @ LlmError::InvalidRequest(message)) => {
                assert!(!message.is_empty());
                assert!(!format!("{error:?} {error} {message}").contains(secret));
            }
            other => panic!("{other:?}"),
        }
    }
}

// --- filter_response ---

#[test]
fn the_answer_keeps_only_the_whitelisted_fields() {
    let raw = json!({
        "id": "chatcmpl-1", "object": "chat.completion", "created": 1700000000, "model": "m-1",
        "system_fingerprint": "fp", "service_tier": "default",
        "choices": [{
            "index": 0, "finish_reason": "stop", "logprobs": {"content": []},
            "message": {"role": "assistant", "content": "ciao", "refusal": null, "tool_calls": [{"id": "x"}], "audio": {}}
        }],
        "usage": {"prompt_tokens": 7, "completion_tokens": 2, "total_tokens": 9, "prompt_tokens_details": {"cached_tokens": 1}}
    });
    let done = filter_response(&serde_json::to_vec(&raw).unwrap(), 100).unwrap();
    assert_eq!((done.prompt_tokens, done.completion_tokens), (7, 2));
    let out: Value = serde_json::from_slice(&done.body).unwrap();
    let mut top: Vec<&str> = out.as_object().unwrap().keys().map(String::as_str).collect();
    top.sort();
    assert_eq!(top, ["choices", "created", "id", "model", "object", "usage"]);
    assert_eq!(out["id"], "chatcmpl-1");
    assert_eq!(out["object"], "chat.completion");
    assert_eq!(out["created"], 1700000000u64);
    assert_eq!(out["model"], "m-1");
    let choice = &out["choices"][0];
    let mut keys: Vec<&str> = choice.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["finish_reason", "index", "message"]);
    assert_eq!(choice["message"], json!({"role": "assistant", "content": "ciao"}));
    assert_eq!(choice["finish_reason"], "stop");
    assert_eq!(out["usage"], json!({"prompt_tokens": 7, "completion_tokens": 2, "total_tokens": 9}));
}

#[test]
fn missing_usage_is_estimated() {
    let raw = json!({"id": "x", "model": "m", "choices": [{"index": 0, "message": {"role": "assistant", "content": "0123456789"}, "finish_reason": "length"}]});
    let done = filter_response(&serde_json::to_vec(&raw).unwrap(), 41).unwrap();
    assert_eq!(done.prompt_tokens, estimate_tokens(41));
    assert_eq!(done.prompt_tokens, 11);
    assert_eq!(done.completion_tokens, 3);
    let out: Value = serde_json::from_slice(&done.body).unwrap();
    assert_eq!(out["usage"], json!({"prompt_tokens": 11, "completion_tokens": 3, "total_tokens": 14}));
    assert_eq!(estimate_tokens(0), 0);
    assert_eq!(estimate_tokens(1), 1);
    assert_eq!(estimate_tokens(4), 1);
    assert_eq!(estimate_tokens(5), 2);
}

#[test]
fn content_that_is_not_a_string_becomes_empty_text() {
    for content in [json!(null), json!(["x"]), json!(5)] {
        let raw = json!({"choices": [{"message": {"role": "assistant", "content": content}}]});
        let done = filter_response(&serde_json::to_vec(&raw).unwrap(), 4).unwrap();
        let out: Value = serde_json::from_slice(&done.body).unwrap();
        assert_eq!(out["choices"][0]["message"]["content"], "");
        assert_eq!(out["choices"][0]["finish_reason"], Value::Null);
        assert_eq!(out["choices"][0]["index"], 0);
    }
}

#[test]
fn a_malformed_answer_is_unavailable() {
    for raw in [
        b"<html>nope</html>".to_vec(),
        b"[]".to_vec(),
        br#"{"id": "x"}"#.to_vec(),
        br#"{"choices": []}"#.to_vec(),
        br#"{"choices": "x"}"#.to_vec(),
        br#"{"choices": [5]}"#.to_vec(),
    ] {
        match filter_response(&raw, 4) {
            Err(LlmError::Unavailable { retry_after_s }) => assert_eq!(retry_after_s, UNAVAILABLE_RETRY_S),
            other => panic!("{:?}", other.map(|c| c.completion_tokens)),
        }
    }
}

// --- classify_status ---

#[test]
fn provider_statuses_are_translated() {
    for status in [500, 502, 503, 401, 403, 402, 404, 422, 302] {
        assert!(matches!(classify_status(status, None, b""), LlmError::Unavailable { retry_after_s: 30 }), "{status}");
    }
    assert!(matches!(classify_status(429, Some(7), b""), LlmError::RateLimited { retry_after_s: 7 }));
    assert!(matches!(classify_status(429, None, b""), LlmError::RateLimited { retry_after_s: 30 }));
    assert!(matches!(classify_status(429, Some(99999), b""), LlmError::RateLimited { retry_after_s: 3600 }));
    assert!(matches!(classify_status(429, Some(0), b""), LlmError::RateLimited { retry_after_s: 1 }));
    assert!(matches!(classify_status(400, None, b"{}"), LlmError::InvalidRequest(_)));
    assert!(matches!(classify_status(400, None, b"not json"), LlmError::InvalidRequest(_)));
    for body in [
        json!({"error": {"code": "context_length_exceeded", "message": "x"}}),
        json!({"error": {"type": "exceed_context_size_error", "message": "x"}}),
        json!({"error": {"message": "This model's Maximum Context Length is 8192 tokens"}}),
        json!({"error": {"message": "above the context window"}}),
        json!({"error": {"message": "request exceeds the available context length"}}),
    ] {
        assert!(matches!(classify_status(400, None, body.to_string().as_bytes()), LlmError::ContextExceeded), "{body}");
    }
    // Il corpo letto e' limitato: un messaggio oltre il limite non conta.
    let mut big = vec![b' '; MAX_ERROR_BODY_BYTES];
    big.extend_from_slice(br#"{"error": {"code": "context_length_exceeded"}}"#);
    assert!(matches!(classify_status(400, None, &big), LlmError::InvalidRequest(_)));
}

#[test]
fn no_provider_text_is_ever_in_an_error() {
    let body = br#"{"error": {"message": "SEGRETO-DEL-PROVIDER", "code": "context_length_exceeded"}}"#;
    let plain = br#"{"error": {"message": "SEGRETO-DEL-PROVIDER"}}"#;
    for (status, body) in [(400, &body[..]), (400, &plain[..]), (401, &plain[..]), (429, &plain[..]), (500, &plain[..])] {
        let error = classify_status(status, Some(5), body);
        let shown = format!("{error:?} | {error}");
        assert!(!shown.contains("SEGRETO-DEL-PROVIDER"), "{shown}");
        let compute: ComputeError = error.into();
        let shown = format!("{compute:?} | {} | {}", compute.message(), compute.code());
        assert!(!shown.contains("SEGRETO-DEL-PROVIDER"), "{shown}");
    }
}

#[test]
fn llm_errors_become_compute_errors() {
    use axum::http::StatusCode;
    let invalid: ComputeError = LlmError::InvalidRequest("bad").into();
    assert_eq!(invalid, ComputeError::InvalidRequest("bad".to_owned()));
    let context: ComputeError = LlmError::ContextExceeded.into();
    assert_eq!((context.code(), context.status()), ("llm_context_exceeded", StatusCode::BAD_REQUEST));
    let unavailable: ComputeError = LlmError::Unavailable { retry_after_s: 30 }.into();
    assert_eq!((unavailable.code(), unavailable.status(), unavailable.retry_after_s()), ("llm_unavailable", StatusCode::SERVICE_UNAVAILABLE, Some(30)));
    let limited: ComputeError = LlmError::RateLimited { retry_after_s: 9 }.into();
    assert_eq!((limited.code(), limited.status(), limited.retry_after_s()), ("rate_limited", StatusCode::TOO_MANY_REQUESTS, Some(9)));
    let consent = ComputeError::LlmConsentRequired;
    assert_eq!((consent.code(), consent.status()), ("llm_consent_required", StatusCode::CONFLICT));
    assert_eq!(MAX_RESPONSE_BYTES, 4 * 1024 * 1024);
    assert_eq!(ROUTE_EXTRA_SECS, 10);
}

#[test]
fn hostile_declared_usage_is_replaced_by_the_estimate() {
    let raw = |p: u64, c: u64| {
        serde_json::to_vec(&json!({
            "id": "x", "model": "m",
            "choices": [{"index": 0, "message": {"role": "assistant", "content": "abcdefgh"}, "finish_reason": "stop"}],
            "usage": {"prompt_tokens": p, "completion_tokens": c},
        }))
        .unwrap()
    };
    // 9e18 per campo: sopra il limite per chiamata, vale la stima (40 caratteri -> 10; 8 -> 2).
    let hostile = filter_response(&raw(9_000_000_000_000_000_000, 9_000_000_000_000_000_000), 40).unwrap();
    assert_eq!((hostile.prompt_tokens, hostile.completion_tokens), (10, 2));
    // Anche un solo campo enorme.
    let one = filter_response(&raw(1, u64::MAX), 40).unwrap();
    assert_eq!((one.prompt_tokens, one.completion_tokens), (10, 2));
    // Al limite (10_000_000 in tutto) il dichiarato vale ancora; un token oltre, la stima.
    let edge = filter_response(&raw(6_000_000, 4_000_000), 40).unwrap();
    assert_eq!((edge.prompt_tokens, edge.completion_tokens), (6_000_000, 4_000_000));
    let over = filter_response(&raw(6_000_000, 4_000_001), 40).unwrap();
    assert_eq!((over.prompt_tokens, over.completion_tokens), (10, 2));
}

#[test]
fn a_label_cannot_start_or_end_with_a_space() {
    let with_label = |label: &str| LlmConfig { label: Some(label.to_owned()), ..internal() }.validate();
    assert!(with_label("Acme AI").is_ok());
    for bad in [" Acme", "Acme ", " ", "  Acme  "] {
        assert!(with_label(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn the_provider_key_must_be_visible_ascii_and_the_error_does_not_print_it() {
    use qamille_server::llm::ApiKey;
    let with_key = |key: &str| LlmConfig { api_key: ApiKey::new(key), ..internal() }.validate();
    assert!(with_key("sk-proj-AbC_123.xyz").is_ok());
    for bad in ["sk-abc def", "sk-abc\ndef", "sk-a\rbc", "sk-abc\tdef", "sk-societ\u{e0}", "sk-abc\u{7f}def", "sk-abc\0def"] {
        let error = with_key(bad).expect_err(bad).to_string();
        assert!(error.contains("QAMILLE_LLM_API_KEY"), "{error}");
        assert!(!error.contains("sk-"), "{error}");
    }
}

#[test]
fn a_json_schema_keeps_only_name_strict_and_schema_within_64_kb() {
    let config = internal();
    let body = |schema: Value| json!({
        "messages": [{"role": "user", "content": "x"}],
        "response_format": {"type": "json_schema", "json_schema": schema},
    });
    let ok = run(
        &config,
        body(json!({"name": "n", "strict": true, "schema": {"type": "object"}, "description": "d", "extra": {"a": 1}})),
    )
    .unwrap();
    assert_eq!(
        ok["response_format"]["json_schema"],
        json!({"name": "n", "strict": true, "schema": {"type": "object"}})
    );
    // Tipi sbagliati.
    assert!(refused(&config, body(json!({"name": 3, "schema": {}}))));
    assert!(refused(&config, body(json!({"name": "n", "strict": "yes", "schema": {}}))));
    assert!(refused(&config, body(json!({"name": "n", "schema": "no"}))));
    // Fino a 64 KB si passa, oltre no.
    let sized = |bytes: usize| json!({"name": "n", "schema": {"description": "a".repeat(bytes)}});
    assert!(run(&config, body(sized(60 * 1024))).is_ok());
    assert!(refused(&config, body(sized(65 * 1024))));
}
