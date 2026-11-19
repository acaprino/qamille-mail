//! `qamille-compute`: the inference of Qamille, used only by `qamille-server` and its
//! tests (spec D3, INV-1: the desktop executable never links it).
//!
//! - [`embedder`]: EmbeddingGemma 2 on ONNX Runtime, passages in the model's own tokens;
//! - [`vision`]: image preprocessing for the vision encoder;
//! - [`extract`]: document readers (PDF, Office, OpenDocument, text);
//! - [`frames`]: the sacrificial parser process and its frame protocol (INV-11);
//! - [`download`]: the downloader shared by the installers (resume, size, SHA-256);
//! - [`llama`]: the pinned llama.cpp engine (`llama-server`), its Windows bundle and its resolution;
//! - [`llm_catalog`]: the pinned catalog of LLM models and their installation;
//! - [`models`]: installation of the model files;
//! - [`speech`]: whisper.cpp (`whisper-cli`), its runtime and its ggml models;
//! - [`memory`]: memory and disk probes.

pub mod download;
pub mod embedder;
pub mod extract;
pub mod frames;
pub mod llama;
pub mod llm_catalog;
pub mod memory;
pub mod models;
pub mod speech;
pub mod vision;
