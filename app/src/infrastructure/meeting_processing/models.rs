//! The Whisper models as the app shows them. The files, the runtime and their download live in
//! the compute server (`qamille-compute::speech`, spec D3); the app lists them from `GET /health`.
//! The administrator of the server installs them: the app installs nothing (INV-41).
use crate::{
    application::{
        AppError, AppResult,
        compute::{ComputeGateway, SPEECH_MODEL_PREFIX},
    },
    domain::meeting::SpeechModel,
};

pub(super) struct ModelInfo {
    pub id: &'static str,
    pub name: &'static str,
    /// SHA-256 of the ggml file (the compute server checks it). It is part of the cache key of
    /// the transcribed windows since before the server, so those entries stay valid.
    pub sha256: &'static str,
}

const CATALOG: &[ModelInfo] = &[
    ModelInfo { id: "tiny", name: "Whisper Tiny · multilingue", sha256: "be07e048e1e599ad46341c8d2a135645097a538221678b7acdd1b1919c6e1b21" },
    ModelInfo { id: "base", name: "Whisper Base · multilingue", sha256: "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe" },
    ModelInfo { id: "small", name: "Whisper Small · multilingue", sha256: "1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b" },
];


pub(super) fn model_info(id: &str) -> AppResult<&'static ModelInfo> {
    CATALOG.iter().find(|m| m.id == id).ok_or_else(|| AppError::validation("Modello di trascrizione non supportato."))
}

/// `base` → `whisper-base`, the id of the compute server.
pub(super) fn server_id(id: &str) -> String {
    format!("{SPEECH_MODEL_PREFIX}{id}")
}

pub(super) fn list(compute: &dyn ComputeGateway) -> AppResult<Vec<SpeechModel>> {
    let health = compute.health()?;
    Ok(CATALOG
        .iter()
        .map(|m| {
            let served = health.model(&server_id(m.id));
            SpeechModel {
                id: m.id.into(),
                name: m.name.into(),
                bytes: served.map_or(0, |s| s.total_bytes),
                installed: served.is_some_and(|s| s.state == "ready"),
                license: "MIT · OpenAI Whisper / whisper.cpp".into(),
            }
        })
        .collect())
}

/// Whether the compute server can transcribe with `id` now.
pub(super) fn ready(compute: &dyn ComputeGateway, id: &str) -> AppResult<bool> {
    Ok(compute.health()?.model(&server_id(id)).is_some_and(|m| m.state == "ready"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::meeting::LOCAL_SPEECH_MODELS;
    use crate::infrastructure::fake_compute::FakeCompute;
    #[test]
    fn settings_can_only_choose_models_the_catalog_has() {
        let catalog: Vec<&str> = CATALOG.iter().map(|m| m.id).collect();
        assert_eq!(catalog, LOCAL_SPEECH_MODELS);
        assert!(model_info("../secret").is_err());
        assert_eq!(server_id("base"), "whisper-base");
    }

    #[test]
    fn the_list_comes_from_the_compute_server() {
        let compute = FakeCompute::default();
        let models = list(&compute).unwrap();
        assert_eq!(models.iter().map(|m| (m.id.as_str(), m.installed)).collect::<Vec<_>>(), [("tiny", false), ("base", false), ("small", false)]);
        assert_eq!(models[1].bytes, 147_951_465);
        // What the administrator installs on the server shows up in the list; the app installs nothing.
        compute.speech_ready.lock().unwrap().push("whisper-base".into());
        assert!(list(&compute).unwrap()[1].installed);
    }
}
