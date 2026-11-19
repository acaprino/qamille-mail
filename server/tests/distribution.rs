//! The Docker distribution files: the image, the compose for one's own PC, the image workflow.
//! They are read as text (no Docker needed); the real build is a manual check.

fn read(path: &str) -> String {
    let full = format!("{}/../{path}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&full).unwrap_or_else(|e| panic!("{full}: {e}"))
}

/// The listed files, without the enterprise ones when this checkout has no `enterprise/` (the public repository).
fn present<'a>(files: &[&'a str]) -> Vec<&'a str> {
    let has_enterprise = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../enterprise").is_dir();
    files.iter().copied().filter(|f| has_enterprise || !f.starts_with("enterprise/")).collect()
}

/// The value after `prefix` when it is exactly `len` lowercase hex digits.
fn hex_arg(text: &str, prefix: &str, len: usize) -> bool {
    text.lines().any(|l| {
        l.trim_start().strip_prefix(prefix).is_some_and(|v| v.len() == len && v.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')))
    })
}

#[test]
fn the_dockerfile_pins_whisper_by_commit_and_hash() {
    let d = read("server/Dockerfile");
    assert!(hex_arg(&d, "ARG WHISPER_COMMIT=", 40), "WHISPER_COMMIT must be 40 hex digits");
    assert!(hex_arg(&d, "ARG WHISPER_SHA256=", 64), "WHISPER_SHA256 must be 64 hex digits");
    assert!(d.contains("sha256sum -c") && d.contains("-DGGML_NATIVE=OFF") && d.contains("/usr/local/bin/whisper-cli"));
    assert!(d.contains("/usr/share/licenses/whisper.cpp"));
    assert!(d.contains("COPY --from=whisper"));
}

#[test]
fn the_whisper_release_in_the_dockerfile_matches_the_compute_crate() {
    assert!(read("server/Dockerfile").contains(&format!("ARG WHISPER_RELEASE={}", qamille_compute::speech::WHISPER_RELEASE)));
}

#[test]
fn compose_local_publishes_only_on_loopback() {
    let c = read("server/compose.local.yaml");
    let ports: Vec<&str> = c
        .lines()
        .skip_while(|l| l.trim() != "ports:")
        .skip(1)
        .take_while(|l| l.trim_start().starts_with('-'))
        .collect();
    assert_eq!(ports.len(), 2);
    assert!(ports.iter().all(|p| p.trim_start_matches(['-', ' ', '"']).starts_with("127.0.0.1:")), "{ports:?}");
    // the only 0.0.0.0 is the listener inside the container
    for l in c.lines().filter(|l| !l.trim_start().starts_with('#') && l.contains("0.0.0.0")) {
        assert!(l.trim() == "QAMILLE_BIND: 0.0.0.0:8787", "{l}");
    }
    assert!(!c.to_lowercase().contains("caddy:") && !c.contains("image: caddy"));
}

#[test]
fn compose_local_allows_plaintext_only_from_the_gateway_of_its_own_network() {
    let c = read("server/compose.local.yaml");
    assert!(c.contains("QAMILLE_PLAINTEXT_FROM: 172.30.88.1") && c.contains("QAMILLE_ADMIN_PLAINTEXT_FROM: 172.30.88.1") && c.contains("gateway: 172.30.88.1"));
}

#[test]
fn compose_local_pins_the_image_by_digest() {
    let c = read("server/compose.local.yaml");
    let digest = c.lines().find_map(|l| l.trim().strip_prefix("image: ghcr.io/acaprino/qamille-server@sha256:")).expect("image line");
    assert!(digest.len() == 64 && digest.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')));
}

#[test]
fn the_docker_image_workflow_follows_the_release_and_can_be_run_by_hand() {
    let w = read(".github/workflows/docker-image.yml");
    for needle in ["workflow_run", "workflows: [Release]", "workflow_dispatch", "packages: write", "linux/amd64", "ghcr.io/acaprino/qamille-server", "server/Dockerfile"] {
        assert!(w.contains(needle), "{needle}");
    }
}

#[test]
fn every_action_in_the_image_workflow_is_pinned_by_commit() {
    let w = read(".github/workflows/docker-image.yml");
    let uses: Vec<&str> = w.lines().filter(|l| l.trim_start().trim_start_matches("- ").starts_with("uses:")).collect();
    assert!(uses.len() >= 5, "{uses:?}");
    for l in uses {
        let spec = l.split("uses:").nth(1).unwrap().split('#').next().unwrap().trim();
        let sha = spec.split('@').nth(1).unwrap_or("");
        assert!(sha.len() == 40 && sha.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')), "not pinned: {l}");
    }
    assert!(w.contains("concurrency:") && w.contains("group: docker-image") && w.contains("cancel-in-progress: false"));
}

#[test]
fn plaintext_from_is_documented_as_loopback_only() {
    for path in ["server/README.md", "docs/server-sul-pc.md"] {
        assert!(read(path).contains("`plaintext_from` è solo per una porta pubblicata su 127.0.0.1"), "{path}");
    }
}

const LLM_KEY_LINE: &str = "QAMILLE_LLM_API_KEY: ${QAMILLE_LLM_API_KEY:-}";

/// Every `QAMILLE_LLM_<NAME>` the configuration reads from the environment.
fn llm_variables() -> Vec<String> {
    let config = read("server/src/config.rs");
    let mut names = Vec::new();
    for (at, _) in config.match_indices("QAMILLE_LLM_") {
        let name: String = config[at..].chars().take_while(|c| c.is_ascii_uppercase() || *c == '_').collect();
        if name.len() > "QAMILLE_LLM_".len() && !name.ends_with('_') && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

#[test]
fn every_llm_variable_the_config_reads_is_in_the_env_example_and_the_readme() {
    let names = llm_variables();
    assert!(names.len() >= 13, "{names:?}");
    for file in present(&["server/.env.example", "server/README.md", "server/compose.yaml", "enterprise/server/.env.example", "enterprise/server/README.md", "enterprise/server/compose.yaml"]) {
        let text = read(file);
        for name in &names {
            assert!(text.contains(name.as_str()), "{file} does not mention {name}");
        }
    }
}

#[test]
fn the_compose_files_never_set_the_provider_key_to_a_literal() {
    for file in present(&["server/compose.yaml", "server/compose.local.yaml", "enterprise/server/compose.yaml"]) {
        let mut set = 0;
        for line in read(file).lines().map(str::trim).filter(|l| !l.starts_with('#') && l.contains("QAMILLE_LLM_API_KEY")) {
            assert_eq!(line, LLM_KEY_LINE, "{file}: the provider key is only a reference to the environment");
            set += 1;
        }
        if file != "server/compose.local.yaml" {
            assert_eq!(set, 1, "{file}");
        }
    }
}

#[test]
fn the_env_examples_have_an_empty_provider_key_and_both_providers_as_examples() {
    for file in present(&["server/.env.example", "enterprise/server/.env.example"]) {
        let text = read(file);
        assert!(text.lines().any(|l| l == "QAMILLE_LLM_API_KEY="), "{file}: an empty key");
        // The other variables are examples: commented out, so the assistant stays off by default.
        for line in text.lines().filter(|l| l.contains("QAMILLE_LLM_") && !l.starts_with('#')) {
            assert_eq!(line, "QAMILLE_LLM_API_KEY=", "{file}: {line}");
        }
        for needle in ["http://ollama:11434/v1", "qwen2.5:14b", "https://api.openai.com/v1", "gpt-4.1-mini", "QAMILLE_LLM_MAX_TOKENS_PER_DAY_PER_USER"] {
            assert!(text.contains(needle), "{file}: {needle}");
        }
    }
}

#[test]
fn the_readme_says_the_llm_migration_is_one_way() {
    let text = read("server/README.md");
    let line = text
        .lines()
        .find(|l| l.contains("core-003-llm-usage") && l.contains("a senso unico"))
        .expect("the README says that core-003-llm-usage is one-way");
    assert!(line.contains("binario precedente"), "{line}");
    assert!(line.contains("rifiuta") && line.contains("copia"), "{line}");
}

// --- The built-in assistant: llama.cpp in both images, the compose files, the examples (Task 13) ---

/// The text of the stage `AS <name>`, from its `FROM` to the line before the next `FROM`.
fn stage(dockerfile: &str, name: &str) -> String {
    let lines: Vec<&str> = dockerfile.lines().collect();
    let start = lines.iter().position(|l| l.starts_with("FROM ") && l.ends_with(&format!(" AS {name}"))).unwrap_or_else(|| panic!("no stage {name}"));
    let end = lines[start + 1..].iter().position(|l| l.starts_with("FROM ")).map_or(lines.len(), |n| start + 1 + n);
    lines[start..end].join("\n")
}

#[test]
fn the_dockerfile_pins_llama_by_commit_and_hash() {
    let d = read("server/Dockerfile");
    assert!(hex_arg(&d, "ARG LLAMA_COMMIT=", 40), "LLAMA_COMMIT must be 40 hex digits");
    assert!(hex_arg(&d, "ARG LLAMA_SHA256=", 64), "LLAMA_SHA256 must be 64 hex digits");
    let llama = stage(&d, "llama");
    // The commit is fetched by its id and checked; nothing is built from a branch or a tag.
    assert!(llama.contains("git fetch -q --depth 1 origin \"${LLAMA_COMMIT}\""), "{llama}");
    assert!(llama.contains("test \"$(git rev-parse HEAD)\" = \"${LLAMA_COMMIT}\""), "{llama}");
    assert!(!llama.contains("--branch") && !llama.contains("git clone") && !llama.contains("master") && !llama.contains("main"), "{llama}");
    assert!(llama.contains("-DGGML_NATIVE=OFF") && llama.contains("-DGGML_OPENMP=OFF") && llama.contains("-DBUILD_SHARED_LIBS=OFF"));
    // Nothing is downloaded while building and no HTTPS client is linked: the server downloads nothing itself.
    assert!(llama.contains("-DLLAMA_USE_PREBUILT_UI=OFF") && llama.contains("-DLLAMA_OPENSSL=OFF"));
    assert!(llama.contains("--target llama-server") && llama.contains("-j 2"));
    assert!(llama.contains("/usr/local/bin/llama-server") && llama.contains("/usr/share/licenses/llama.cpp"));
    assert!(d.contains("COPY --from=llama /usr/local/bin/llama-server /usr/local/bin/llama-server"));
    assert!(d.contains("COPY --from=llama /usr/share/licenses/llama.cpp/LICENSE"));
}

#[test]
fn the_llama_pins_in_the_dockerfile_match_the_compute_crate() {
    for file in present(&["server/Dockerfile", "enterprise/server/Dockerfile"]) {
        let d = read(file);
        assert!(d.contains(&format!("ARG LLAMA_TAG={}\n", qamille_compute::llama::LLAMA_TAG)), "{file}");
        assert!(d.contains(&format!("ARG LLAMA_COMMIT={}\n", qamille_compute::llama::LLAMA_COMMIT)), "{file}");
        assert!(d.contains(&format!("ARG LLAMA_SHA256={}\n", qamille_compute::llama::SOURCE_SHA256)), "{file}");
    }
}

#[test]
fn the_dockerfile_checks_the_flags_the_supervisor_uses() {
    let llama = stage(&read("server/Dockerfile"), "llama");
    let check = llama.lines().find(|l| l.contains("for flag in")).expect("the loop over the flags");
    for flag in qamille_server::llm_builtin::REQUIRED_FLAGS {
        assert!(check.split_whitespace().any(|word| word.trim_end_matches(';') == *flag), "the build does not check {flag}: {check}");
    }
    assert!(llama.contains("llama-server --help") && llama.contains("exit 1"));
}

#[test]
fn the_enterprise_dockerfile_has_the_same_llama_stage() {
    if present(&["enterprise/server/Dockerfile"]).is_empty() {
        eprintln!("skipped: no enterprise/ in this checkout");
        return;
    }
    assert_eq!(stage(&read("server/Dockerfile"), "llama"), stage(&read("enterprise/server/Dockerfile"), "llama"));
    let e = read("enterprise/server/Dockerfile");
    assert!(e.contains("COPY --from=llama /usr/local/bin/llama-server /usr/local/bin/llama-server"));
    assert!(e.contains("COPY --from=llama /usr/share/licenses/llama.cpp/LICENSE"));
}

#[test]
fn the_image_copies_only_the_server_and_the_engines() {
    for file in present(&["server/Dockerfile", "enterprise/server/Dockerfile"]) {
        let d = read(file);
        assert!(!d.contains("fake-llama") && !d.contains("target/debug"), "{file}");
        for line in d.lines().filter(|l| l.starts_with("COPY --from=")) {
            let from = line.split_whitespace().nth(1).unwrap();
            assert!(["--from=build", "--from=whisper", "--from=llama"].contains(&from), "{file}: {line}");
            if from == "--from=build" {
                assert!(line.contains("/target/release/qamille-server"), "{file}: {line}");
            }
        }
        // Only the real binary is built: the fake program is a test helper.
        for line in d.lines().filter(|l| l.contains("cargo build")) {
            assert!(line.contains("--release") && line.contains("--bin qamille-server"), "{file}: {line}");
        }
    }
}

#[test]
fn the_compose_files_never_set_the_hf_token_to_a_literal() {
    for file in present(&["server/compose.yaml", "server/compose.local.yaml", "enterprise/server/compose.yaml"]) {
        let mut found = 0;
        for line in read(file).lines().map(str::trim).filter(|l| !l.starts_with('#') && l.contains("QAMILLE_HF_TOKEN")) {
            assert_eq!(line, "QAMILLE_HF_TOKEN: ${QAMILLE_HF_TOKEN:-}", "{file}: the token is only a reference to the environment");
            found += 1;
        }
        assert_eq!(found, 1, "{file}");
    }
}

#[test]
fn the_compose_files_have_the_builtin_variables_and_init() {
    for file in present(&["server/compose.yaml", "server/compose.local.yaml", "enterprise/server/compose.yaml"]) {
        let c = read(file);
        for name in ["QAMILLE_LLM_BUILTIN_MODEL", "QAMILLE_LLM_BUILTIN_CONTEXT", "QAMILLE_LLM_BUILTIN_IDLE_SECS", "QAMILLE_LLM_BUILTIN_SLOTS", "QAMILLE_MODELS_MIRROR_URL"] {
            assert!(c.lines().any(|l| l.trim() == format!("{name}: ${{{name}:-}}")), "{file}: {name}");
        }
        // The child of the assistant leaves no zombie: PID 1 is an init.
        assert!(c.lines().any(|l| l.trim() == "init: true"), "{file}");
        assert!(c.contains("mem_limit") && c.contains("QAMILLE_LLM_PROVIDER=builtin") && c.contains("llm-piccolo"), "{file}: the memory example");
    }
    // With the provider empty nothing changes: the variable is only passed through.
    assert!(read("server/compose.local.yaml").lines().any(|l| l.trim() == "QAMILLE_LLM_PROVIDER: ${QAMILLE_LLM_PROVIDER:-}"));
}

#[test]
fn the_env_examples_have_an_empty_hf_token_and_the_builtin_example() {
    for file in present(&["server/.env.example", "enterprise/server/.env.example", "server/.env.local.example"]) {
        let text = read(file);
        assert_eq!(text.lines().filter(|l| l.starts_with("QAMILLE_HF_TOKEN")).collect::<Vec<_>>(), ["QAMILLE_HF_TOKEN="], "{file}");
        assert!(text.contains("# QAMILLE_LLM_PROVIDER=builtin") && text.contains("QAMILLE_LLM_BUILTIN_MODEL=llm-piccolo"), "{file}");
    }
}

#[test]
fn compose_local_still_publishes_only_on_loopback() {
    let c = read("server/compose.local.yaml");
    for line in c.lines().filter(|l| !l.trim_start().starts_with('#') && l.trim_start().starts_with("- \"")) {
        assert!(line.trim_start().starts_with("- \"127.0.0.1:"), "{line}");
    }
    assert!(!c.contains("0.0.0.0:8788") && !c.contains("\"8787:8787\""));
}

// --- Documents of the built-in assistant (Task 14) -----------------------------------------

const BUILTIN_NAMES: [&str; 14] = [
    "llm_builtin_model",
    "llm_builtin_context",
    "llm_builtin_idle_secs",
    "llm_builtin_slots",
    "llm_builtin_program",
    "llm_accept_license",
    "models_mirror_url",
    "QAMILLE_HF_TOKEN",
    "QAMILLE_LLM_BUILTIN_MODEL",
    "QAMILLE_LLM_BUILTIN_CONTEXT",
    "QAMILLE_LLM_BUILTIN_IDLE_SECS",
    "QAMILLE_LLM_BUILTIN_SLOTS",
    "QAMILLE_LLM_ACCEPT_LICENSE",
    "QAMILLE_MODELS_MIRROR_URL",
];

#[test]
fn the_readme_names_every_builtin_key_and_variable() {
    for file in present(&["server/README.md", "enterprise/server/README.md"]) {
        let text = read(file);
        for name in BUILTIN_NAMES.iter().chain(["QAMILLE_LLM_BUILTIN_PROGRAM"].iter()) {
            assert!(text.contains(name), "{file} does not mention {name}");
        }
    }
    let core = read("server/README.md");
    assert!(core.contains("### Modelli predefiniti"), "the section of the built-in models");
    // Every model of the catalog, with its disk and its memory, as the page /admin shows them.
    for spec in qamille_compute::llm_catalog::CATALOG {
        assert!(core.contains(spec.id), "{}", spec.id);
        let disk = format!("{:.1}", qamille_compute::llm_catalog::required_disk_bytes(spec) as f64 / 1e9).replace('.', ",");
        let memory = format!("{:.1}", qamille_compute::llm_catalog::required_ram_mb(spec, 8192) as f64 / 1024.0).replace('.', ",");
        assert!(core.contains(&format!("{disk} GB")) && core.contains(&format!("{memory} GB")), "{}: {disk} GB, {memory} GB", spec.id);
    }
    for command in ["model install", "model remove", "model list"] {
        assert!(core.contains(command), "{command}");
    }
}

#[test]
fn every_builtin_name_is_in_the_env_examples_and_the_server_on_the_pc_guide() {
    for file in present(&["server/.env.example", "enterprise/server/.env.example"]) {
        let text = read(file);
        for name in ["QAMILLE_LLM_BUILTIN_MODEL", "QAMILLE_LLM_BUILTIN_CONTEXT", "QAMILLE_LLM_BUILTIN_IDLE_SECS", "QAMILLE_LLM_BUILTIN_SLOTS", "QAMILLE_MODELS_MIRROR_URL", "QAMILLE_HF_TOKEN"] {
            assert!(text.contains(name), "{file}: {name}");
        }
    }
    let guide = read("docs/server-sul-pc.md");
    for spec in qamille_compute::llm_catalog::CATALOG {
        assert!(guide.contains(spec.id), "docs/server-sul-pc.md does not mention {}", spec.id);
    }
    assert!(guide.contains("mem_limit") || guide.contains("memory="));
}

#[test]
fn the_readme_states_that_gpu_is_not_supported_yet_and_how_to_update_the_engine() {
    let text = read("server/README.md");
    assert!(text.contains("La GPU non è ancora supportata"), "the GPU is not supported yet");
    for needle in ["compute/src/llama.rs", "compute/tools/llama-lookup.mjs", "server/Dockerfile", "LLAMA_COMMIT", "SOURCE_SHA256"] {
        assert!(text.contains(needle), "how to update the engine: {needle}");
    }
    assert!(text.contains("compute/tools/llm-catalog-lookup.mjs") && text.contains("CATALOG"), "how to update a model");
}

#[test]
fn the_spec_lists_the_slots_the_readiness_probe_and_version_5() {
    let spec = read("docs/specs/2026-11-17-gateway-llm-design.md");
    assert!(spec.contains("**Versione:** 5"));
    assert!(spec.lines().any(|l| l.starts_with("| `llm_builtin_slots` |") && l.contains("`1`") && l.contains("scelta mia")));
    assert!(spec.contains("/tokenize"), "the readiness probe");
    for key in ["llm_builtin_context", "llm_builtin_idle_secs"] {
        assert!(spec.lines().any(|l| l.starts_with(&format!("| `{key}` |"))), "{key}");
    }
}

// --- Sincronizzazione delle caselle (wave 6) -----------------------------------------------

/// Every `QAMILLE_SYNC_<NAME>` the configuration reads from the environment.
fn sync_variables() -> Vec<String> {
    let config = read("server/src/config.rs");
    let mut names = Vec::new();
    for (at, _) in config.match_indices("QAMILLE_SYNC_") {
        let name: String = config[at..].chars().take_while(|c| c.is_ascii_uppercase() || *c == '_').collect();
        if name.len() > "QAMILLE_SYNC_".len() && !name.ends_with('_') && !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

#[test]
fn every_sync_variable_the_config_reads_is_in_the_env_examples_the_readmes_and_the_compose_files() {
    let names = sync_variables();
    assert_eq!(names.len(), 4, "{names:?}");
    for file in present(&["server/.env.example", "server/README.md", "enterprise/server/.env.example", "enterprise/server/README.md"]) {
        let text = read(file);
        for name in &names {
            assert!(text.contains(name.as_str()), "{file} does not mention {name}");
        }
    }
    for file in present(&["server/compose.yaml", "server/compose.local.yaml", "enterprise/server/compose.yaml"]) {
        let text = read(file);
        for name in ["QAMILLE_SYNC_KEY", "QAMILLE_SYNC_KEY_OLD"] {
            assert!(text.contains(name), "{file} does not pass {name}");
        }
    }
}

#[test]
fn the_compose_files_never_set_the_sync_key_to_a_literal() {
    for file in present(&["server/compose.yaml", "server/compose.local.yaml", "enterprise/server/compose.yaml"]) {
        let mut seen = 0;
        for line in read(file).lines().map(str::trim).filter(|l| !l.starts_with('#') && l.contains("QAMILLE_SYNC_")) {
            let name = line.split(':').next().unwrap();
            assert_eq!(line, format!("{name}: ${{{name}:-}}"), "{file}: a sync key is only a reference to the environment");
            seen += 1;
        }
        assert_eq!(seen, 2, "{file}");
    }
}

#[test]
fn the_env_examples_have_an_empty_sync_key() {
    for file in present(&["server/.env.example", "enterprise/server/.env.example"]) {
        let text = read(file);
        assert!(text.lines().any(|l| l == "QAMILLE_SYNC_KEY="), "{file}: an empty key");
        for line in text.lines().filter(|l| !l.starts_with('#') && l.contains("QAMILLE_SYNC_")) {
            assert_eq!(line, "QAMILLE_SYNC_KEY=", "{file}: only the empty key is set; the rest are comments");
        }
    }
}

#[test]
fn the_readme_says_the_sync_migration_is_one_way_and_where_the_key_lives() {
    let text = read("server/README.md");
    let line = text.lines().find(|l| l.contains("core-004-sync") && l.contains("a senso unico")).expect("a line on the one-way migration");
    for needle in ["binario precedente", "rifiuta", "copia"] {
        assert!(line.contains(needle), "{needle}");
    }
    for needle in ["sync keygen", "sync rekey", "sync reset --yes", "fuori dal database", "backup", "mai in `server.toml`"] {
        assert!(text.contains(needle), "{needle}");
    }
}

#[test]
fn the_readme_lists_the_sync_routes_and_errors() {
    let text = read("server/README.md");
    for needle in ["/v1/sync/items", "sync_conflict", "sync_limit", "`epoch`", "ChaCha20-Poly1305"] {
        assert!(text.contains(needle), "{needle}");
    }
}
