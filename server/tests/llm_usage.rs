//! Contatori dell'assistente (migrazione `core-003-llm-usage`): token per utente e per giorno,
//! pulizia dei giorni vecchi e sink di consumo aggiuntivi.
mod support;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use qamille_compute::speech::Segment;
use qamille_server::accounts::ComputeUsage;
use qamille_server::compute::speech::{SpeechBackend, TranscriptionInput};
use qamille_server::compute::{ComputeBackend, ComputeError, DIM, MODEL_ID, ModelStatus, Usage, UsageSink};
use qamille_server::db::{CORE_MIGRATIONS, SCHEMA_V1, SCHEMA_V4};
use qamille_server::Db;
use rusqlite::Connection;
use serde_json::json;
use support::{TestServer, bearer, remote, request_raw, test_config, user_with_device};

fn llm(prompt: u64, completion: u64) -> ComputeUsage {
    ComputeUsage { llm_prompt_tokens: prompt, llm_completion_tokens: completion, ..ComputeUsage::default() }
}

fn db_path(ts: &TestServer) -> std::path::PathBuf {
    ts.dir.path().join("server.db")
}

fn count(path: &Path, sql: &str) -> i64 {
    Connection::open(path).unwrap().query_row(sql, [], |r| r.get(0)).unwrap()
}

#[test]
fn core_003_keeps_the_rows_of_a_database_of_today() {
    let dir = support::TempDir::new();
    let path = dir.path().join("server.db");
    {
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(SCHEMA_V1).unwrap();
        conn.execute_batch(SCHEMA_V4).unwrap();
        conn.pragma_update(None, "user_version", 5).unwrap();
        conn.execute("INSERT INTO users (id, username, created_at) VALUES ('u1', 'mario', '2026-11-01T00:00:00.000Z')", []).unwrap();
        conn.execute("INSERT INTO compute_usage (user_id, requests, audio_seconds, embedded_tokens, updated_at) VALUES ('u1', 4, 1.5, 9, '2026-11-02T00:00:00.000Z')", []).unwrap();
    }
    drop(Db::open(&path, CORE_MIGRATIONS).expect("apertura"));
    let conn = Connection::open(&path).unwrap();
    let row: (i64, f64, i64, i64, i64) = conn
        .query_row(
            "SELECT requests, audio_seconds, embedded_tokens, llm_prompt_tokens, llm_completion_tokens FROM compute_usage WHERE user_id = 'u1'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
        )
        .unwrap();
    assert_eq!(row, (4, 1.5, 9, 0, 0));
    assert_eq!(count(&path, "SELECT COUNT(*) FROM users"), 1);
    assert_eq!(count(&path, "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' AND name = 'llm_usage_day'"), 1);
    let last: String = conn
        .query_row("SELECT name FROM schema_migrations ORDER BY rowid DESC LIMIT 1", [], |r| r.get(0))
        .unwrap();
    assert_eq!(last, "core-004-sync");
}

#[tokio::test]
async fn llm_tokens_are_added_to_the_counters_and_to_the_day() {
    let ts = TestServer::standard();
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let accounts = ts.accounts();
    accounts.record_usage(&login.user_id, llm(10, 5)).await.unwrap();
    accounts.record_usage(&login.user_id, llm(20, 1)).await.unwrap();
    let usage = accounts.usage(&login.user_id).await.unwrap();
    assert_eq!((usage.llm_prompt_tokens, usage.llm_completion_tokens), (30, 6));
    assert_eq!(accounts.llm_tokens_on(&login.user_id, "2026-11-13").await.unwrap(), 36);
    assert_eq!(accounts.llm_tokens_on(&login.user_id, "2026-11-12").await.unwrap(), 0);
    let users = accounts.list_users().await.unwrap();
    assert_eq!(users[0].usage.llm_prompt_tokens, 30);
    assert_eq!(users[0].usage.llm_completion_tokens, 6);
    // Senza token LLM la tabella dei giorni non cambia.
    accounts.record_usage(&login.user_id, ComputeUsage { requests: 1, ..ComputeUsage::default() }).await.unwrap();
    assert_eq!(accounts.llm_tokens_on(&login.user_id, "2026-11-13").await.unwrap(), 36);
}

#[tokio::test]
async fn the_day_follows_the_clock_and_old_days_are_pruned() {
    let ts = TestServer::standard();
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let other = user_with_device(&ts, "anna", "un'altra password").await;
    let accounts = ts.accounts();
    accounts.record_usage(&login.user_id, llm(1, 1)).await.unwrap();
    accounts.record_usage(&other.user_id, llm(3, 3)).await.unwrap();
    ts.clock.advance(chrono::TimeDelta::days(3));
    accounts.record_usage(&login.user_id, llm(2, 2)).await.unwrap();
    assert_eq!(accounts.llm_tokens_on(&login.user_id, "2026-11-13").await.unwrap(), 2);
    assert_eq!(accounts.llm_tokens_on(&login.user_id, "2026-11-16").await.unwrap(), 4);
    ts.clock.advance(chrono::TimeDelta::days(5));
    accounts.record_usage(&login.user_id, llm(7, 0)).await.unwrap();
    // Passati 8 giorni dal primo, la sua riga non c'e' piu'; quella di tre giorni fa si'.
    assert_eq!(accounts.llm_tokens_on(&login.user_id, "2026-11-13").await.unwrap(), 0);
    assert_eq!(accounts.llm_tokens_on(&login.user_id, "2026-11-16").await.unwrap(), 4);
    assert_eq!(accounts.llm_tokens_on(&login.user_id, "2026-11-21").await.unwrap(), 7);
    // La pulizia tocca solo l'utente che scrive.
    assert_eq!(accounts.llm_tokens_on(&other.user_id, "2026-11-13").await.unwrap(), 6);
}

struct NullSink;
impl UsageSink for NullSink {
    fn record(&self, _: &str, _: Usage) {}
}

#[tokio::test]
async fn hostile_token_counts_saturate_instead_of_turning_into_reals() {
    let ts = TestServer::standard();
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    let hostile = llm(9_000_000_000_000_000_000, 9_000_000_000_000_000_000);
    ts.accounts().record_usage(&login.user_id, hostile).await.unwrap();
    ts.accounts().record_usage(&login.user_id, hostile).await.unwrap();

    // Niente REAL nei contatori: restano interi, al massimo di i64.
    let path = db_path(&ts);
    for sql in [
        "SELECT COUNT(*) FROM compute_usage WHERE typeof(llm_prompt_tokens) <> 'integer' OR typeof(llm_completion_tokens) <> 'integer'",
        "SELECT COUNT(*) FROM llm_usage_day WHERE typeof(tokens) <> 'integer'",
    ] {
        assert_eq!(count(&path, sql), 0, "{sql}");
    }
    // L'elenco degli utenti dell'amministrazione continua a funzionare.
    let users = ts.accounts().list_users().await.unwrap();
    assert_eq!(users[0].usage.llm_prompt_tokens, i64::MAX as u64);
    assert_eq!(ts.accounts().usage(&login.user_id).await.unwrap().llm_completion_tokens, i64::MAX as u64);
    let day = ts.accounts().now().format("%Y-%m-%d").to_string();
    assert_eq!(ts.accounts().llm_tokens_on(&login.user_id, &day).await.unwrap(), i64::MAX as u64);

    // Il tetto giornaliero scatta ancora (gateway nuovo: legge dal database).
    let config = qamille_server::llm::LlmConfig {
        provider: qamille_server::llm::LlmProvider::OpenAiCompatible,
        base_url: Some("http://ollama:11434/v1".to_owned()),
        model: Some("m".to_owned()),
        max_tokens_per_day_per_user: 1000,
        ..Default::default()
    };
    let gateway = qamille_server::llm::LlmGateway::new(&config, Arc::clone(ts.accounts()), Arc::new(NullSink)).unwrap();
    assert!(gateway.within_daily_cap(&login.user_id).await.is_err());
}

#[tokio::test]
async fn a_deleted_user_leaves_no_llm_rows() {
    let ts = TestServer::standard();
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    ts.accounts().record_usage(&login.user_id, llm(10, 5)).await.unwrap();
    assert_eq!(count(&db_path(&ts), "SELECT COUNT(*) FROM llm_usage_day"), 1);
    ts.accounts().delete_user(&login.user_id).await.unwrap();
    assert_eq!(count(&db_path(&ts), "SELECT COUNT(*) FROM compute_usage"), 0);
    assert_eq!(count(&db_path(&ts), "SELECT COUNT(*) FROM llm_usage_day"), 0);
    // Un conto per un utente che non c'e' piu' non lascia righe.
    ts.accounts().record_usage(&login.user_id, llm(10, 5)).await.unwrap();
    assert_eq!(count(&db_path(&ts), "SELECT COUNT(*) FROM compute_usage"), 0);
    assert_eq!(count(&db_path(&ts), "SELECT COUNT(*) FROM llm_usage_day"), 0);
}

// --- Sink aggiuntivi -------------------------------------------------------------

struct Fake;

fn unit() -> Vec<f32> {
    let mut v = vec![1.0f32; DIM];
    let norm = (DIM as f32).sqrt();
    v.iter_mut().for_each(|x| *x /= norm);
    v
}

impl ComputeBackend for Fake {
    fn chunk(&self, text: &str) -> Result<Vec<String>, ComputeError> {
        Ok(vec![text.to_owned()])
    }
    fn embed(&self, inputs: &[String]) -> Result<Vec<Vec<f32>>, ComputeError> {
        Ok(inputs.iter().map(|_| unit()).collect())
    }
    fn embed_counted(&self, inputs: &[String]) -> Result<(Vec<Vec<f32>>, u64), ComputeError> {
        let tokens = inputs.iter().map(|input| input.split_whitespace().count() as u64).sum();
        self.embed(inputs).map(|vectors| (vectors, tokens))
    }
    fn embed_image(&self, _: &[u8]) -> Result<Vec<f32>, ComputeError> {
        Ok(unit())
    }
    fn extract(&self, _: &str, _: &[u8]) -> Result<String, ComputeError> {
        Ok(String::new())
    }
    fn models(&self) -> Vec<ModelStatus> {
        Vec::new()
    }
    fn install(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::ModelMissing("none".to_owned()))
    }
}

struct Speech;

impl SpeechBackend for Speech {
    fn models(&self) -> Vec<ModelStatus> {
        Vec::new()
    }
    fn install(&self, _: &str) -> Result<ModelStatus, ComputeError> {
        Err(ComputeError::ModelMissing("none".to_owned()))
    }
    fn transcribe(&self, _: &TranscriptionInput, _: &AtomicBool) -> Result<Vec<Segment>, ComputeError> {
        Ok(Vec::new())
    }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<(String, Usage)>>);

impl UsageSink for Recorder {
    fn record(&self, user_id: &str, usage: Usage) {
        self.0.lock().unwrap().push((user_id.to_owned(), usage));
    }
}

fn with_sinks(sinks: &[Arc<Recorder>]) -> TestServer {
    let TestServer { server, clock, dir, .. } = TestServer::new(test_config());
    let mut server = server.with_compute(Arc::new(Fake), Arc::new(Speech), 1);
    for sink in sinks {
        server = server.with_usage_sink(sink.clone());
    }
    TestServer { public: server.public_router(), admin: server.admin_router(), server, clock, dir }
}

async fn embed(ts: &TestServer, key: &str, text: &str) {
    let body = serde_json::to_vec(&json!({"model": MODEL_ID, "input": [text], "dimensions": DIM})).unwrap();
    let ok = request_raw(&ts.public, "POST", "/v1/embeddings", Some(body), remote([10, 0, 0, 9]), &[("authorization", &bearer(key))]).await;
    assert_eq!(ok.status, 200, "{}", ok.text);
}

async fn settled(ts: &TestServer, user_id: &str, at_least: u64) -> ComputeUsage {
    let mut usage = ts.accounts().usage(user_id).await.unwrap();
    for _ in 0..50 {
        if usage.requests >= at_least {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
        usage = ts.accounts().usage(user_id).await.unwrap();
    }
    usage
}

#[tokio::test]
async fn an_extra_usage_sink_receives_the_same_numbers_as_the_core_counters() {
    let recorder = Arc::new(Recorder::default());
    let ts = with_sinks(std::slice::from_ref(&recorder));
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    embed(&ts, &login.device_key, "fattura di ottobre").await;
    let usage = settled(&ts, &login.user_id, 1).await;
    let seen = recorder.0.lock().unwrap().clone();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].0, login.user_id);
    assert_eq!(seen[0].1.requests, 1);
    assert_eq!(seen[0].1.embedded_tokens, 3);
    // I contatori del core non sono stati sostituiti dal sink.
    assert_eq!((usage.requests, usage.embedded_tokens), (seen[0].1.requests, seen[0].1.embedded_tokens));
}

#[tokio::test]
async fn two_extra_sinks_both_receive_every_record() {
    let first = Arc::new(Recorder::default());
    let second = Arc::new(Recorder::default());
    let ts = with_sinks(&[first.clone(), second.clone()]);
    let login = user_with_device(&ts, "mario", "una password lunga").await;
    embed(&ts, &login.device_key, "uno due").await;
    embed(&ts, &login.device_key, "tre").await;
    let usage = settled(&ts, &login.user_id, 2).await;
    assert_eq!(usage.requests, 2);
    for sink in [&first, &second] {
        let seen = sink.0.lock().unwrap().clone();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen.iter().map(|(_, u)| u.requests).sum::<u64>(), 2);
        assert_eq!(seen.iter().map(|(_, u)| u.embedded_tokens).sum::<u64>(), 3);
    }
}
