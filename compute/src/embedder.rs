//! EmbeddingGemma 2 on the CPU through ONNX Runtime, using the smallest published build (4-bit
//! weights, 16-bit floats). Moved unchanged from the app's `rag.rs` (spec D3): the server holds
//! the model, the app only ever sees vectors and passages.
//!
//! The prompts (`task: search result | query: …`, `title: … | text: …`) are added by the client
//! (spec D2); [`query_prompt`] and [`document_prompt`] are the reference forms.

use crate::models;
use crate::vision::{PATCH_VALUES, PATCHES, Pixels};
use ort::session::Session;
use ort::value::Tensor;
use std::path::{Path, PathBuf};

/// Model card: 256 dimensions lose almost nothing against the native 768.
pub const DIM: usize = 256;
const NATIVE_DIM: usize = 768;
/// Model card: the three feature inputs carry 512 values per token; text-only search leaves them empty.
const FEATURE_DIM: i64 = 512;
pub const CHUNK_TOKENS: usize = 256; // SparkAIModels.dll embeddings(input:chunked:) sub_180002bc0
pub const CHUNK_OVERLAP_TOKENS: usize = 20; // same call
pub const MAX_DOC_CHARS: usize = 200_000;
/// Longest input EmbeddingGemma takes, in tokens with the special ones: attention grows with
/// the square of the length, so longer inputs are refused, never run.
pub const MAX_INPUT_TOKENS: usize = 2048; // estimated: stops one huge attachment taking over
/// Threads for one inference: half the logical cores (the other half stays with the UI, the
/// mail sync and the user's own work), never fewer than 2 or more than `MAX_CPU_THREADS`.
/// One measurement, one machine (Intel Core Ultra 7 155H, 22 logical cores), one ~1000-character
/// passage, 12 runs per setting: 2 threads 1.1-1.9 s (two runs), 4 threads 0.80 s, 6 threads
/// 0.61 s, 8 threads 0.62 s, 11 threads 0.58 s. The last three are within noise: the cap is a
/// prudent reading of three close points, not a proven optimum. Not measured: other CPUs, long
/// mails, the UI and the mail sync running at the same time.
pub const MIN_CPU_THREADS: usize = 2;
pub const MAX_CPU_THREADS: usize = 6;
/// Recorded with every index (spec D4).
pub const QUANTIZATION: &str = "q4f16";

pub fn cpu_threads_for(logical_cores: usize) -> usize {
    (logical_cores / 2).clamp(MIN_CPU_THREADS, MAX_CPU_THREADS)
}
/// Computed once per process, from the machine it runs on.
pub fn cpu_threads() -> usize {
    cpu_threads_for(std::thread::available_parallelism().map_or(MIN_CPU_THREADS, |n| n.get()))
}

/// The query prompt of the model card, as the client sends it.
pub fn query_prompt(query: &str) -> String {
    format!("task: search result | query: {query}")
}

/// The document prompt of the model card, as the client sends it.
pub fn document_prompt(title: &str, text: &str) -> String {
    let t = if title.trim().is_empty() { "none" } else { title.trim() };
    format!("title: {t} | text: {text}")
}

pub struct Embedder {
    session: Session,
    threads: usize,
    tokenizer: tokenizers::Tokenizer,
    vision: Option<Session>,
    environment: ort::environment::Environment,
    dir: PathBuf,
}

impl Embedder {
    /// Loads the model installed in `dir` (`<models>/embeddinggemma-2`).
    pub fn load(dir: &Path) -> Result<Embedder, String> {
        Self::load_with_threads(dir, cpu_threads())
    }

    pub fn load_with_threads(dir: &Path, threads: usize) -> Result<Embedder, String> {
        if !models::installed(dir) {
            return Err(models::MODEL_MISSING_MESSAGE.into());
        }
        let environment = ort::init()
            .build()
            .map_err(|e| format!("Couldn't start the model runtime: {e}"))?;
        let session = Session::builder(&environment)
            .map_err(|e| e.to_string())?
            .with_execution_providers([ort::ep::CPU::default().with_arena_allocator(false).build()])
            .map_err(|e| e.to_string())?
            .with_memory_pattern(false)
            .map_err(|e| e.to_string())?
            .with_intra_threads(threads)
            .map_err(|e| e.to_string())?
            .with_inter_threads(1)
            .map_err(|e| e.to_string())?
            .with_intra_op_spinning(false).map_err(|e| e.to_string())?
            .with_inter_op_spinning(false).map_err(|e| e.to_string())?
            .commit_from_file(dir.join("model_q4f16.onnx"))
            .map_err(|e| format!("Couldn't load the model: {e}"))?;
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json"))
            .map_err(|e| format!("Couldn't load the tokenizer: {e}"))?;
        Ok(Embedder { session, threads, tokenizer, vision: None, environment, dir: dir.to_path_buf() })
    }

    /// One vector of `DIM` values for an already prompted text.
    pub fn embed(&mut self, text: &str) -> Result<Vec<f32>, String> {
        let enc = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| e.to_string())?;
        let ids: Vec<i64> = enc.get_ids().iter().map(|x| *x as i64).collect();
        let n = ids.len();
        if n > MAX_INPUT_TOKENS {
            return Err(format!("The input has {n} tokens; the model takes at most {MAX_INPUT_TOKENS}."));
        }
        let mask = vec![1i64; n];
        let tensor_i =
            |v: Vec<i64>| Tensor::from_array(([1usize, n], v)).map_err(|e| e.to_string());
        let empty = || {
            Tensor::from_array(([0usize, FEATURE_DIM as usize], Vec::<f32>::new()))
                .map_err(|e| e.to_string())
        };
        let outputs = self
            .session
            .run(ort::inputs! {
                "input_ids" => tensor_i(ids)?,
                "attention_mask" => tensor_i(mask)?,
                "image_features" => empty()?,
                "video_features" => empty()?,
                "audio_features" => empty()?,
            })
            .map_err(|e| format!("Embedding failed: {e}"))?;
        let (_, data) = outputs["sentence_embedding"]
            .try_extract_tensor::<f32>()
            .map_err(|e| e.to_string())?;
        if data.len() < NATIVE_DIM {
            return Err(format!(
                "The model returned {} values, expected {NATIVE_DIM}.",
                data.len()
            ));
        }
        Ok(truncate_normalize(&data[..NATIVE_DIM]))
    }

    /// Tokens of a text without special tokens (for tests and diagnostics).
    pub fn count_tokens(&self, text: &str) -> Result<usize, String> {
        Ok(self.tokenizer.encode(text, false).map_err(|e| e.to_string())?.get_ids().len())
    }

    /// Tokens of an input as [`Embedder::embed`] runs it (with the special tokens).
    pub fn input_tokens(&self, text: &str) -> Result<usize, String> {
        Ok(self.tokenizer.encode(text, true).map_err(|e| e.to_string())?.get_ids().len())
    }

    /// Cuts text into passages measured in the model's own tokens.
    pub fn chunk_text(&self, text: &str) -> Result<Vec<String>, String> {
        let capped: String = text.chars().take(MAX_DOC_CHARS).collect();
        let enc = self
            .tokenizer
            .encode(capped.as_str(), false)
            .map_err(|e| e.to_string())?;
        Ok(chunk_ranges(&capped, enc.get_offsets())
            .into_iter()
            .filter_map(|(a, b)| capped.get(a..b))
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect())
    }

    pub fn embed_query(&mut self, q: &str) -> Result<Vec<f32>, String> {
        self.embed(&query_prompt(q))
    }

    pub fn embed_document(&mut self, title: &str, text: &str) -> Result<Vec<f32>, String> {
        self.embed(&document_prompt(title, text))
    }

    /// The vector of an image already decoded and cut into patches by the parser process
    /// ([`crate::vision::prepare`], INV-11): this process never decodes image bytes.
    pub fn embed_pixels(&mut self, pixels: &Pixels) -> Result<Vec<f32>, String> {
        if pixels.rgb.len() != PATCHES * PATCH_VALUES || pixels.positions.len() != PATCHES * 2 || pixels.tokens == 0 || pixels.tokens > PATCHES / 9 {
            return Err("Image patches have an unexpected shape".into());
        }
        if self.vision.is_none() {
            self.vision = Some(Session::builder(&self.environment).map_err(|e| e.to_string())?
                .with_execution_providers([ort::ep::CPU::default().with_arena_allocator(false).build()]).map_err(|e| e.to_string())?
                .with_memory_pattern(false).map_err(|e| e.to_string())?
                .with_intra_threads(self.threads).map_err(|e| e.to_string())?
                .with_inter_threads(1).map_err(|e| e.to_string())?
                .with_intra_op_spinning(false).map_err(|e| e.to_string())?
                .with_inter_op_spinning(false).map_err(|e| e.to_string())?
                .commit_from_file(self.dir.join("vision_encoder_q4f16.onnx")).map_err(|e| e.to_string())?);
        }
        let output = self.vision.as_mut().unwrap().run(ort::inputs! {
            "pixel_values" => Tensor::from_array(([1usize, PATCHES, PATCH_VALUES], pixels.values())).map_err(|e| e.to_string())?,
            "pixel_position_ids" => Tensor::from_array(([1usize, PATCHES, 2], pixels.positions.clone())).map_err(|e| e.to_string())?,
        }).map_err(|e| format!("Image encoder failed: {e}"))?;
        let (_, features) = output[0].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        if features.len() != pixels.tokens * FEATURE_DIM as usize { return Err("Image encoder returned an unexpected shape".into()); }
        let features = features.to_vec();
        drop(output);
        // The tokenizer inserts BOS/EOS around the expanded media sequence.
        let begin = self.tokenizer.id_to_token(255999).ok_or("Image start token missing")?;
        let end = self.tokenizer.id_to_token(258882).ok_or("Image end token missing")?;
        let image_token = self.tokenizer.id_to_token(258880).ok_or("Image token missing")?;
        let marker = format!("{begin}{}{end}", image_token.repeat(pixels.tokens));
        let encoded = self.tokenizer.encode(marker, true).map_err(|e| e.to_string())?;
        let ids: Vec<i64> = encoded.get_ids().iter().map(|id| *id as i64).collect();
        if ids.iter().filter(|&&id| id == 258880).count() != pixels.tokens { return Err("Image tokenizer placeholders do not match the encoder".into()); }
        let n = ids.len();
        let empty = || Tensor::from_array(([0usize, FEATURE_DIM as usize], Vec::<f32>::new())).map_err(|e| e.to_string());
        let output = self.session.run(ort::inputs! {
            "input_ids" => Tensor::from_array(([1usize, n], ids)).map_err(|e| e.to_string())?,
            "attention_mask" => Tensor::from_array(([1usize, n], vec![1i64; n])).map_err(|e| e.to_string())?,
            "image_features" => Tensor::from_array(([pixels.tokens, FEATURE_DIM as usize], features)).map_err(|e| e.to_string())?,
            "video_features" => empty()?,
            "audio_features" => empty()?,
        }).map_err(|e| format!("Image embedding failed: {e}"))?;
        let (_, vector) = output["sentence_embedding"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        if vector.len() != NATIVE_DIM { return Err("Image embedding returned an unexpected dimension".into()); }
        Ok(truncate_normalize(vector))
    }
}

pub fn truncate_normalize(v: &[f32]) -> Vec<f32> {
    let mut out: Vec<f32> = v.iter().take(DIM).copied().collect();
    let norm = out.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-9);
    out.iter_mut().for_each(|x| *x /= norm);
    out
}

/// Token windows of at most CHUNK_TOKENS, sharing CHUNK_OVERLAP_TOKENS, as byte ranges of the text.
/// Each window prefers to stop after a sentence or line end in its last quarter, as a recursive
/// splitter would (the original's RecursiveCharacterTextSplitter, BaseTextSplitter sub_180004920).
pub fn chunk_ranges(text: &str, offsets: &[(usize, usize)]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut first = 0usize;
    while first < offsets.len() {
        let mut last = (first + CHUNK_TOKENS).min(offsets.len());
        if last < offsets.len() {
            let floor = first + CHUNK_TOKENS * 3 / 4;
            let ends_sentence = |i: usize| {
                text.get(offsets[i].0..offsets[i].1)
                    .is_some_and(|t| t.ends_with(['.', '!', '?', '\n']))
            };
            if let Some(i) = (floor..last).rev().find(|&i| ends_sentence(i)) {
                last = i + 1;
            }
        }
        out.push((offsets[first].0, offsets[last - 1].1));
        if last >= offsets.len() {
            break;
        }
        first = last.saturating_sub(CHUNK_OVERLAP_TOKENS).max(first + 1);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One fake token per word, with its byte offsets.
    fn word_offsets(text: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut pos = 0;
        for w in text.split_inclusive(' ') {
            out.push((pos, pos + w.trim_end().len()));
            pos += w.len();
        }
        out
    }

    #[test]
    fn chunks_hold_256_tokens_and_share_20() {
        let text: String = (0..1000).map(|i| format!("w{i} ")).collect();
        let off = word_offsets(&text);
        let r = chunk_ranges(&text, &off);
        assert!(r.len() > 3);
        let count = |c: &(usize, usize)| text[c.0..c.1].split_whitespace().count();
        assert_eq!(count(&r[0]), CHUNK_TOKENS);
        assert_eq!(count(&r[0]), 256);
        let tail: Vec<&str> = text[r[0].0..r[0].1]
            .split_whitespace()
            .rev()
            .take(20)
            .collect();
        assert!(
            tail.iter()
                .all(|w| text[r[1].0..r[1].1].split_whitespace().any(|x| x == *w)),
            "neighbours share 20 tokens"
        );
        assert!(
            (0..1000).all(|i| r.iter().any(|c| text[c.0..c.1]
                .split_whitespace()
                .any(|x| x == format!("w{i}")))),
            "no word lost"
        );
    }

    #[test]
    fn chunks_stop_at_a_sentence_when_they_can() {
        let mut text: String = (0..300).map(|i| format!("w{i} ")).collect();
        text = text.replacen("w220 ", "w220. ", 1);
        let r = chunk_ranges(&text, &word_offsets(&text));
        assert!(text[r[0].0..r[0].1].ends_with("w220."));
    }

    #[test]
    fn chunking_short_or_blank_text() {
        assert_eq!(chunk_ranges("hello", &[(0, 5)]), vec![(0, 5)]);
        assert!(chunk_ranges("", &[]).is_empty());
    }

    #[test]
    fn inference_threads_follow_the_machine_within_bounds() {
        assert_eq!(cpu_threads_for(1), MIN_CPU_THREADS, "a tiny machine still gets the floor");
        assert_eq!(cpu_threads_for(2), MIN_CPU_THREADS);
        assert_eq!(cpu_threads_for(3), MIN_CPU_THREADS);
        assert_eq!(cpu_threads_for(4), 2);
        assert_eq!(cpu_threads_for(5), 2);
        assert_eq!(cpu_threads_for(8), 4);
        assert_eq!(cpu_threads_for(12), 6);
        assert_eq!(cpu_threads_for(22), MAX_CPU_THREADS, "a big machine is capped");
        assert_eq!(cpu_threads_for(128), MAX_CPU_THREADS);
        assert_eq!(cpu_threads_for(256), MAX_CPU_THREADS);
        assert!((MIN_CPU_THREADS..=MAX_CPU_THREADS).contains(&cpu_threads()), "whatever the host");
    }

    #[test]
    fn truncation_gives_unit_vector() {
        let v: Vec<f32> = (0..NATIVE_DIM).map(|i| i as f32 + 1.0).collect();
        let t = truncate_normalize(&v);
        assert_eq!(t.len(), DIM);
        assert!((t.iter().map(|x| x * x).sum::<f32>() - 1.0).abs() < 1e-4);
    }

    #[test]
    fn prompts_follow_the_model_card() {
        assert_eq!(query_prompt("budget"), "task: search result | query: budget");
        assert_eq!(document_prompt("  ", "body"), "title: none | text: body");
        assert_eq!(document_prompt(" Invoice ", "body"), "title: Invoice | text: body");
    }

    /// Needs the real model: `QAMILLE_MODELS=<data>/models cargo test real_model -- --ignored`.
    #[test]
    #[ignore]
    fn real_model_cuts_long_text_in_256_tokens() {
        let dir = std::path::PathBuf::from(std::env::var("QAMILLE_MODELS").expect("QAMILLE_MODELS")).join(models::MODEL_ID);
        let m = Embedder::load(&dir).unwrap();
        let text = "La fattura di ottobre è già stata pagata, città di Cefalù. ".repeat(200);
        let parts = m.chunk_text(&text).unwrap();
        assert!(parts.len() > 2);
        for p in &parts {
            let n = m.count_tokens(p).unwrap();
            assert!(n <= CHUNK_TOKENS + 2, "{n} tokens");
        }
    }
}
