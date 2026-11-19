//! Pure package builders for Qamille's assistant integrations.
use crate::application::{AppError, AppResult};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io::{Cursor, Write},
};

pub type PackageFile = (String, Vec<u8>);

pub fn skill_text() -> &'static str {
    include_str!("../../../integrations/qamille/skills/qamille/SKILL.md")
}

/// The single top-level directory is the account-skill ZIP format and the
/// relative directory to copy below a CLI client's personal skills directory.
pub fn skill_files() -> Vec<PackageFile> {
    vec![(
        "qamille/SKILL.md".into(),
        skill_text().as_bytes().to_vec(),
    )]
}

pub fn skill_zip() -> AppResult<Vec<u8>> {
    zip_files(&skill_files())
}

fn json_file(name: &str, value: Value) -> AppResult<PackageFile> {
    let mut bytes = serde_json::to_vec_pretty(&value)?;
    bytes.push(b'\n');
    Ok((name.into(), bytes))
}

fn checked_runtime(command: &str, data_dir: &str) -> AppResult<()> {
    if command.trim().is_empty()
        || data_dir.trim().is_empty()
        || command.chars().any(char::is_control)
        || data_dir.chars().any(char::is_control)
    {
        return Err(AppError::validation(
            "The Qamille executable and data directory are required",
        ));
    }
    Ok(())
}

fn mcp_args(args: &[String]) -> Vec<String> {
    let mut result = args.to_vec();
    // The installed executable can be the desktop app or the standalone server.
    if !result.iter().any(|arg| arg == "--mcp") {
        result.insert(0, "--mcp".into());
    }
    result
}

/// Creates the documented plugin layout. The runtime paths are specific to the
/// current PC; importing the archive never installs it into an account itself.
pub fn plugin_files(
    target: &str,
    command: &str,
    args: &[String],
    data_dir: &str,
    include_skill: bool,
    include_mcp: bool,
) -> AppResult<Vec<PackageFile>> {
    if !include_skill && !include_mcp {
        return Err(AppError::validation(
            "Select a skill or MCP server to package",
        ));
    }
    let manifest_name = match target {
        "chatgpt" => ".codex-plugin/plugin.json",
        "claude_cowork" => ".claude-plugin/plugin.json",
        _ => return Err(AppError::validation("Unknown assistant package target")),
    };
    let mut manifest = json!({
        "name":"qamille",
        "version":env!("CARGO_PKG_VERSION"),
        "description":"Use Qamille for email, drafts, calendars and saved meetings.",
        "author":{"name":"Qamille"}
    });
    let mut files = Vec::new();
    if include_skill {
        manifest["skills"] = json!("./skills/");
        files.push((
            "skills/qamille/SKILL.md".into(),
            skill_text().as_bytes().to_vec(),
        ));
    }
    if include_mcp {
        checked_runtime(command, data_dir)?;
        manifest["mcpServers"] = json!("./.mcp.json");
        if target == "chatgpt" {
            manifest["requires_local_executor"] = json!(true);
        }
        files.push(json_file(
            ".mcp.json",
            json!({"mcpServers":{"qamille":{
                "type":"stdio", "command":command,"args":mcp_args(args),
                "env":{"QAMILLE_DATA":data_dir}
            }}}),
        )?);
    }
    files.push(json_file(manifest_name, manifest)?);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(files)
}

/// Native Windows desktop extension. The executable bytes are supplied by the
/// caller; no downloads, host configuration changes, or account writes occur.
pub fn mcpb_files(binary: &[u8], args: &[String], data_dir: &str) -> AppResult<Vec<PackageFile>> {
    checked_runtime("qamille-mcp.exe", data_dir)?;
    if binary.is_empty() {
        return Err(AppError::validation("The Qamille executable is empty"));
    }
    let manifest = json!({
        "manifest_version":"0.3",
        "name":"qamille",
        "display_name":"Qamille",
        "version":env!("CARGO_PKG_VERSION"),
        "description":"Email, drafts, calendars and saved meetings from Qamille on this PC.",
        "author":{"name":"Qamille"},
        "server":{
            "type":"binary",
            "entry_point":"server/qamille-mcp.exe",
            "mcp_config":{
                "command":"${__dirname}/server/qamille-mcp.exe",
                "args":mcp_args(args),
                "env":{"QAMILLE_DATA":data_dir}
            }
        },
        "tools_generated":true,
        "compatibility":{"platforms":["win32"]}
    });
    Ok(vec![
        json_file("manifest.json", manifest)?,
        ("server/qamille-mcp.exe".into(), binary.to_vec()),
    ])
}

pub fn mcpb_zip(binary: &[u8], args: &[String], data_dir: &str) -> AppResult<Vec<u8>> {
    zip_files(&mcpb_files(binary, args, data_dir)?)
}

fn checked_entry(name: &str) -> AppResult<()> {
    if name.is_empty()
        || name.contains(['\\', ':'])
        || name.chars().any(char::is_control)
        || name
            .split('/')
            .any(|part| part.is_empty() || part == "." || part == "..")
    {
        return Err(AppError::validation(
            "Package files need a safe relative archive path",
        ));
    }
    Ok(())
}

/// Stable names, timestamps and order make repeated exports byte-for-byte equal.
pub fn zip_files(files: &[PackageFile]) -> AppResult<Vec<u8>> {
    let mut sorted = BTreeMap::new();
    for (name, bytes) in files {
        checked_entry(name)?;
        if sorted.insert(name.as_str(), bytes.as_slice()).is_some() {
            return Err(AppError::validation("Duplicate file in assistant package"));
        }
    }
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    for (name, bytes) in sorted {
        let permissions = if name.starts_with("server/") && name.ends_with(".exe") {
            0o755
        } else {
            0o644
        };
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .compression_level(Some(1))
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(permissions);
        archive
            .start_file(name, options)
            .map_err(|e| AppError::storage(e.to_string()))?;
        archive.write_all(bytes)?;
    }
    Ok(archive
        .finish()
        .map_err(|e| AppError::storage(e.to_string()))?
        .into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;

    fn unpack(bytes: Vec<u8>) -> BTreeMap<String, Vec<u8>> {
        let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).unwrap();
        let mut files = BTreeMap::new();
        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).unwrap();
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).unwrap();
            files.insert(entry.name().to_owned(), bytes);
        }
        files
    }
    fn json_at(files: &BTreeMap<String, Vec<u8>>, path: &str) -> Value {
        serde_json::from_slice(&files[path]).unwrap()
    }
    #[test]
    fn account_skill_has_one_root_folder_and_zip_is_deterministic() {
        let first = skill_zip().unwrap();
        assert_eq!(first, skill_zip().unwrap());
        let files = unpack(first);
        assert_eq!(files.len(), 1);
        assert_eq!(files["qamille/SKILL.md"], skill_text().as_bytes());
    }
    #[test]
    fn plugins_reference_only_packaged_components_and_preserve_runtime_arguments() {
        let args = vec!["--mcp".into(), "--mcp-profile".into(), "draft-only".into()];
        for target in ["chatgpt", "claude_cowork"] {
            let files = unpack(
                zip_files(
                    &plugin_files(
                        target,
                        r"C:\Program Files\Qamille\qamille.exe",
                        &args,
                        r"C:\Users\User\Qamille",
                        true,
                        true,
                    )
                    .unwrap(),
                )
                .unwrap(),
            );
            let path = if target == "chatgpt" {
                ".codex-plugin/plugin.json"
            } else {
                ".claude-plugin/plugin.json"
            };
            let manifest = json_at(&files, path);
            assert_eq!(manifest["mcpServers"], "./.mcp.json");
            assert_eq!(manifest["skills"], "./skills/");
            assert!(files.contains_key("skills/qamille/SKILL.md"));
            let config = json_at(&files, ".mcp.json");
            assert_eq!(config["mcpServers"]["qamille"]["args"], json!(args));
            assert_eq!(
                config["mcpServers"]["qamille"]["env"]["QAMILLE_DATA"],
                r"C:\Users\User\Qamille"
            );
            assert_eq!(
                manifest["requires_local_executor"],
                if target == "chatgpt" {
                    json!(true)
                } else {
                    Value::Null
                }
            );
        }
    }
    #[test]
    fn skill_only_and_mcp_only_exports_do_not_claim_missing_components() {
        let skill_only = plugin_files("chatgpt", "", &[], "", true, false).unwrap();
        assert!(!skill_only.iter().any(|(name, _)| name == ".mcp.json"));
        let files = unpack(zip_files(&skill_only).unwrap());
        let manifest = json_at(&files, ".codex-plugin/plugin.json");
        assert!(manifest.get("mcpServers").is_none());
        assert!(manifest.get("requires_local_executor").is_none());
        let mcp_only = unpack(
            zip_files(
                &plugin_files("claude_cowork", "qamille.exe", &[], "data", false, true).unwrap(),
            )
            .unwrap(),
        );
        assert!(!mcp_only.contains_key("skills/qamille/SKILL.md"));
        assert!(
            json_at(&mcp_only, ".claude-plugin/plugin.json")
                .get("skills")
                .is_none()
        );
    }
    #[test]
    fn native_bundle_contains_the_binary_and_desktop_mcp_switch() {
        let args = vec!["--mcp-profile".into(), "organize".into()];
        let files = unpack(mcpb_zip(b"synthetic executable", &args, r"C:\Data").unwrap());
        assert_eq!(files["server/qamille-mcp.exe"], b"synthetic executable");
        let manifest = json_at(&files, "manifest.json");
        assert_eq!(manifest["server"]["type"], "binary");
        assert_eq!(
            manifest["server"]["entry_point"],
            "server/qamille-mcp.exe"
        );
        assert_eq!(
            manifest["server"]["mcp_config"]["args"],
            json!(["--mcp", "--mcp-profile", "organize"])
        );
        assert_eq!(manifest["compatibility"]["platforms"], json!(["win32"]));
    }
    #[test]
    fn archive_paths_cannot_escape_or_collide() {
        for name in [
            "../skill",
            "/skill",
            r"C:\skill",
            r"a\skill",
            "a/../skill",
            "a//skill",
            "a/./skill",
            "skill\n.json",
        ] {
            assert!(zip_files(&[(name.into(), vec![])]).is_err(), "{name}");
        }
        assert!(zip_files(&[("skill".into(), vec![]), ("skill".into(), vec![])]).is_err());
        assert!(plugin_files("unknown", "exe", &[], "data", true, true).is_err());
        assert!(mcpb_zip(&[], &[], "data").is_err());
    }
}
