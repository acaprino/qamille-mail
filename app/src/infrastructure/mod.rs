//! Concrete adapters for storage, Gmail, credentials, AI, search and audio.
pub mod mail;
mod mail_reader;
pub mod oauth;
pub mod google_oauth_config;
pub mod ai;
pub mod attach;
pub mod rag;
pub mod recorder;
pub mod secret;
#[cfg(windows)]
pub mod dpapi;
pub mod browser_sign_in;
pub mod server_account;
pub mod sync_http;
pub mod sync_state;
pub mod legacy_local;
pub mod tls;
mod legacy_store;
pub mod activity_db;
pub mod repository;
pub mod sqlite_store;
pub mod store_migration;
pub mod contact_repository;
pub mod google_people;
pub mod gateways;
pub mod idle;
pub mod mail_cache;
mod mail_cache_download;
pub mod timezone;
pub mod search_support;
pub mod meeting_repository;
pub mod meeting_audio;
pub mod meeting_processing;
pub mod meeting_gateways;
pub mod storage_fs;
#[cfg(windows)]
pub(crate) mod win_path;
pub mod compute_factory;
#[cfg(any(test, feature = "test-support"))]
pub mod fake_compute;
pub mod http_compute;

pub mod mail_index_source;
pub mod rag_segments;
pub mod lexical_index;
pub mod mcp_config;
pub mod mailbox_access;
pub mod assistant_install;
pub mod assistant_packages;
pub mod google_calendar;
pub mod gmail_drafts;
pub mod workspace_guard;
pub mod provider_registry;
pub mod portable_files;
