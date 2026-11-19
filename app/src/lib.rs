//! Shared Rust engine. Desktop adapters depend on application services; services
//! depend on ports and domain rules, never on Tauri or egui.
pub mod domain;
pub mod application;
pub mod infrastructure;
pub mod adapters;
pub mod edition;

// Compatibility exports while the native UI and CLI migrate incrementally.
pub use domain::{actions, model, richtext, rules};
pub mod richdoc {
    pub use crate::domain::richdoc::*;
    #[cfg(feature = "native-ui")]
    pub use crate::adapters::native::rendering::*;
}
pub use infrastructure::{ai, attach, mail, oauth, rag, recorder, secret};
pub use adapters::mcp;
#[cfg(feature = "native-ui")]
pub use adapters::native::{composer_tabs, ui};
