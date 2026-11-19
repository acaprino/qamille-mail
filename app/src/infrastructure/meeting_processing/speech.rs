//! Transcription of a meeting in windows of 15 s (16 kHz mono, 500 ms of overlap), each one
//! cached in `<meeting>/.stt-cache` (spec D3: the windows, the cache and the jobs stay in the
//! app). Two engines:
//! - «Server di calcolo» (`compute`): `POST /v1/audio/transcriptions` of the compute server
//!   through [`ComputeGateway`]; whisper.cpp runs there;
//! - «Esterno compatibile OpenAI» (`external`): the same route of `ai_base_url` with `ai_key`.
//!
//! The cache key holds the audio, the model, the language and the endpoint's *identity*:
//! `local` for the local server whatever its port (INV-8, EX-8), the address of a remote one, the
//! configured URL of the external service. Keys of the windows transcribed before the compute
//! server (`local` and `remote` engines) are computed the same way, so they stay valid.
use super::{cancelled, models, processing_error, unique_id};
use crate::{
    application::{
        AppError, AppResult,
        compute::{ComputeGateway, LOCAL_IDENTITY, SpeechSegment},
    },
    domain::meeting::*,
    infrastructure::meeting_audio::mix_audio,
    model::{AiProvider, Settings},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{fs, io::Write, path::Path, sync::atomic::AtomicBool, time::Duration};

/// The recognizer contract of the cached windows (unchanged since whisper.cpp ran in the app).
const RECOGNIZER: &str = "whisper.cpp-v1.9.2";

enum Engine {
    Compute { model: String, identity: String },
    External,
}

pub(super) fn transcribe(
    compute: &dyn ComputeGateway,
    folder: &Path,
    chunks: &[AudioChunk],
    options: &TranscribeOptions,
    settings: &Settings,
    cancel: &AtomicBool,
    progress: &dyn Fn(u32),
    authorize: &dyn Fn() -> AppResult<()>,
) -> AppResult<TranscriptVersion> {
    cancelled(cancel)?;
    if chunks.is_empty() {
        return Err(AppError::validation(
            "Non ci sono blocchi audio da trascrivere.",
        ));
    }
    if !valid_transcription_language(&options.language) {
        return Err(AppError::validation("Lingua di trascrizione non valida."));
    }
    let duration = chunks.iter().map(|c| c.end_ms).max().unwrap_or(0);
    if duration == 0 {
        return Err(AppError::validation("La registrazione è vuota."));
    }
    let engine = match canonical_engine(&options.provider) {
        ENGINE_COMPUTE => {
            models::model_info(&options.model)?;
            // The endpoint is the one of this job, chosen once (D3: a job ends where it began).
            let identity = compute.identity();
            if identity != LOCAL_IDENTITY {
                authorize()?;
            }
            if !models::ready(compute, &options.model)? {
                return Err(AppError::new(
                    "model_missing",
                    "Installa il modello locale prima della trascrizione.",
                ));
            }
            Engine::Compute { model: models::server_id(&options.model), identity }
        }
        ENGINE_EXTERNAL => {
            if settings.ai_provider != AiProvider::OpenAiCompatible {
                return Err(AppError::validation(
                    "La trascrizione remota richiede un endpoint compatibile OpenAI configurato.",
                ));
            }
            if !valid_remote_model_name(remote_model(&options.model)) {
                return Err(AppError::validation("Nome del modello remoto non valido."));
            }
            Engine::External
        }
        _ => {
            return Err(AppError::validation(
                "Provider di trascrizione sconosciuto.",
            ));
        }
    };
    let cache_folder = folder.join(".stt-cache");
    fs::create_dir_all(&cache_folder)?;
    let mut segments = Vec::new();
    let mut offset = 0_u64;
    while offset < duration {
        cancelled(cancel)?;
        let end = (offset + 15_000).min(duration);
        let samples = mix_audio(folder, chunks, offset, end, 16_000)?;
        let identity = match &engine {
            Engine::Compute { identity, .. } => Some(identity.as_str()),
            Engine::External => None,
        };
        let key = window_key(&samples, options, settings, identity, offset, end);
        let cached = read_window_cache(&cache_folder, &key, offset, end);
        let parsed = if let Some(cached) = cached {
            cached
        } else if has_signal(&samples) {
            let wave = wav_bytes(&samples)?;
            let parsed = match &engine {
                Engine::Compute { model, identity } => {
                    if identity != LOCAL_IDENTITY {
                        authorize()?;
                    }
                    let rows = compute.transcribe_window(&wave, model, &options.language, cancel)?;
                    window_segments(&rows, offset, end)
                }
                Engine::External => remote_window(
                    &wave,
                    settings,
                    remote_model(&options.model),
                    &options.language,
                    offset,
                    end,
                    cancel,
                    authorize,
                )?,
            };
            cancelled(cancel)?;
            write_window_cache(&cache_folder, &key, &parsed)?;
            parsed
        } else {
            write_window_cache(&cache_folder, &key, &[])?;
            Vec::new()
        };
        append_segments(&mut segments, parsed);
        progress(((end * 100 / duration).min(99)) as u32);
        if end == duration {
            break;
        }
        offset = end - 500;
    }
    cancelled(cancel)?;
    for (index, segment) in segments.iter_mut().enumerate() {
        segment.id = format!("segment-{}", index + 1);
    }
    progress(100);
    Ok(TranscriptVersion {
        id: String::new(),
        created_at: crate::model::now_ts(),
        model: match engine {
            Engine::Compute { .. } => format!("{RECOGNIZER}/{}", options.model),
            Engine::External => format!("remote/{}", remote_model(&options.model)),
        },
        language: options.language.clone(),
        segments,
        complete: true,
    })
}

/// A 16-bit PCM WAV of one window, in memory.
fn wav_bytes(samples: &[f32]) -> AppResult<Vec<u8>> {
    let spec = hound::WavSpec { channels: 1, sample_rate: 16_000, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut bytes = std::io::Cursor::new(Vec::with_capacity(samples.len() * 2 + 44));
    let mut writer = hound::WavWriter::new(&mut bytes, spec).map_err(|e| AppError::storage(e.to_string()))?;
    for sample in samples {
        if !sample.is_finite() {
            return Err(processing_error("Campione audio non valido."));
        }
        writer.write_sample((sample.clamp(-1.0, 1.0) * i16::MAX as f32) as i16).map_err(|e| AppError::storage(e.to_string()))?;
    }
    writer.finalize().map_err(|e| AppError::storage(e.to_string()))?;
    Ok(bytes.into_inner())
}

/// Segments of the server (ms from the window start) placed in the meeting and clamped to it.
fn window_segments(rows: &[SpeechSegment], start: u64, end: u64) -> Vec<TranscriptSegment> {
    let rows: Vec<Value> = rows.iter().map(|row| json!({"text": row.text, "offsets": {"from": row.start_ms, "to": row.end_ms}})).collect();
    // The rows were checked by the server; a malformed one is simply left out.
    parse_local(&json!({"transcription": rows}), start, end).unwrap_or_default()
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CachedWindow {
    key: String,
    segments: Vec<TranscriptSegment>,
}

fn window_key(
    samples: &[f32],
    options: &TranscribeOptions,
    settings: &Settings,
    identity: Option<&str>,
    start: u64,
    end: u64,
) -> String {
    let mut digest = Sha256::new();
    // Version the recognition/preprocessing contract so changed algorithms invalidate cache.
    // The prefix keeps the name from before the rebranding: it is part of the cache key,
    // and changing it would discard every transcription already cached.
    digest.update(format!("sparklone-stt-window-v1/{RECOGNIZER}/16000/").as_bytes());
    let fields = match identity {
        Some(identity) => {
            if let Ok(model) = models::model_info(&options.model) {
                digest.update(model.sha256.as_bytes());
            }
            // `local` is also what the engine of this computer was called before the server.
            json!([identity, options.model, options.language, start, end])
        }
        None => {
            // Only the endpoint identity is hashed; credentials are never cached or logged.
            json!([
                LEGACY_ENGINE_REMOTE,
                remote_model(&options.model),
                options.language,
                settings.ai_base_url.trim().trim_end_matches('/'),
                start,
                end
            ])
        }
    };
    digest.update(fields.to_string().as_bytes());
    for sample in samples {
        digest.update(sample.to_le_bytes());
    }
    format!("{:x}", digest.finalize())
}

fn read_window_cache(
    folder: &Path,
    key: &str,
    start: u64,
    end: u64,
) -> Option<Vec<TranscriptSegment>> {
    let path = folder.join(format!("{key}.json"));
    if fs::metadata(&path).ok()?.len() > 4 * 1024 * 1024 {
        return None;
    }
    let cached: CachedWindow = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    if cached.key != key
        || cached.segments.iter().any(|segment| {
            segment.start_ms < start
                || segment.end_ms > end
                || segment.end_ms <= segment.start_ms
                || segment.text.len() > 100_000
                || segment.confidence.is_some_and(|value| !value.is_finite())
        })
    {
        return None;
    }
    Some(cached.segments)
}

fn write_window_cache(folder: &Path, key: &str, segments: &[TranscriptSegment]) -> AppResult<()> {
    let destination = folder.join(format!("{key}.json"));
    let staging = folder.join(format!("{key}.{}.part", unique_id()?));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)?;
        let bytes = serde_json::to_vec(&CachedWindow {
            key: key.into(),
            segments: segments.to_vec(),
        })
        .map_err(|_| processing_error("Impossibile salvare il blocco di trascrizione."))?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        // An invalid cache entry may exist after disk corruption or an older interrupted run.
        if destination.exists() {
            fs::remove_file(&destination)?;
        }
        fs::rename(&staging, destination)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(staging);
    }
    result
}

fn has_signal(samples: &[f32]) -> bool {
    if samples.is_empty() {
        return false;
    }
    let energy = samples.iter().map(|v| (*v as f64).powi(2)).sum::<f64>() / samples.len() as f64;
    energy.sqrt() >= 0.0005 && samples.iter().any(|v| v.abs() >= 0.002)
}

fn parse_local(response: &Value, start: u64, end: u64) -> AppResult<Vec<TranscriptSegment>> {
    let rows = response["transcription"].as_array().ok_or_else(|| {
        processing_error("La risposta locale non contiene una trascrizione valida.")
    })?;
    let mut out = Vec::new();
    for row in rows {
        let text = row["text"].as_str().unwrap_or("").trim();
        if text.is_empty() || text.starts_with("[_") || text == "[BLANK_AUDIO]" {
            continue;
        }
        let from = row["offsets"]["from"]
            .as_u64()
            .ok_or_else(|| processing_error("Timestamp del motore non valido."))?;
        let to = row["offsets"]["to"]
            .as_u64()
            .ok_or_else(|| processing_error("Timestamp del motore non valido."))?;
        let begin = start.saturating_add(from).min(end);
        let finish = start.saturating_add(to).min(end);
        if finish <= begin {
            continue;
        }
        out.push(TranscriptSegment {
            start_ms: begin,
            end_ms: finish,
            text: text.into(),
            original_text: text.into(),
            source: "mixed".into(),
            confidence: None,
            ..Default::default()
        });
    }
    Ok(out)
}

fn remote_window(
    bytes: &[u8],
    settings: &Settings,
    model: &str,
    language: &str,
    start: u64,
    end: u64,
    cancel: &AtomicBool,
    authorize: &dyn Fn() -> AppResult<()>,
) -> AppResult<Vec<TranscriptSegment>> {
    cancelled(cancel)?;
    // One bounded 15-second, 16 kHz mono window.
    if bytes.len() > 1_000_000 {
        return Err(processing_error(
            "Blocco audio remoto oltre il limite interno.",
        ));
    }
    let boundary = format!("qamille-{}", unique_id()?);
    let mut body = Vec::with_capacity(bytes.len() + 2048);
    let response_format = if model.starts_with("gpt-") {
        "json"
    } else {
        "verbose_json"
    };
    for (field, value) in [("model", model), ("response_format", response_format)] {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{field}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    if language != "auto" {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"language\"\r\n\r\n{language}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"audio.wav\"\r\nContent-Type: audio/wav\r\n\r\n").as_bytes());
    body.extend_from_slice(bytes);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    let base = if settings.ai_base_url.trim().is_empty() {
        "https://api.openai.com/v1"
    } else {
        settings.ai_base_url.trim().trim_end_matches('/')
    };
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(90)))
        .http_status_as_error(false)
        .build()
        .into();
    let mut request = agent.post(format!("{base}/audio/transcriptions")).header(
        "Content-Type",
        format!("multipart/form-data; boundary={boundary}"),
    );
    if !settings.ai_key.trim().is_empty() {
        request = request.header(
            "Authorization",
            format!("Bearer {}", settings.ai_key.trim()),
        );
    }
    authorize()?;
    let mut response = request
        .send(&body[..])
        .map_err(|_| processing_error("Il servizio di trascrizione non è raggiungibile."))?;
    cancelled(cancel)?;
    if response.status().as_u16() >= 400 {
        return Err(processing_error(format!(
            "Il servizio di trascrizione ha risposto con errore {}. L'audio rimane salvato.",
            response.status().as_u16()
        )));
    }
    let value: Value = response
        .body_mut()
        .read_json()
        .map_err(|_| processing_error("Risposta del servizio di trascrizione non valida."))?;
    parse_remote(&value, start, end)
}

fn remote_model(configured: &str) -> &str {
    match configured.trim() {
        "" | "tiny" | "base" | "small" => "whisper-1",
        name => name,
    }
}

fn parse_remote(value: &Value, start: u64, end: u64) -> AppResult<Vec<TranscriptSegment>> {
    let rows = if let Some(items) = value["segments"].as_array() {
        items.iter().map(|r| json!({"text":r["text"], "offsets":{"from":(r["start"].as_f64().unwrap_or(0.0).max(0.0)*1000.0) as u64,
            "to":(r["end"].as_f64().unwrap_or((end-start) as f64/1000.0).max(0.0)*1000.0) as u64}})).collect::<Vec<_>>()
    } else if let Some(text) = value["text"].as_str() {
        vec![json!({"text":text,"offsets":{"from":0,"to":end-start}})]
    } else {
        return Err(processing_error(
            "Il servizio non ha restituito testo o segmenti.",
        ));
    };
    parse_local(&json!({"transcription":rows}), start, end)
}

fn append_segments(target: &mut Vec<TranscriptSegment>, segments: Vec<TranscriptSegment>) {
    for mut segment in segments {
        if let Some(previous) = target.last() {
            if segment.end_ms <= previous.end_ms {
                continue;
            }
            if segment.start_ms < previous.end_ms {
                segment.text = remove_overlap(&previous.text, &segment.text);
                if segment.text.is_empty() {
                    continue;
                }
                segment.start_ms = previous.end_ms;
            }
        }
        target.push(segment);
    }
}
fn remove_overlap(previous: &str, next: &str) -> String {
    let before: Vec<_> = previous.split_whitespace().collect();
    let after: Vec<_> = next.split_whitespace().collect();
    for size in (1..=before.len().min(after.len()).min(20)).rev() {
        if before[before.len() - size..]
            .iter()
            .zip(&after[..size])
            .all(|(a, b)| {
                a.trim_matches(|c: char| c.is_ascii_punctuation())
                    .eq_ignore_ascii_case(b.trim_matches(|c: char| c.is_ascii_punctuation()))
            })
        {
            return after[size..].join(" ");
        }
    }
    next.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::write_wav;
    #[test]
    fn silence_is_not_sent_to_the_recognizer() {
        assert!(!has_signal(&vec![0.0; 16000]));
        assert!(has_signal(&[0.1, -0.1]));
    }
    #[test]
    fn server_segments_are_placed_in_the_meeting_and_clamped() {
        let rows = [
            SpeechSegment { start_ms: 50, end_ms: 3000, text: "ciao".into() },
            SpeechSegment { start_ms: 3000, end_ms: 4000, text: "[BLANK_AUDIO]".into() },
        ];
        let placed = window_segments(&rows, 10_000, 12_000);
        assert_eq!(placed.len(), 1);
        assert_eq!((placed[0].start_ms, placed[0].end_ms, placed[0].text.as_str()), (10_050, 12_000, "ciao"));
    }
    #[test]
    fn cli_timestamps_are_clamped_to_the_window() {
        let response = json!({"transcription":[{"text":"ciao","offsets":{"from":50,"to":3000}}]});
        let rows = parse_local(&response, 10000, 12000).unwrap();
        assert_eq!((rows[0].start_ms, rows[0].end_ms), (10050, 12000));
        assert!(rows[0].confidence.is_none());
    }
    #[test]
    fn overlapping_text_is_not_duplicated() {
        assert_eq!(
            remove_overlap("Una riunione domani mattina", "domani mattina alle nove"),
            "alle nove"
        );
        let mut old = vec![TranscriptSegment {
            text: "Domani mattina".into(),
            end_ms: 15000,
            ..Default::default()
        }];
        append_segments(
            &mut old,
            vec![TranscriptSegment {
                text: "mattina alle nove".into(),
                start_ms: 14500,
                end_ms: 17000,
                ..Default::default()
            }],
        );
        assert_eq!(old[1].text, "alle nove");
        assert_eq!(old[1].start_ms, 15000);
    }

    #[test]
    fn cache_identity_tracks_audio_model_language_and_endpoint_without_credentials() {
        let mut settings = Settings::default();
        let mut options = TranscribeOptions::default();
        let local = Some(LOCAL_IDENTITY);
        let original = window_key(&[0.1], &options, &settings, local, 0, 1000);
        assert_ne!(original, window_key(&[0.2], &options, &settings, local, 0, 1000));
        assert_ne!(original, window_key(&[0.1], &options, &settings, Some("https://compute.example"), 0, 1000));
        options.language = "en".into();
        assert_ne!(original, window_key(&[0.1], &options, &settings, local, 0, 1000));
        options.language = "it".into();
        options.model = "small".into();
        assert_ne!(original, window_key(&[0.1], &options, &settings, local, 0, 1000));
        options.provider = ENGINE_EXTERNAL.into();
        let remote = window_key(&[0.1], &options, &settings, None, 0, 1000);
        settings.ai_key = "private-token-never-cached".into();
        assert_eq!(remote, window_key(&[0.1], &options, &settings, None, 0, 1000));
        settings.ai_base_url = "https://transcription.example/v1".into();
        assert_ne!(remote, window_key(&[0.1], &options, &settings, None, 0, 1000));
    }

    #[test]
    fn windows_cached_before_the_compute_server_keep_their_keys() {
        // Computed with the formula of the engines `local` (whisper.cpp in the app) and `remote`.
        let options = TranscribeOptions::default();
        assert_eq!(
            window_key(&[0.1], &options, &Settings::default(), Some(LOCAL_IDENTITY), 0, 1000),
            "d99198d3faddceb6ff0a1cb0241ab40d573a63dcdc1a8655eeb8c27f92ddf1a2"
        );
        let settings = Settings { ai_base_url: "https://transcription.example/v1/".into(), ..Default::default() };
        let external = TranscribeOptions { provider: ENGINE_EXTERNAL.into(), ..Default::default() };
        assert_eq!(
            window_key(&[0.1], &external, &settings, None, 0, 1000),
            "aad7d3f5e70e01a5bb95530faf70d1953cf3e4db0b1be301281101e7c37596c7"
        );
    }

    fn recording(name: &str, seconds: u64) -> (std::path::PathBuf, Vec<AudioChunk>) {
        let root = std::env::temp_dir().join(format!("qamille-{name}-{}", unique_id().unwrap()));
        fs::create_dir(&root).unwrap();
        let samples: Vec<f32> = (0..16_000 * seconds).map(|i| ((i % 80) as f32 / 80.0 - 0.5) * 0.4).collect();
        write_wav(&root.join("source.wav"), &samples, 16_000).unwrap();
        let chunks = vec![AudioChunk { source: "import".into(), path: "source.wav".into(), start_ms: 0, end_ms: seconds * 1000, sample_rate: 16_000, channels: 1, ..Default::default() }];
        (root, chunks)
    }

    fn compute_options() -> TranscribeOptions {
        TranscribeOptions { provider: ENGINE_COMPUTE.into(), model: "base".into(), language: "it".into() }
    }

    #[test]
    fn the_compute_engine_sends_each_window_through_the_gateway() {
        use crate::infrastructure::fake_compute::FakeCompute;
        let (root, chunks) = recording("compute-windows", 20);
        let compute = FakeCompute::with_identity(LOCAL_IDENTITY);
        let refused = transcribe(&compute, &root, &chunks, &compute_options(), &Settings::default(), &AtomicBool::new(false), &|_| {}, &|| Ok(())).unwrap_err();
        assert_eq!(refused.code, "model_missing", "a model the server has not installed");
        compute.speech_ready.lock().unwrap().push("whisper-base".into());
        let version = transcribe(&compute, &root, &chunks, &compute_options(), &Settings::default(), &AtomicBool::new(false), &|_| {}, &|| panic!("the local server needs no grant")).unwrap();
        let windows: Vec<_> = compute.calls().into_iter().filter(|c| c.starts_with("transcribe")).collect();
        assert_eq!(windows, ["transcribe:whisper-base:it", "transcribe:whisper-base:it"], "0–15 s and 14.5–20 s");
        assert_eq!(version.model, "whisper.cpp-v1.9.2/base");
        assert_eq!(version.segments[0].text, "finestra di 15000 ms");
        assert_eq!((version.segments[1].start_ms, version.segments[1].end_ms), (15_000, 20_000));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cached_windows_are_recognized_after_a_restart_on_another_port() {
        // EX-8: the local server starts again on another port; its identity is still `local`.
        use crate::infrastructure::fake_compute::FakeCompute;
        let (root, chunks) = recording("compute-restart", 16);
        let first = FakeCompute::with_identity(LOCAL_IDENTITY);
        first.speech_ready.lock().unwrap().push("whisper-base".into());
        let before = transcribe(&first, &root, &chunks, &compute_options(), &Settings::default(), &AtomicBool::new(false), &|_| {}, &|| Ok(())).unwrap();
        assert_eq!(first.calls().iter().filter(|c| c.starts_with("transcribe")).count(), 2);
        let restarted = FakeCompute::with_identity(LOCAL_IDENTITY);
        restarted.speech_ready.lock().unwrap().push("whisper-base".into());
        let after = transcribe(&restarted, &root, &chunks, &compute_options(), &Settings::default(), &AtomicBool::new(false), &|_| {}, &|| Ok(())).unwrap();
        assert_eq!(restarted.calls().iter().filter(|c| c.starts_with("transcribe")).count(), 0, "every window came from the cache");
        assert_eq!(before.segments.iter().map(|s| &s.text).collect::<Vec<_>>(), after.segments.iter().map(|s| &s.text).collect::<Vec<_>>());
        // Another server is another identity: nothing of the cache is used for it.
        let remote = FakeCompute::with_identity("https://compute.example");
        remote.speech_ready.lock().unwrap().push("whisper-base".into());
        transcribe(&remote, &root, &chunks, &compute_options(), &Settings::default(), &AtomicBool::new(false), &|_| {}, &|| Ok(())).unwrap();
        assert_eq!(remote.calls().iter().filter(|c| c.starts_with("transcribe")).count(), 2);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_remote_compute_server_rechecks_the_grant_before_each_window() {
        use crate::infrastructure::fake_compute::FakeCompute;
        let (root, chunks) = recording("compute-grant", 16);
        let remote = FakeCompute::with_identity("https://compute.example");
        remote.speech_ready.lock().unwrap().push("whisper-base".into());
        let checks = std::sync::atomic::AtomicUsize::new(0);
        let authorize = || {
            if checks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) >= 2 { Err(AppError::new("permission_denied", "revoked")) } else { Ok(()) }
        };
        let error = transcribe(&remote, &root, &chunks, &compute_options(), &Settings::default(), &AtomicBool::new(false), &|_| {}, &authorize).unwrap_err();
        assert_eq!(error.code, "permission_denied");
        assert_eq!(remote.calls().iter().filter(|c| c.starts_with("transcribe")).count(), 1, "the grant was revoked after the first window");
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn cache_round_trips_and_rejects_invalid_windows() {
        let root =
            std::env::temp_dir().join(format!("qamille-stt-cache-{}", unique_id().unwrap()));
        fs::create_dir(&root).unwrap();
        let segment = TranscriptSegment {
            text: "ciao".into(),
            start_ms: 10,
            end_ms: 100,
            ..Default::default()
        };
        write_window_cache(&root, "test", &[segment]).unwrap();
        assert_eq!(
            read_window_cache(&root, "test", 0, 1000).unwrap()[0].text,
            "ciao"
        );
        assert!(read_window_cache(&root, "test", 50, 1000).is_none());
        fs::write(root.join("test.json"), b"interrupted").unwrap();
        assert!(read_window_cache(&root, "test", 0, 1000).is_none());
        fs::remove_file(root.join("test.json")).unwrap();
        fs::remove_dir(root).unwrap();
    }

    #[test]
    fn remote_responses_preserve_segments_or_use_explicit_window_times() {
        let rows = parse_remote(
            &json!({"segments":[{"text":"ciao","start":0.5,"end":1.5}]}),
            10_000,
            25_000,
        )
        .unwrap();
        assert_eq!((rows[0].start_ms, rows[0].end_ms), (10_500, 11_500));
        let text = parse_remote(&json!({"text":"solo testo"}), 10_000, 25_000).unwrap();
        assert_eq!((text[0].start_ms, text[0].end_ms), (10_000, 25_000));
        assert!(parse_remote(&json!({"error":"invalid"}), 0, 15_000).is_err());
        assert_eq!(remote_model("base"), "whisper-1");
        assert_eq!(remote_model("my-whisper-model"), "my-whisper-model");
    }

    #[test]
    fn transcription_rechecks_authorization_before_each_remote_window() {
        use std::sync::atomic::Ordering;
        let server = super::super::http_fixture::Server::new(r#"{"text":"synthetic speech"}"#.into(), 200, 1);
        let root = std::env::temp_dir().join(format!("qamille-revoked-audio-{}", unique_id().unwrap()));
        fs::create_dir(&root).unwrap();
        write_wav(&root.join("source.wav"), &vec![0.2; 16_000 * 16], 16_000).unwrap();
        let chunks = vec![AudioChunk { source: "import".into(), path: "source.wav".into(), start_ms: 0, end_ms: 16_000, sample_rate: 16_000, channels: 1, ..Default::default() }];
        let settings = Settings { ai_provider: AiProvider::OpenAiCompatible, ai_base_url: server.url.clone(), ..Default::default() };
        let options = TranscribeOptions { provider: ENGINE_EXTERNAL.into(), language: "en".into(), ..Default::default() };
        let compute = crate::infrastructure::fake_compute::FakeCompute::default();
        let error = transcribe(&compute, &root, &chunks, &options, &settings, &AtomicBool::new(false), &|_| {}, &|| server.authorize()).unwrap_err();
        assert_eq!(error.code, "permission_denied");
        assert_eq!(server.calls.load(Ordering::SeqCst), 1);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn remote_request_uses_configured_endpoint_key_model_and_language() {
        use std::{io::Read, net::TcpListener, time::Instant};
        let root =
            std::env::temp_dir().join(format!("qamille-remote-test-{}", unique_id().unwrap()));
        fs::create_dir(&root).unwrap();
        let wave = wav_bytes(&vec![0.01; 1600]).unwrap();
        for (model, expected_format) in [
            ("custom-whisper", "verbose_json"),
            ("gpt-4o-transcribe", "json"),
        ] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let address = listener.local_addr().unwrap();
            let server = std::thread::spawn(move || {
                let began = Instant::now();
                let mut stream = loop {
                    match listener.accept() {
                        Ok((stream, _)) => break stream,
                        Err(error)
                            if error.kind() == std::io::ErrorKind::WouldBlock
                                && began.elapsed() < Duration::from_secs(5) =>
                        {
                            std::thread::sleep(Duration::from_millis(10))
                        }
                        Err(error) => panic!("local test server unavailable: {error}"),
                    }
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0_u8; 4096];
                loop {
                    let count = stream.read(&mut buffer).unwrap();
                    assert!(count > 0);
                    request.extend_from_slice(&buffer[..count]);
                    assert!(request.len() < 1_000_000);
                    if let Some(header_end) = request.windows(4).position(|v| v == b"\r\n\r\n") {
                        let headers =
                            String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
                        let content_length: usize = headers
                            .lines()
                            .find_map(|line| line.strip_prefix("content-length:"))
                            .unwrap()
                            .trim()
                            .parse()
                            .unwrap();
                        if request.len() >= header_end + 4 + content_length {
                            break;
                        }
                    }
                }
                let response = r#"{"text":"audio sintetico"}"#;
                write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
                request
            });
            let settings = Settings {
                ai_base_url: format!("http://{address}/v1"),
                ai_key: "synthetic-test-key".into(),
                ..Settings::default()
            };
            let rows = remote_window(
                &wave,
                &settings,
                model,
                "it",
                500,
                600,
                &AtomicBool::new(false),
                &|| Ok(()),
            )
            .unwrap();
            assert_eq!(rows[0].text, "audio sintetico");
            let request = server.join().unwrap();
            let request = String::from_utf8_lossy(&request);
            assert!(request.starts_with("POST /v1/audio/transcriptions HTTP/1.1\r\n"));
            assert!(request
                .to_ascii_lowercase()
                .contains("authorization: bearer synthetic-test-key\r\n"));
            for (field, value) in [
                ("model", model),
                ("language", "it"),
                ("response_format", expected_format),
            ] {
                assert!(request.contains(&format!("name=\"{field}\"\r\n\r\n{value}\r\n")));
            }
            assert!(request.contains("filename=\"audio.wav\""));
            assert!(!request.contains("synthetic.wav"));
        }
        fs::remove_dir(root).unwrap();
    }
}
