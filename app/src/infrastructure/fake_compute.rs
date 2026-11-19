//! A compute gateway without a model, for tests: passages are sentences, a vector counts the
//! words of the text in 256 hashed buckets (texts sharing words score high), documents are read
//! as UTF-8, a speech window is one segment saying how long it is. It records the calls so a
//! test can see what reached the "server". Only for tests (`test-support`).
use crate::application::compute::{ComputeGateway, ComputeHealth, ComputeModel, EMBEDDING_DIMENSIONS, EMBEDDING_MODEL_ID, SpeechSegment, codes};
use crate::application::{AppError, AppResult};
use std::sync::Mutex;
use std::sync::atomic::AtomicBool;

#[derive(Default)]
pub struct FakeCompute {
    pub calls: Mutex<Vec<String>>,
    /// When set, every inference fails with this code (and its message).
    pub fail: Mutex<Option<(&'static str, String)>>,
    /// The gateway's identity; `fake` when unset.
    pub identity: Option<String>,
    /// Speech models (`whisper-base`) the fake server has installed.
    pub speech_ready: Mutex<Vec<String>>,
    /// The embedding model `/health` declares (D4); `None`: not declared (the local model).
    pub embedding: Mutex<Option<crate::application::compute::EmbeddingIdentity>>,
}

impl FakeCompute {
    /// A fake that calls itself `identity` (`local`, or a server address).
    pub fn with_identity(identity: &str) -> Self {
        FakeCompute { identity: Some(identity.to_owned()), ..Default::default() }
    }
}

fn words(text: &str) -> impl Iterator<Item = String> + '_ {
    text.split(|c: char| !c.is_alphanumeric()).filter(|w| w.chars().count() > 2).map(str::to_lowercase)
}

/// Words hashed into `EMBEDDING_DIMENSIONS` buckets, normalized.
pub fn bag_of_words(text: &str) -> Vec<f32> {
    let mut vector = vec![0f32; EMBEDDING_DIMENSIONS];
    for word in words(text) {
        let mut hash: u64 = 0xcbf29ce484222325;
        for byte in word.bytes() {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        vector[(hash % EMBEDDING_DIMENSIONS as u64) as usize] += 1.0;
    }
    let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm == 0.0 {
        vector[0] = 1.0;
    } else {
        vector.iter_mut().for_each(|x| *x /= norm);
    }
    vector
}

impl FakeCompute {
    fn record(&self, call: impl Into<String>) -> AppResult<()> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).push(call.into());
        match self.fail.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            Some((code, message)) => Err(AppError::new(code, message)),
            None => Ok(()),
        }
    }

    pub fn calls(&self) -> Vec<String> {
        self.calls.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl ComputeGateway for FakeCompute {
    fn chunk_text(&self, text: &str) -> AppResult<Vec<String>> {
        self.record("chunks")?;
        Ok(text.split_inclusive(['.', '\n']).map(str::trim).filter(|t| !t.is_empty()).map(str::to_owned).collect())
    }
    fn embed_query(&self, text: &str) -> AppResult<Vec<f32>> {
        self.record("query")?;
        Ok(bag_of_words(text))
    }
    fn embed_document(&self, _title: &str, text: &str) -> AppResult<Vec<f32>> {
        self.record("document")?;
        Ok(bag_of_words(text))
    }
    fn embed_documents(&self, _title: &str, texts: &[String]) -> AppResult<Vec<Vec<f32>>> {
        self.record(format!("documents:{}", texts.len()))?;
        Ok(texts.iter().map(|text| bag_of_words(text)).collect())
    }
    fn embed_image(&self, bytes: &[u8]) -> AppResult<Vec<f32>> {
        self.record("image")?;
        if bytes.is_empty() {
            return Err(AppError::new(codes::DOCUMENT_UNREADABLE, "Empty image"));
        }
        Ok(bag_of_words("image picture photo"))
    }
    fn extract_document(&self, _name: &str, bytes: &[u8]) -> AppResult<String> {
        self.record("extract")?;
        Ok(String::from_utf8_lossy(bytes).trim().to_owned())
    }
    fn health(&self) -> AppResult<ComputeHealth> {
        self.record("health")?;
        let ready = self.speech_ready.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let mut models = vec![ComputeModel { id: EMBEDDING_MODEL_ID.into(), state: "ready".into(), percent: Some(100), dimensions: EMBEDDING_DIMENSIONS, quantization: "q4f16".into(), ..Default::default() }];
        for (size, bytes) in [("tiny", 77_691_713u64), ("base", 147_951_465), ("small", 487_601_967)] {
            let id = format!("whisper-{size}");
            let installed = ready.contains(&id);
            models.push(ComputeModel {
                state: if installed { "ready" } else { "missing" }.into(),
                percent: installed.then_some(100),
                done_bytes: if installed { bytes } else { 0 },
                total_bytes: bytes,
                id,
                quantization: "ggml".into(),
                ..Default::default()
            });
        }
        let embedding = self.embedding.lock().unwrap_or_else(|e| e.into_inner()).clone();
        Ok(ComputeHealth { version: "fake".into(), models, embedding })
    }
    fn transcribe_window(&self, wav: &[u8], model: &str, language: &str, _cancel: &AtomicBool) -> AppResult<Vec<SpeechSegment>> {
        self.record(format!("transcribe:{model}:{language}"))?;
        let ms = (wav.len().saturating_sub(44) as u64) * 1000 / 32_000;
        Ok(vec![SpeechSegment { start_ms: 0, end_ms: ms.max(1), text: format!("finestra di {ms} ms") }])
    }
    fn identity(&self) -> String {
        self.identity.clone().unwrap_or_else(|| "fake".into())
    }
    /// Remote only when it calls itself by a server address.
    fn is_remote(&self) -> bool {
        self.identity.as_deref().is_some_and(|identity| identity.starts_with("http"))
    }
}
