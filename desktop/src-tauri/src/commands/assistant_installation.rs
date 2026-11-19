use crate::runtime::blocking;
use serde_json::{Value, json};
use qamille::{
    application::{AppError, AppResult},
    infrastructure::{
        assistant_install::{InstallRequest, Installer, Target},
        mcp_config,
    },
};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
};

fn installer() -> AppResult<Installer> {
    Installer::from_environment()
}

fn request(
    target: Target,
    profile: String,
    install_mcp: bool,
    install_skill: bool,
) -> AppResult<InstallRequest> {
    if install_mcp && profile != "default" && !mcp_config::load()?.profiles.contains_key(&profile) {
        return Err(AppError::validation(
            "Save this MCP profile in Qamille before connecting an assistant",
        ));
    }
    Ok(InstallRequest {
        target,
        profile,
        install_mcp,
        install_skill,
    })
}

#[tauri::command]
pub async fn assistant_installation_status() -> AppResult<Value> {
    blocking(move || {
        let statuses = installer()?.status()?;
        let targets: Vec<Value> = statuses
            .into_iter()
            .map(|status| {
                let mut value = serde_json::to_value(status)?;
                let id = value["id"].clone();
                value["name"] = value["label"].clone();
                value["mode"] = json!(
                    if matches!(id.as_str(), Some("chatgpt" | "claude_cowork")) {
                        "package"
                    } else {
                        "direct"
                    }
                );
                value["supports_mcp"] = value["mcp_supported"].clone();
                value["supports_skill"] = value["skill_supported"].clone();
                value["installed"] =
                    json!(value["mcp_installed"] == true || value["skill_installed"] == true);
                value["prepared"] =
                    json!(value["mcp_prepared"] == true || value["skill_prepared"] == true);
                Ok(value)
            })
            .collect::<AppResult<_>>()?;
        Ok(json!({"targets":targets,"executable_path":std::env::current_exe()?.to_string_lossy()}))
    })
    .await
}

#[tauri::command]
pub async fn assistant_installation_preview(
    target: Target,
    profile: String,
    install_mcp: bool,
    install_skill: bool,
) -> AppResult<Value> {
    blocking(move || {
        let request = request(target, profile, install_mcp, install_skill)?;
        let plan = serde_json::to_value(installer()?.preview(&request)?)?;
        let mut notes = plan["notes"].as_array().cloned().unwrap_or_default();
        notes.extend(plan["next_steps"].as_array().into_iter().flatten().cloned());
        Ok(json!({"target":plan["target"],"token":plan["fingerprint"],"files":plan["changes"],"notes":notes,"requires_import":plan["requires_import"]}))
    }).await
}

fn openings() -> &'static Mutex<HashMap<String, String>> {
    static OPENINGS: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();
    OPENINGS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn encoded(value: &str) -> String {
    value
        .bytes()
        .map(|byte| {
            if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
                (byte as char).to_string()
            } else {
                format!("%{byte:02X}")
            }
        })
        .collect()
}

#[tauri::command]
pub async fn assistant_installation_apply(
    target: Target,
    profile: String,
    install_mcp: bool,
    install_skill: bool,
    token: String,
) -> AppResult<Value> {
    blocking(move || {
        let request = request(target, profile, install_mcp, install_skill)?;
        let result = serde_json::to_value(installer()?.install(&request, &token)?)?;
        let destination = result["open_path"].as_str().map(|path| {
            if result["target"] == "chatgpt" {
                format!("codex://plugins/qamille?marketplacePath={}", encoded(path))
            } else { path.to_owned() }
        });
        if let (Some(target), Some(destination)) = (result["target"].as_str(), &destination) {
            openings().lock().map_err(|_| AppError::storage("Assistant installation result is unavailable"))?.insert(target.to_owned(), destination.clone());
        }
        let mut notes = result["notes"].as_array().cloned().unwrap_or_default();
        notes.extend(result["next_steps"].as_array().into_iter().flatten().cloned());
        Ok(json!({"target":result["target"],"changed_paths":result["paths"],"backup_paths":result["backups"],"notes":notes,"requires_import":result["requires_import"],"package_path":result["open_path"],"open_url":destination}))
    }).await
}

#[tauri::command]
pub async fn assistant_installation_open(target: Target) -> AppResult<()> {
    blocking(move || {
        let id = serde_json::to_value(target)?
            .as_str()
            .ok_or_else(|| AppError::validation("Unknown assistant"))?
            .to_owned();
        let destination = openings()
            .lock()
            .map_err(|_| AppError::storage("Assistant installation result is unavailable"))?
            .get(&id)
            .cloned()
            .ok_or_else(|| AppError::missing("Prepared installation"))?;
        open::that(destination).map_err(|error| AppError::external(error.to_string()))
    })
    .await
}
