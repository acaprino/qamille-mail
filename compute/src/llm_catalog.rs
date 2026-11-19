//! The catalog of the LLM models the server can download and run itself (`llm_provider =
//! "builtin"`, spec L8): each entry is pinned in the code to a Hugging Face commit and a SHA-256
//! (INV-39). The catalog is checked into the code like `models.rs` and `speech.rs`; an entry that
//! is not pinned is refused by [`install`] and fails `the_catalog_has_no_unpinned_entries` with
//! the name of the decision that is missing.

use crate::download::{self, DownloadError, Options, Target};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Every id of this catalog starts with this.
pub const ID_PREFIX: &str = "llm-";

/// The licence text of an entry that is not pinned yet.
pub const LICENSE_UNDECIDED: &str = "DA DECIDERE";

#[derive(Debug)]
pub struct LlmModelSpec {
    pub id: &'static str,
    /// `owner/name` on Hugging Face.
    pub repo: &'static str,
    /// The commit (40 lowercase hex digits), never a branch.
    pub revision: &'static str,
    pub file: &'static str,
    pub bytes: u64,
    pub sha256: &'static str,
    /// SPDX name of the licence, read at `revision`.
    pub license: &'static str,
    /// Where the licence terms are published (shown before the operator installs the model).
    pub license_url: &'static str,
    /// The quantisation, as the people who made the file call it (shown in the model list).
    pub quantization: &'static str,
    /// What the operator writes in `QAMILLE_LLM_ACCEPT_LICENSE` before the server downloads this
    /// model by itself ("" when no acknowledgement is asked).
    pub accept_license: &'static str,
    /// Memory of the weights and the engine at the smallest context, in MiB.
    pub ram_mb: u32,
    /// KV cache per 1024 tokens of context, in MiB.
    pub kv_mb_per_1k: u32,
    /// Recommended context, in tokens, per slot.
    pub context: u32,
    pub speed_note: &'static str,
    /// The repository asks to accept a licence (a token is needed). The default catalog has none.
    pub gated: bool,
}

/// The default catalog. A value that could not be found stays empty, so that
/// `the_catalog_has_no_unpinned_entries` fails and names the owner's decision.
pub const CATALOG: &[LlmModelSpec] = &[
    LlmModelSpec {
        id: "llm-piccolo",
        repo: "google/gemma-4-E4B-it-qat-q4_0-gguf",
        revision: "4b4a2c1d584be7264f87aac328a1bc739ce81b6c",
        file: "gemma-4-E4B_q4_0-it.gguf",
        bytes: 5_154_941_280,
        sha256: "676c35070db6dbe52f93e9c864ee0fba4eddea94b9c875d9cb10daff453fbaee",
        license: "Apache-2.0",
        license_url: "https://ai.google.dev/gemma/docs/gemma_4_license",
        quantization: "Q4_0 (QAT)",
        accept_license: "gemma",
        ram_mb: 5708,
        kv_mb_per_1k: 56,
        context: 8192,
        speed_note: "not measured; SHA-256 verified by a full download",
        gated: false,
    },
    LlmModelSpec {
        id: "llm-medio",
        repo: "google/gemma-4-12B-it-qat-q4_0-gguf",
        revision: "29d097773436b69ff9feafd636ab4cf873786537",
        file: "gemma-4-12b-it-qat-q4_0.gguf",
        bytes: 6_975_879_296,
        sha256: "93567e57a8fe10b23569b9d9ec38cd005deedf71e29477c421a4b83f418a538b",
        license: "Apache-2.0",
        license_url: "https://ai.google.dev/gemma/docs/gemma_4_license",
        quantization: "Q4_0 (QAT)",
        accept_license: "gemma",
        ram_mb: 7618,
        kv_mb_per_1k: 336,
        context: 8192,
        speed_note: "not measured; SHA-256 from the Hub, not downloaded again",
        gated: false,
    },
    LlmModelSpec {
        id: "llm-grande",
        repo: "google/gemma-4-26B-A4B-it-qat-q4_0-gguf",
        revision: "d1c082be9cf3c8a514acf63b8761f4b41935842e",
        file: "gemma-4-26B_q4_0-it.gguf",
        bytes: 14_439_363_584,
        sha256: "3eca3b8f6d7baf218a7dd6bba5fb59a56ee25fe2d567b6f5f589b4f697eca51d",
        license: "Apache-2.0",
        license_url: "https://ai.google.dev/gemma/docs/gemma_4_license",
        quantization: "Q4_0 (QAT)",
        accept_license: "gemma",
        ram_mb: 15448,
        kv_mb_per_1k: 220,
        context: 8192,
        speed_note: "not measured; SHA-256 from the Hub, not downloaded again",
        gated: false,
    },
];

/// The entry named exactly `id`: no normalisation, so an id cannot name another folder.
pub fn spec(id: &str) -> Option<&'static LlmModelSpec> {
    spec_in(CATALOG, id)
}

pub fn spec_in<'a>(catalog: &'a [LlmModelSpec], id: &str) -> Option<&'a LlmModelSpec> {
    catalog.iter().find(|spec| spec.id == id)
}

fn is_lower_hex(value: &str, len: usize) -> bool {
    value.len() == len && value.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Revision, size, SHA-256 and licence are all known (INV-39).
pub fn is_pinned(spec: &LlmModelSpec) -> bool {
    is_lower_hex(spec.revision, 40)
        && is_lower_hex(spec.sha256, 64)
        && spec.bytes > 0
        && !spec.license.is_empty()
        && spec.license != LICENSE_UNDECIDED
}

/// The file's address, at the pinned revision.
pub fn url(spec: &LlmModelSpec) -> String {
    format!("https://huggingface.co/{}/resolve/{}/{}", spec.repo, spec.revision, spec.file)
}

/// `<models>/llm/<id>`.
pub fn model_dir(models_dir: &Path, spec: &LlmModelSpec) -> PathBuf {
    models_dir.join("llm").join(spec.id)
}

pub fn model_file(models_dir: &Path, spec: &LlmModelSpec) -> PathBuf {
    model_dir(models_dir, spec).join(spec.file)
}

/// The file exists with the declared size (a full check is [`verify`]).
pub fn present(models_dir: &Path, spec: &LlmModelSpec) -> bool {
    std::fs::metadata(model_file(models_dir, spec)).is_ok_and(|m| m.is_file() && m.len() == spec.bytes)
}

/// Size and SHA-256 of `path` against the entry.
pub fn verify(path: &Path, spec: &LlmModelSpec, cancel: &AtomicBool) -> Result<(), String> {
    const FAILED: &str = "The model failed its integrity check. Reinstall the model.";
    let mut file = std::fs::File::open(path).map_err(|e| format!("Cannot read the model: {}", e.kind()))?;
    if file.metadata().map_err(|e| e.to_string())?.len() != spec.bytes {
        return Err(FAILED.to_owned());
    }
    let mut hash = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled.".to_owned());
        }
        let n = file.read(&mut buffer).map_err(|e| format!("Cannot read the model: {}", e.kind()))?;
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if download::hash_hex(hash) == spec.sha256 { Ok(()) } else { Err(FAILED.to_owned()) }
}

/// Memory to start the engine with `context_total` tokens of context (all slots together), in MiB,
/// with a 15% margin.
pub fn required_ram_mb(spec: &LlmModelSpec, context_total: u32) -> u64 {
    (u64::from(spec.ram_mb) + u64::from(spec.kv_mb_per_1k) * u64::from(context_total) / 1024) * 115 / 100
}

/// Disk to install the model: the file plus the reserve [`download`] keeps free.
pub fn required_disk_bytes(spec: &LlmModelSpec) -> u64 {
    spec.bytes.saturating_add(download::PHYSICAL_RESERVE)
}

/// Downloads and checks the model into `<models>/llm/<id>`. One installation at a time across
/// threads and processes; the second finds the model installed. Never a half file under the final
/// name. `options` brings the mirror and the token; `gated` is the entry's.
pub fn install(
    models_dir: &Path,
    spec: &LlmModelSpec,
    options: &Options,
    cancel: &AtomicBool,
    progress: &(dyn Fn(u64, u64) + Sync),
) -> Result<(), String> {
    if !is_pinned(spec) {
        return Err("This model is not available yet: the catalog entry is not pinned.".to_owned());
    }
    let dir = model_dir(models_dir, spec);
    std::fs::create_dir_all(&dir).map_err(|e| format!("Cannot write the model: {}", e.kind()))?;
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(dir.join("download.lock"))
        .map_err(|e| format!("Cannot write the model: {}", e.kind()))?;
    lock.lock().map_err(|e| format!("Cannot write the model: {e}"))?;
    if present(models_dir, spec) {
        // The size alone proves nothing: a same-size corrupted file is deleted and fetched again.
        if verify(&model_file(models_dir, spec), spec, cancel).is_ok() {
            return Ok(());
        }
        if cancel.load(Ordering::Relaxed) {
            return Err("Cancelled.".to_owned());
        }
        std::fs::remove_file(model_file(models_dir, spec)).map_err(|e| format!("Cannot write the model: {}", e.kind()))?;
    }
    let options = Options { gated: spec.gated, resume: true, ..options.clone() };
    let target = Target { name: spec.file, size: spec.bytes, sha256: spec.sha256 };
    let agent = download::agent();
    download::fetch(&agent, &url(spec), &dir, spec.file, &target, &options, cancel, &|got| progress(got, spec.bytes))
        .map_err(|error: DownloadError| error.to_string())?;
    progress(spec.bytes, spec.bytes);
    Ok(())
}

/// Deletes `<models>/llm/<id>` with its part files and lock. A model that is not there: `Ok`.
pub fn remove(models_dir: &Path, spec: &LlmModelSpec) -> Result<(), String> {
    match std::fs::remove_dir_all(model_dir(models_dir, spec)) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(format!("Cannot remove the model: {}", error.kind())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::download::Mirror;
    use std::io::{BufRead, Write};
    use std::sync::{Arc, Mutex};

    // --- the real catalog -------------------------------------------------------------

    #[test]
    fn ids_are_unique_and_start_with_llm() {
        let mut ids: Vec<&str> = CATALOG.iter().map(|s| s.id).collect();
        assert!(ids.iter().all(|id| id.starts_with(ID_PREFIX)));
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), CATALOG.len());
    }

    #[test]
    fn the_three_sizes_exist() {
        for id in ["llm-piccolo", "llm-medio", "llm-grande"] {
            assert!(spec(id).is_some(), "{id}");
        }
    }

    #[test]
    fn sha256_has_64_lowercase_hex_digits() {
        for s in CATALOG {
            assert!(is_lower_hex(s.sha256, 64), "{}: sha256 {:?}", s.id, s.sha256);
        }
    }

    #[test]
    fn revision_has_40_lowercase_hex_digits() {
        for s in CATALOG {
            assert!(is_lower_hex(s.revision, 40), "{}: revision {:?}", s.id, s.revision);
        }
    }

    #[test]
    fn sizes_and_memory_figures_are_positive() {
        for s in CATALOG {
            assert!(s.bytes > 0 && s.ram_mb > 0 && s.kv_mb_per_1k > 0 && s.context > 0, "{}: figures missing", s.id);
        }
    }

    #[test]
    fn repo_and_file_are_plain() {
        for s in CATALOG {
            let (owner, name) = s.repo.split_once('/').expect("owner/name");
            for part in [owner, name] {
                assert!(!part.is_empty() && !part.contains("..") && !part.contains('/'), "{}", s.repo);
            }
            assert!(!s.file.contains('/') && !s.file.contains('\\') && !s.file.contains(".."), "{}", s.file);
            assert!(s.file.ends_with(".gguf"), "{}", s.file);
        }
    }

    #[test]
    fn no_entry_is_gated() {
        assert!(CATALOG.iter().all(|s| !s.gated), "the default catalog must not need a token (L8)");
    }

    #[test]
    fn the_models_are_gemma_from_the_authors_ungated_repositories() {
        for s in CATALOG {
            assert!(s.repo.starts_with("google/gemma-4-"), "{}: {}", s.id, s.repo);
            assert!(!s.gated, "{}", s.id);
        }
    }

    #[test]
    fn every_entry_names_its_license_and_where_the_terms_are() {
        for s in CATALOG {
            assert!(!s.license.is_empty() && s.license != LICENSE_UNDECIDED, "{}: licence", s.id);
            assert!(s.license_url.starts_with("https://") && !s.license_url.contains(' '), "{}: terms URL {:?}", s.id, s.license_url);
            assert!(!s.quantization.is_empty(), "{}: quantization", s.id);
        }
    }

    #[test]
    fn the_gemma_models_ask_for_the_acknowledgement_before_an_automatic_download() {
        for s in CATALOG {
            assert_eq!(s.accept_license, "gemma", "{}", s.id);
        }
    }

    #[test]
    fn licenses_are_open() {
        for s in CATALOG {
            let open = matches!(s.license, "Apache-2.0" | "MIT") || s.license.starts_with("accettata dal proprietario: ");
            assert!(open, "{}: licence {:?} is not Apache-2.0, MIT or accepted by the owner", s.id, s.license);
        }
    }

    #[test]
    fn the_catalog_has_no_unpinned_entries() {
        let unpinned: Vec<&str> = CATALOG.iter().filter(|s| !is_pinned(s)).map(|s| s.id).collect();
        assert!(
            unpinned.is_empty(),
            "decisione del proprietario: queste voci del catalogo non sono fissate (revisione, SHA-256, dimensione e licenza): {unpinned:?}"
        );
    }

    // --- the logic, with a test catalog -------------------------------------------------

    fn leak(text: String) -> &'static str {
        Box::leak(text.into_boxed_str())
    }

    fn payload(len: usize) -> Vec<u8> {
        (0..len).map(|i| (i * 31 % 251) as u8).collect()
    }

    fn sha(bytes: &[u8]) -> String {
        let mut hash = Sha256::new();
        hash.update(bytes);
        download::hash_hex(hash)
    }

    fn test_spec(id: &'static str, bytes: &[u8]) -> &'static LlmModelSpec {
        Box::leak(Box::new(LlmModelSpec {
            id,
            repo: "acme/test-GGUF",
            revision: leak("a".repeat(40)),
            file: "test-Q4_K_M.gguf",
            bytes: bytes.len() as u64,
            sha256: leak(sha(bytes)),
            license: "Apache-2.0",
            license_url: "https://example.test/terms",
            quantization: "Q4_K_M",
            accept_license: "",
            ram_mb: 4000,
            kv_mb_per_1k: 100,
            context: 8192,
            speed_note: "",
            gated: false,
        }))
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("qamille-llm-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Path and `Range` header of each request, in order.
    type Requests = Arc<Mutex<Vec<(String, Option<String>)>>>;

    struct Server {
        url: String,
        requests: Requests,
    }

    /// Serves `payload` for any path; the first answer stops after `cut` bytes.
    fn serve(payload: Vec<u8>, cut: usize) -> Server {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let requests: Requests = Arc::default();
        let log = requests.clone();
        std::thread::spawn(move || {
            for (index, stream) in listener.incoming().enumerate() {
                let mut stream = stream.unwrap();
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut first = String::new();
                reader.read_line(&mut first).unwrap();
                let path = first.split_whitespace().nth(1).unwrap_or("").to_owned();
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if reader.read_line(&mut line).unwrap() == 0 || line.trim_end().is_empty() {
                        break;
                    }
                    if let Some(value) = line.to_ascii_lowercase().trim_end().strip_prefix("range: ") {
                        range = Some(value.to_owned());
                    }
                }
                log.lock().unwrap().push((path, range.clone()));
                let len = payload.len();
                let start = range
                    .as_deref()
                    .and_then(|r| r.strip_prefix("bytes="))
                    .and_then(|r| r.strip_suffix('-'))
                    .and_then(|n| n.parse::<usize>().ok());
                let head = match start {
                    Some(s) => format!("HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {s}-{}/{len}\r\nConnection: close\r\n\r\n", len - s, len - 1),
                    None => format!("HTTP/1.1 200 OK\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n"),
                };
                let _ = stream.write_all(head.as_bytes());
                let body = &payload[start.unwrap_or(0)..];
                let body = if index == 0 { &body[..cut.min(body.len())] } else { body };
                let _ = stream.write_all(body);
                let _ = stream.flush();
                let _ = stream.shutdown(std::net::Shutdown::Write);
            }
        });
        Server { url: format!("http://127.0.0.1:{port}"), requests }
    }

    impl Server {
        fn hits(&self) -> usize {
            self.requests.lock().unwrap().len()
        }
        fn options(&self) -> Options {
            Options::resumable().with_mirror(Mirror::parse(&self.url).ok())
        }
    }

    fn install_ok(dir: &Path, spec: &LlmModelSpec, options: &Options) -> Result<(), String> {
        install(dir, spec, options, &AtomicBool::new(false), &|_, _| {})
    }

    #[test]
    fn the_model_id_cannot_escape_the_folder() {
        for id in ["llm-../x", "../llm-piccolo", "LLM-PICCOLO", "llm-piccolo ", " llm-piccolo", "llm-piccolo/..", "llm-piccolo\\x", ""] {
            assert!(spec(id).is_none(), "{id:?}");
        }
        let one = test_spec("llm-test", b"abc");
        assert!(spec_in(&[], "llm-test").is_none());
        assert!(spec_in(std::slice::from_ref(one), "llm-test").is_some());
    }

    #[test]
    fn urls_and_paths_follow_the_revision() {
        let s = test_spec("llm-test", b"abc");
        assert_eq!(url(s), format!("https://huggingface.co/acme/test-GGUF/resolve/{}/test-Q4_K_M.gguf", "a".repeat(40)));
        let models = Path::new("models");
        assert_eq!(model_dir(models, s), models.join("llm").join("llm-test"));
        assert_eq!(model_file(models, s), models.join("llm").join("llm-test").join("test-Q4_K_M.gguf"));
    }

    #[test]
    fn required_ram_includes_the_context_and_the_margin() {
        let s = test_spec("llm-test", b"abc");
        // (4000 + 100 x 8192 / 1024) x 1.15 = 4800 x 1.15
        assert_eq!(required_ram_mb(s, 8192), 5520);
        assert_eq!(required_ram_mb(s, 0), 4600);
        assert_eq!(required_disk_bytes(s), 3 + download::PHYSICAL_RESERVE);
    }

    #[test]
    fn install_downloads_into_the_model_folder() {
        let bytes = payload(300_000);
        let s = test_spec("llm-test", &bytes);
        let server = serve(bytes.clone(), usize::MAX);
        let dir = temp_dir("install");
        let seen = Mutex::new(Vec::new());
        install(&dir, s, &server.options(), &AtomicBool::new(false), &|done, total| seen.lock().unwrap().push((done, total))).unwrap();
        assert_eq!(std::fs::read(model_file(&dir, s)).unwrap(), bytes);
        assert!(present(&dir, s));
        let leftovers: Vec<_> = std::fs::read_dir(model_dir(&dir, s)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(leftovers.iter().all(|name| !name.ends_with(".part")), "{leftovers:?}");
        assert_eq!(*seen.lock().unwrap().last().unwrap(), (300_000, 300_000));
        assert_eq!(server.requests.lock().unwrap()[0].0, format!("/acme/test-GGUF/resolve/{}/test-Q4_K_M.gguf", "a".repeat(40)));
    }

    #[test]
    fn install_resumes_after_a_cut() {
        let bytes = payload(300_000);
        let s = test_spec("llm-test", &bytes);
        let server = serve(bytes.clone(), 100_000);
        let dir = temp_dir("resume");
        assert!(install_ok(&dir, s, &server.options()).is_err());
        assert!(!present(&dir, s));
        install_ok(&dir, s, &server.options()).unwrap();
        assert_eq!(std::fs::read(model_file(&dir, s)).unwrap(), bytes);
        assert!(server.requests.lock().unwrap()[1].1.as_deref().is_some_and(|r| r.starts_with("bytes=")));
    }

    #[test]
    fn a_wrong_sha_is_refused_and_nothing_is_left() {
        let bytes = payload(50_000);
        let other: Vec<u8> = bytes.iter().map(|b| b ^ 1).collect();
        let s = test_spec("llm-test", &bytes);
        let server = serve(other, usize::MAX);
        let dir = temp_dir("wrongsha");
        let error = install_ok(&dir, s, &server.options()).unwrap_err();
        assert!(error.contains("integrity"), "{error}");
        assert!(!present(&dir, s));
        let names: Vec<_> = std::fs::read_dir(model_dir(&dir, s)).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| n == "download.lock"), "{names:?}");
    }

    #[test]
    fn a_different_size_is_refused() {
        let bytes = payload(50_000);
        let s = test_spec("llm-test", &bytes);
        let server = serve(payload(80_000), usize::MAX);
        let dir = temp_dir("size");
        let error = install_ok(&dir, s, &server.options()).unwrap_err();
        assert!(error.contains("exceeded its declared size"), "{error}");
        assert!(!present(&dir, s));
    }

    #[test]
    fn install_refuses_an_unpinned_entry() {
        let server = serve(payload(10), usize::MAX);
        let dir = temp_dir("unpinned");
        let unpinned = Box::leak(Box::new(LlmModelSpec { revision: "", ..clone_spec(test_spec("llm-unpinned", b"abc")) }));
        let error = install_ok(&dir, unpinned, &server.options()).unwrap_err();
        assert_eq!(error, "This model is not available yet: the catalog entry is not pinned.");
        assert_eq!(server.hits(), 0);
        assert!(!dir.join("llm").exists());
    }

    #[test]
    fn a_gated_entry_without_a_token_is_refused_before_any_request() {
        let bytes = payload(10_000);
        let gated = Box::leak(Box::new(LlmModelSpec { gated: true, ..clone_spec(test_spec("llm-gated", &bytes)) }));
        let server = serve(bytes, usize::MAX);
        let dir = temp_dir("gated");
        let error = install_ok(&dir, gated, &Options::resumable().with_mirror(Mirror::parse(&server.url).ok())).unwrap_err();
        assert!(error.contains("token"), "{error}");
        assert_eq!(server.hits(), 0);
    }

    fn clone_spec(s: &LlmModelSpec) -> LlmModelSpec {
        LlmModelSpec { ..*s }
    }

    // The free disk space is known only on Windows (memory::available_disk_bytes): elsewhere the
    // install does not check it before downloading.
    #[cfg(windows)]
    #[test]
    fn install_needs_free_disk() {
        let bytes = payload(1000);
        let huge = Box::leak(Box::new(LlmModelSpec { bytes: 1 << 60, ..clone_spec(test_spec("llm-huge", &bytes)) }));
        let server = serve(bytes, usize::MAX);
        let dir = temp_dir("disk");
        let error = install_ok(&dir, huge, &server.options()).unwrap_err();
        assert!(error.contains("Not enough free disk space"), "{error}");
        assert_eq!(server.hits(), 0);
    }

    #[test]
    fn two_installs_at_once_take_turns() {
        let bytes = payload(400_000);
        let s = test_spec("llm-test", &bytes);
        let server = serve(bytes.clone(), usize::MAX);
        let dir = temp_dir("turns");
        let options = server.options();
        let workers: Vec<_> = (0..2)
            .map(|_| {
                let (dir, options) = (dir.clone(), options.clone());
                std::thread::spawn(move || install_ok(&dir, s, &options))
            })
            .collect();
        for worker in workers {
            worker.join().unwrap().unwrap();
        }
        assert_eq!(server.hits(), 1, "the second finds the model installed");
        assert!(present(&dir, s));
    }

    #[test]
    fn remove_deletes_the_folder_with_its_part_files() {
        let bytes = payload(1000);
        let s = test_spec("llm-test", &bytes);
        let dir = temp_dir("remove");
        std::fs::create_dir_all(model_dir(&dir, s)).unwrap();
        std::fs::write(model_file(&dir, s), &bytes).unwrap();
        std::fs::write(model_dir(&dir, s).join("test-Q4_K_M.gguf.part"), b"half").unwrap();
        std::fs::write(model_dir(&dir, s).join("download.lock"), b"").unwrap();
        remove(&dir, s).unwrap();
        assert!(!model_dir(&dir, s).exists());
        assert!(dir.join("llm").exists(), "only the model's own folder goes");
    }

    #[test]
    fn install_replaces_a_corrupted_file_of_the_right_size() {
        let bytes = payload(300_000);
        let s = test_spec("llm-test", &bytes);
        let server = serve(bytes.clone(), usize::MAX);
        let dir = temp_dir("install-corrupt");
        install_ok(&dir, s, &server.options()).unwrap();
        let mut bad = bytes.clone();
        bad[1000] ^= 0xff;
        std::fs::write(model_file(&dir, s), &bad).unwrap();
        assert!(present(&dir, s), "the size matches: only the hash tells");
        let before = server.hits();
        install_ok(&dir, s, &server.options()).unwrap();
        assert!(server.hits() > before, "downloaded again");
        assert_eq!(std::fs::read(model_file(&dir, s)).unwrap(), bytes);
        let hits = server.hits();
        install_ok(&dir, s, &server.options()).unwrap();
        assert_eq!(server.hits(), hits, "a verified file is not downloaded again");
    }

    #[test]
    fn remove_of_a_missing_model_is_ok() {
        let s = test_spec("llm-test", b"abc");
        remove(&temp_dir("remove-missing"), s).unwrap();
    }

    #[test]
    fn verify_catches_a_flipped_byte_and_a_wrong_size() {
        let bytes = payload(5000);
        let s = test_spec("llm-test", &bytes);
        let dir = temp_dir("verify");
        let path = dir.join("m.gguf");
        let cancel = AtomicBool::new(false);
        std::fs::write(&path, &bytes).unwrap();
        verify(&path, s, &cancel).unwrap();
        let mut flipped = bytes.clone();
        flipped[2500] ^= 0xff;
        std::fs::write(&path, &flipped).unwrap();
        assert!(verify(&path, s, &cancel).is_err());
        std::fs::write(&path, &bytes[..4999]).unwrap();
        assert!(verify(&path, s, &cancel).is_err());
        assert!(verify(&dir.join("missing.gguf"), s, &cancel).is_err());
    }
}
