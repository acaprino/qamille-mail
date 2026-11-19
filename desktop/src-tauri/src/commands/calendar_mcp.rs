use crate::runtime::blocking;
use serde_json::Value;
use qamille::{
    application::AppResult,
    mcp::{calendar, server},
};

#[tauri::command]
pub async fn calendar_mcp(action: String, args: Value) -> AppResult<Value> {
    blocking(move || calendar::call_desktop(server::context()?, &action, &args)).await
}
