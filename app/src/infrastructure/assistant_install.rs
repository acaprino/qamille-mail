//! Previewable installation of Qamille's own client integration.
//! Client configuration is merged; unrelated servers and skills are never replaced.
use crate::application::{AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeMap,
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Target {
    Codex,
    ClaudeCode,
    ClaudeDesktop,
    Chatgpt,
    ClaudeCowork,
}
impl Target {
    pub fn id(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::ClaudeCode => "claude_code",
            Self::ClaudeDesktop => "claude_desktop",
            Self::Chatgpt => "chatgpt",
            Self::ClaudeCowork => "claude_cowork",
        }
    }
    fn label(self) -> &'static str {
        match self {
            Self::Codex => "Codex",
            Self::ClaudeCode => "Claude Code",
            Self::ClaudeDesktop => "Claude Desktop",
            Self::Chatgpt => "ChatGPT Work",
            Self::ClaudeCowork => "Claude Cowork",
        }
    }
}
fn yes() -> bool {
    true
}
fn default_profile() -> String {
    "default".into()
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InstallRequest {
    pub target: Target,
    #[serde(default = "default_profile")]
    pub profile: String,
    #[serde(default = "yes")]
    pub install_mcp: bool,
    #[serde(default = "yes")]
    pub install_skill: bool,
}
#[derive(Clone, Debug)]
pub struct InstallPaths {
    pub home: PathBuf,
    pub codex_home: PathBuf,
    pub claude_home: PathBuf,
    pub claude_config: PathBuf,
    pub appdata: PathBuf,
    pub data_dir: PathBuf,
}
impl InstallPaths {
    pub fn from_environment() -> AppResult<Self> {
        let home = dirs::home_dir()
            .ok_or_else(|| AppError::storage("The user home directory is unavailable"))?;
        let claude_override = std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from);
        let claude_home = claude_override
            .clone()
            .unwrap_or_else(|| home.join(".claude"));
        let claude_config = claude_override
            .map(|p| p.join(".claude.json"))
            .unwrap_or_else(|| home.join(".claude.json"));
        Ok(Self {
            codex_home: std::env::var_os("CODEX_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join(".codex")),
            claude_home,
            claude_config,
            appdata: std::env::var_os("APPDATA")
                .map(PathBuf::from)
                .unwrap_or_else(|| home.join("AppData/Roaming")),
            data_dir: crate::model::Store::data_dir(),
            home,
        })
    }
}
#[derive(Clone, Debug, Serialize)]
pub struct FileChange {
    pub path: String,
    pub kind: String,
    pub action: String,
}
#[derive(Clone, Debug, Serialize)]
pub struct InstallPreview {
    pub target: Target,
    pub profile: String,
    pub fingerprint: String,
    pub changes: Vec<FileChange>,
    pub notes: Vec<String>,
    pub next_steps: Vec<String>,
    pub requires_import: bool,
    pub open_path: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct InstallResult {
    pub target: Target,
    pub paths: Vec<String>,
    pub backups: Vec<String>,
    pub notes: Vec<String>,
    pub next_steps: Vec<String>,
    pub requires_import: bool,
    pub open_path: Option<String>,
}
#[derive(Clone, Debug, Serialize)]
pub struct TargetStatus {
    pub id: Target,
    pub label: String,
    pub detected: bool,
    pub mcp_supported: bool,
    pub skill_supported: bool,
    pub mcp_installed: bool,
    pub skill_installed: bool,
    pub mcp_prepared: bool,
    pub skill_prepared: bool,
    pub config_path: Option<String>,
    pub skill_path: Option<String>,
    pub requires_import: bool,
    pub notes: Vec<String>,
}
#[derive(Default, Deserialize, Serialize)]
struct Receipt {
    #[serde(default)]
    managed_by: String,
    #[serde(default)]
    files: BTreeMap<String, String>,
    #[serde(default)]
    server_hash: Option<String>,
    #[serde(default)]
    mcp_prepared: bool,
    #[serde(default)]
    skill_prepared: bool,
}
/// Names written by versions released before the rebrandings (Sparklone, then
/// Qemille): their installations are still recognized, and replaced on the next install.
const LEGACY_NAMES: [&str; 2] = ["qemille", "sparklone"];
fn legacy_plugin_folder(name: &str) -> String {
    format!(".codex/plugins/{name}-local")
}
fn legacy_data_variable(name: &str) -> String {
    format!("{}_DATA", name.to_ascii_uppercase())
}

/// A file installed under the old name, still exactly as that version wrote it.
struct LegacyFile {
    path: PathBuf,
    hash: String,
}
struct PlannedFile {
    path: PathBuf,
    kind: &'static str,
    before: Option<Vec<u8>>,
    after: Vec<u8>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct RecoveryRecord {
    target: PathBuf,
    backup: PathBuf,
    before_hash: String,
}
struct BinaryPlan {
    source: PathBuf,
    target: PathBuf,
    hash: String,
}
struct Plan {
    preview: InstallPreview,
    files: Vec<PlannedFile>,
    binary: Option<BinaryPlan>,
    profile_bytes: Option<Vec<u8>>,
    legacy_files: Vec<LegacyFile>,
}
pub struct Installer {
    pub paths: InstallPaths,
    source_exe: PathBuf,
}

impl Installer {
    pub fn new(paths: InstallPaths, source_exe: PathBuf) -> Self {
        Self { paths, source_exe }
    }
    pub fn from_environment() -> AppResult<Self> {
        Ok(Self::new(
            InstallPaths::from_environment()?,
            std::env::current_exe()?,
        ))
    }
    fn state_root(&self) -> PathBuf {
        self.paths.data_dir.join("integrations")
    }
    fn lock_and_recover(&self) -> AppResult<File> {
        let root = self.state_root();
        std::fs::create_dir_all(&root)?;
        let guard = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("install.lock"))?;
        guard.lock()?;
        let recovery = root.join("recovery");
        if recovery.exists() {
            for entry in std::fs::read_dir(&recovery)? {
                let path = entry?.path();
                if path.extension().is_none_or(|extension| extension != "json") {
                    continue;
                }
                let record: RecoveryRecord = serde_json::from_slice(&std::fs::read(&path)?)
                    .map_err(|_| AppError::storage("An interrupted integration recovery record is invalid; existing files were preserved"))?;
                let allowed = [
                    Target::Codex,
                    Target::ClaudeCode,
                    Target::ClaudeDesktop,
                    Target::Chatgpt,
                ]
                .into_iter()
                .filter_map(|target| self.config_path(target))
                .any(|p| p == record.target)
                    || [Target::Codex, Target::ClaudeCode]
                        .into_iter()
                        .filter_map(|target| self.skill_parent(target))
                        .any(|parent| {
                            record.target.starts_with(parent.join("qamille"))
                                || LEGACY_NAMES.iter().any(|name| record.target.starts_with(parent.join(name)))
                        })
                    || record
                        .target
                        .starts_with(self.paths.home.join(".codex/plugins/qamille-local"))
                    || LEGACY_NAMES.iter().any(|name| {
                        record.target.starts_with(self.paths.home.join(legacy_plugin_folder(name)))
                    })
                    || record.target.starts_with(root.join("installed"))
                    || record.target.starts_with(root.join("exports"));
                if !allowed {
                    return Err(AppError::storage(
                        "An interrupted integration recovery record has an unexpected destination; existing files were preserved",
                    ));
                }
                recover_file(&path, &record)?;
            }
        }
        Ok(guard)
    }
    fn receipt_path(&self, target: Target) -> PathBuf {
        self.state_root()
            .join("installed")
            .join(format!("{}.json", target.id()))
    }
    fn receipt(&self, target: Target) -> AppResult<Receipt> {
        match read_optional(&self.receipt_path(target))? {
            None => Ok(Receipt::default()),
            Some(bytes) => {
                let receipt: Receipt = serde_json::from_slice(&bytes).map_err(|_| {
                    AppError::new(
                        "installation_conflict",
                        "The integration ownership record is invalid; no client files were changed",
                    )
                })?;
                if receipt.managed_by != "qamille" && !LEGACY_NAMES.contains(&receipt.managed_by.as_str()) {
                    return Err(AppError::new(
                        "installation_conflict",
                        "The integration ownership record belongs to another application",
                    ));
                }
                Ok(receipt)
            }
        }
    }
    fn config_path(&self, target: Target) -> Option<PathBuf> {
        match target {
            Target::Codex => Some(self.paths.codex_home.join("config.toml")),
            Target::ClaudeCode => Some(self.paths.claude_config.clone()),
            Target::ClaudeDesktop => {
                Some(self.paths.appdata.join("Claude/claude_desktop_config.json"))
            }
            Target::Chatgpt => Some(self.paths.home.join(".agents/plugins/marketplace.json")),
            Target::ClaudeCowork => None,
        }
    }
    fn skill_parent(&self, target: Target) -> Option<PathBuf> {
        match target {
            Target::Codex => Some(self.paths.home.join(".agents/skills")),
            Target::ClaudeCode => Some(self.paths.claude_home.join("skills")),
            _ => None,
        }
    }
    /// Which components (MCP server, skill) the plugin prepared before the rebranding
    /// still has, as that version wrote them.
    fn legacy_plugin_components(&self, receipt: &Receipt) -> (bool, bool) {
        LEGACY_NAMES.iter().fold((false, false), |(mcp, skill), name| {
            let folder = self.paths.home.join(legacy_plugin_folder(name));
            (
                mcp || (receipt.mcp_prepared
                    && owned_file_available(receipt, &folder.join(".mcp.json"))),
                skill
                    || (receipt.skill_prepared
                        && owned_file_available(
                            receipt,
                            &folder.join(format!("skills/{name}/SKILL.md")),
                        )),
            )
        })
    }
    /// The old plugin is replaced only when this install brings back everything it had.
    fn replaces_legacy_plugin(&self, request: &InstallRequest, receipt: &Receipt) -> bool {
        let (mcp, skill) = self.legacy_plugin_components(receipt);
        (!mcp || request.install_mcp) && (!skill || request.install_skill)
    }
    /// The folders that belonged only to the old installation (removed once empty).
    fn legacy_roots(&self) -> Vec<PathBuf> {
        let mut roots: Vec<PathBuf> = [Target::Codex, Target::ClaudeCode]
            .into_iter()
            .filter_map(|target| self.skill_parent(target))
            .flat_map(|parent| LEGACY_NAMES.map(|name| parent.join(name)))
            .collect();
        roots.extend(LEGACY_NAMES.map(|name| self.paths.home.join(legacy_plugin_folder(name))));
        roots
    }
    fn export_path(&self, target: Target, profile: &str, extension: &str) -> PathBuf {
        self.state_root()
            .join("exports")
            .join(format!("qamille-{}-{profile}.{extension}", target.id()))
    }
    pub fn status(&self) -> AppResult<Vec<TargetStatus>> {
        let _guard = self.lock_and_recover()?;
        let mut results = Vec::new();
        for target in [
            Target::Codex,
            Target::ClaudeCode,
            Target::ClaudeDesktop,
            Target::Chatgpt,
            Target::ClaudeCowork,
        ] {
            let receipt = self.receipt(target)?;
            let config = self.config_path(target);
            let skill = self
                .skill_parent(target)
                .map(|p| p.join("qamille/SKILL.md"));
            let mcp_installed = match target {
                Target::Codex | Target::ClaudeCode | Target::ClaudeDesktop => config
                    .as_ref()
                    .and_then(|p| read_optional(p).ok().flatten())
                    .and_then(|bytes| configuration_entry(target, &bytes).ok().flatten())
                    .is_some_and(|entry| {
                        receipt.server_hash.as_deref() == Some(hash_value(&entry).as_str())
                            && self.executable_available(&entry)
                    }),
                _ => false,
            };
            let skill_installed = skill.as_ref().is_some_and(|p| {
                read_optional(p)
                    .ok()
                    .flatten()
                    .is_some_and(|b| receipt.files.get(&display(p)) == Some(&hash(&b)))
            });
            let (mcp_prepared, skill_prepared) = match target {
                Target::Codex | Target::ClaudeCode => (mcp_installed, skill_installed),
                Target::ClaudeDesktop => (
                    mcp_installed,
                    self.export_available(&receipt, target, "skill.zip"),
                ),
                Target::ClaudeCowork => (
                    self.export_available(&receipt, target, "mcpb"),
                    self.export_available(&receipt, target, "skill.zip"),
                ),
                Target::Chatgpt => {
                    let folder = self.paths.home.join(".codex/plugins/qamille-local");
                    let manifest_path = folder.join(".codex-plugin/plugin.json");
                    let manifest = owned_file_available(&receipt, &manifest_path);
                    let declared = read_optional(&manifest_path)
                        .ok()
                        .flatten()
                        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                        .unwrap_or(Value::Null);
                    let catalog = config
                        .as_ref()
                        .and_then(|p| read_optional(p).ok().flatten())
                        .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                        .is_some_and(|v| {
                            v["plugins"].as_array().is_some_and(|entries| {
                                entries.iter().any(|e| {
                                    e["name"] == "qamille"
                                        && e["source"]["source"] == "local"
                                        && e["source"]["path"] == "./.codex/plugins/qamille-local"
                                })
                            })
                        });
                    let mcp = owned_file_available(&receipt, &folder.join(".mcp.json"))
                        && read_optional(&folder.join(".mcp.json"))
                            .ok()
                            .flatten()
                            .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                            .is_some_and(|v| {
                                self.executable_available(&v["mcpServers"]["qamille"])
                            });
                    (
                        manifest && catalog && declared["mcpServers"] == "./.mcp.json" && mcp,
                        manifest
                            && catalog
                            && declared["skills"] == "./skills/"
                            && owned_file_available(
                                &receipt,
                                &folder.join("skills/qamille/SKILL.md"),
                            ),
                    )
                }
            };
            let detected = match target {
                Target::Codex => self.paths.codex_home.exists(),
                Target::ClaudeCode => {
                    self.paths.claude_home.exists()
                        || self.paths.claude_config.exists()
                        || self.paths.home.join(".local/bin/claude.exe").exists()
                }
                Target::ClaudeDesktop | Target::ClaudeCowork => {
                    self.paths.appdata.join("Claude").exists()
                }
                Target::Chatgpt => {
                    self.paths.codex_home.exists() || self.paths.appdata.join("OpenAI").exists()
                }
            };
            let requires_import = matches!(target, Target::Chatgpt | Target::ClaudeCowork)
                || (target == Target::ClaudeDesktop && skill_prepared);
            results.push(TargetStatus {
                id: target,
                label: target.label().into(),
                detected,
                mcp_supported: true,
                skill_supported: true,
                mcp_installed,
                skill_installed,
                mcp_prepared,
                skill_prepared,
                config_path: config.as_ref().map(|p| display(p)),
                skill_path: skill.as_ref().map(|p| display(p)),
                requires_import,
                notes: if matches!(target, Target::Chatgpt | Target::ClaudeCowork) {
                    vec!["Preparation is local; finish installation in the assistant app.".into()]
                } else {
                    vec![]
                },
            });
        }
        Ok(results)
    }
    fn executable_available(&self, entry: &Value) -> bool {
        let Some(command) = entry["command"].as_str() else {
            return false;
        };
        let path = Path::new(command);
        if !path.starts_with(self.state_root().join("bin")) {
            return false;
        }
        let Some(expected) = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|s| s.to_str())
        else {
            return false;
        };
        expected.len() == 64
            && expected.bytes().all(|b| b.is_ascii_hexdigit())
            && hash_file(path).is_ok_and(|actual| actual == expected)
    }
    fn export_available(&self, receipt: &Receipt, target: Target, extension: &str) -> bool {
        let prefix = format!("qamille-{}-", target.id());
        receipt.files.keys().map(Path::new).any(|p| {
            p.parent() == Some(self.state_root().join("exports").as_path())
                && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.starts_with(&prefix) && n.ends_with(&format!(".{extension}"))
                })
                && owned_file_available(receipt, p)
        })
    }
    pub fn preview(&self, request: &InstallRequest) -> AppResult<InstallPreview> {
        let _guard = self.lock_and_recover()?;
        Ok(self.plan(request)?.preview)
    }
    pub fn install(
        &self,
        request: &InstallRequest,
        expected_fingerprint: &str,
    ) -> AppResult<InstallResult> {
        // Repair a proven interrupted publication before computing its new plan.
        {
            let _guard = self.lock_and_recover()?;
        }
        let plan = self.plan(request)?;
        if plan.preview.fingerprint != expected_fingerprint {
            return Err(stale());
        }
        // Copy a potentially large executable before acquiring the configuration lock.
        if let Some(binary) = &plan.binary {
            install_binary(binary)?;
        }
        let _guard = self.lock_and_recover()?;
        if request.install_mcp
            && read_optional(&self.paths.data_dir.join("mcp-config.json"))? != plan.profile_bytes
        {
            return Err(stale());
        }
        for file in &plan.files {
            if read_optional(&file.path)? != file.before {
                return Err(stale());
            }
        }
        let mut paths = Vec::new();
        let mut backups = Vec::new();
        let mut committed: Vec<&PlannedFile> = Vec::new();
        for file in &plan.files {
            paths.push(display(&file.path));
            if file.before.as_deref() == Some(file.after.as_slice()) {
                continue;
            }
            let write = conditional_write(file, &mut backups, &self.state_root().join("recovery"));
            if let Err(error) = write {
                rollback(&committed, &self.state_root().join("recovery"));
                return Err(AppError::new(
                    "installation_failed",
                    format!(
                        "Installation could not finish. Existing files were restored where unchanged; backups: {}. {}",
                        backups.join(", "),
                        error.message
                    ),
                ));
            }
            committed.push(file);
        }
        for legacy in &plan.legacy_files {
            // Best effort: the new installation is complete either way. A file changed
            // since the preview is kept.
            if hash_file(&legacy.path).is_ok_and(|actual| actual == legacy.hash)
                && std::fs::remove_file(&legacy.path).is_ok()
            {
                remove_empty_parents(&legacy.path, &self.legacy_roots());
            }
        }
        if let Some(binary) = &plan.binary {
            paths.insert(0, display(&binary.target));
        }
        Ok(InstallResult {
            target: request.target,
            paths,
            backups,
            notes: plan.preview.notes,
            next_steps: plan.preview.next_steps,
            requires_import: plan.preview.requires_import,
            open_path: plan.preview.open_path,
        })
    }
    fn plan(&self, request: &InstallRequest) -> AppResult<Plan> {
        self.validate(request)?;
        let profile_bytes = if request.install_mcp {
            read_optional(&self.paths.data_dir.join("mcp-config.json"))?
        } else {
            None
        };
        if let Some(bytes) = &profile_bytes {
            let config: super::mcp_config::Config =
                serde_json::from_slice(bytes).map_err(|_| {
                    AppError::validation(
                        "Repair the assistant profiles in Qamille Settings before installing",
                    )
                })?;
            if request.profile != "default" && !config.profiles.contains_key(&request.profile) {
                return Err(AppError::validation(
                    "Save the selected assistant profile in Qamille Settings first",
                ));
            }
        } else if request.install_mcp && request.profile != "default" {
            return Err(AppError::validation(
                "Save the selected assistant profile in Qamille Settings first",
            ));
        }
        let mut receipt = self.receipt(request.target)?;
        let mut files = Vec::new();
        let mut binary = None;
        let args = vec![
            "--mcp".into(),
            "--mcp-profile".into(),
            request.profile.clone(),
        ];
        let source_hash = if request.install_mcp {
            Some(hash_file(&self.source_exe)?)
        } else {
            None
        };
        let command = if let Some(digest) = &source_hash {
            let name = self
                .source_exe
                .file_name()
                .ok_or_else(|| AppError::validation("The Qamille executable has no filename"))?;
            let target = self.state_root().join("bin").join(digest).join(name);
            if let Some(existing) = read_optional(&target)? {
                if hash(&existing) != *digest {
                    return Err(AppError::new(
                        "installation_conflict",
                        "The managed executable was modified; no client files were changed",
                    ));
                }
            }
            if request.target != Target::ClaudeCowork {
                binary = Some(BinaryPlan {
                    source: self.source_exe.clone(),
                    target: target.clone(),
                    hash: digest.clone(),
                });
            }
            display(&target)
        } else {
            String::new()
        };
        let server = json!({"type":"stdio","command":command,"args":args,"env":{"QAMILLE_DATA":display(&self.paths.data_dir)}});
        let mut notes=vec!["Existing Qamille profile permissions are preserved. Installation does not grant mail, calendar or AI access.".into()];
        let mut next_steps = Vec::new();
        let mut open_path = None;
        let mut requires_import = false;
        match request.target {
            Target::Codex | Target::ClaudeCode | Target::ClaudeDesktop => {
                if request.install_mcp {
                    let path = self.config_path(request.target).unwrap();
                    let before = read_optional(&path)?;
                    let after = merge_configuration(
                        request.target,
                        before.as_deref(),
                        &server,
                        receipt.server_hash.as_deref(),
                    )?;
                    receipt.server_hash = Some(hash_value(&server));
                    receipt.mcp_prepared = true;
                    files.push(PlannedFile {
                        path,
                        kind: "mcp_configuration",
                        before,
                        after,
                    });
                }
                if request.install_skill {
                    if let Some(parent) = self.skill_parent(request.target) {
                        for (relative, content) in super::assistant_packages::skill_files() {
                            add_owned(
                                &mut files,
                                &mut receipt,
                                parent.join(safe_relative(&relative)?),
                                "skill",
                                content,
                            )?;
                        }
                    } else {
                        let path = self.export_path(request.target, &request.profile, "skill.zip");
                        add_owned(
                            &mut files,
                            &mut receipt,
                            path.clone(),
                            "skill_archive",
                            super::assistant_packages::skill_zip()?,
                        )?;
                        open_path = path.parent().map(display);
                        requires_import = true;
                        next_steps.push("In Claude Customize > Add > Upload Skill, upload the skill ZIP; this does not install an MCP connection.".into());
                    }
                    receipt.skill_prepared = true;
                }
                next_steps.push(match request.target {Target::Codex=>"Start a new Codex session and check that the Qamille tools are available.",Target::ClaudeCode=>"Start a new Claude Code session and check /mcp and /skills.",_=>"Quit and reopen Claude Desktop to load the MCP connection."}.into());
            }
            Target::Chatgpt => {
                let folder = self.paths.home.join(".codex/plugins/qamille-local");
                // Installing one component keeps the other component's existing declaration.
                let include_mcp = request.install_mcp
                    || (receipt.mcp_prepared
                        && owned_file_available(&receipt, &folder.join(".mcp.json")));
                let include_skill = request.install_skill
                    || (receipt.skill_prepared
                        && owned_file_available(
                            &receipt,
                            &folder.join("skills/qamille/SKILL.md"),
                        ));
                let replace_legacy = self.replaces_legacy_plugin(request, &receipt);
                for (relative, mut content) in super::assistant_packages::plugin_files(
                    "chatgpt",
                    &command,
                    &args,
                    &display(&self.paths.data_dir),
                    request.install_skill,
                    request.install_mcp,
                )? {
                    if relative == ".codex-plugin/plugin.json" {
                        let mut manifest: Value = serde_json::from_slice(&content)?;
                        if include_mcp {
                            manifest["mcpServers"] = json!("./.mcp.json");
                            manifest["requires_local_executor"] = json!(true);
                        }
                        if include_skill {
                            manifest["skills"] = json!("./skills/");
                        }
                        content = serde_json::to_vec_pretty(&manifest)?;
                        content.push(b'\n');
                    }
                    add_owned(
                        &mut files,
                        &mut receipt,
                        folder.join(safe_relative(&relative)?),
                        "plugin",
                        content,
                    )?;
                }
                let path = self.config_path(request.target).unwrap();
                let before = read_optional(&path)?;
                let after = merge_marketplace(before.as_deref(), replace_legacy)?;
                files.push(PlannedFile {
                    path: path.clone(),
                    kind: "plugin_catalog",
                    before,
                    after,
                });
                receipt.mcp_prepared |= request.install_mcp;
                receipt.skill_prepared |= request.install_skill;
                open_path = Some(display(&path));
                requires_import = true;
                next_steps.push("Restart ChatGPT desktop, open Plugins > Personal, find Qamille and complete installation. Use a new local Work chat.".into());
                if !replace_legacy && self.legacy_plugin_components(&receipt) != (false, false) {
                    notes.push("The plugin prepared by the previous version stays until you install both the MCP server and the skill it had.".into());
                }
                notes.push("This prepares a personal local plugin. It does not publish to the public directory or enable the plugin automatically. ChatGPT web and custom GPTs do not read this PC's local configuration.".into());
            }
            Target::ClaudeCowork => {
                if request.install_mcp {
                    let path = self.export_path(request.target, &request.profile, "mcpb");
                    let bytes = std::fs::read(&self.source_exe)?;
                    if Some(hash(&bytes)) != source_hash {
                        return Err(stale());
                    }
                    add_owned(
                        &mut files,
                        &mut receipt,
                        path,
                        "mcp_extension",
                        super::assistant_packages::mcpb_zip(
                            &bytes,
                            &args,
                            &display(&self.paths.data_dir),
                        )?,
                    )?;
                    receipt.mcp_prepared = true;
                    next_steps.push("Open the .mcpb extension in Claude Desktop and complete its installation; enable the connector in Cowork where available.".into());
                }
                if request.install_skill {
                    let path = self.export_path(request.target, &request.profile, "skill.zip");
                    add_owned(
                        &mut files,
                        &mut receipt,
                        path,
                        "skill_archive",
                        super::assistant_packages::skill_zip()?,
                    )?;
                    receipt.skill_prepared = true;
                    next_steps.push("In Claude Customize > Add > Upload Skill, upload the skill ZIP and enable it for Cowork.".into());
                }
                open_path = Some(display(&self.state_root().join("exports")));
                requires_import = true;
                notes.push("The extension and skill are prepared locally; complete their installation in Claude. The extension points to this PC's Qamille data directory.".into());
            }
        }
        // Files the old version installed under its name: they leave the record now and
        // are deleted after the install, if nobody changed them in the meantime.
        let legacy_files = self.legacy_files(request, &receipt);
        for legacy in &legacy_files {
            receipt.files.remove(&display(&legacy.path));
        }
        receipt.managed_by = "qamille".into();
        let receipt_path = self.receipt_path(request.target);
        let before = read_optional(&receipt_path)?;
        files.push(PlannedFile {
            path: receipt_path,
            kind: "ownership_record",
            before,
            after: serde_json::to_vec_pretty(&receipt)?,
        });
        let mut changes: Vec<_> = files
            .iter()
            .map(|f| FileChange {
                path: display(&f.path),
                kind: f.kind.into(),
                action: action(f.before.as_deref(), &f.after).into(),
            })
            .collect();
        changes.extend(legacy_files.iter().map(|f| FileChange {
            path: display(&f.path),
            kind: "previous_version".into(),
            action: "delete".into(),
        }));
        if let Some(b) = &binary {
            changes.insert(
                0,
                FileChange {
                    path: display(&b.target),
                    kind: "executable".into(),
                    action: if b.target.exists() {
                        "unchanged"
                    } else {
                        "create"
                    }
                    .into(),
                },
            );
        }
        let fingerprints: Vec<_> = files
            .iter()
            .map(|f| {
                json!([
                    display(&f.path),
                    f.before.as_ref().map(|b| hash(b)),
                    hash(&f.after)
                ])
            })
            .collect();
        let removals: Vec<_> = legacy_files.iter().map(|f| json!([display(&f.path), f.hash])).collect();
        let fingerprint = hash_value(
            &json!({"request":request,"source":source_hash,"files":fingerprints,"profile":profile_bytes.as_ref().map(|b|hash(b)),"removals":removals}),
        );
        Ok(Plan {
            preview: InstallPreview {
                target: request.target,
                profile: request.profile.clone(),
                fingerprint,
                changes,
                notes,
                next_steps,
                requires_import,
                open_path,
            },
            files,
            binary,
            profile_bytes,
            legacy_files,
        })
    }
    /// The files of `receipt` that a version released before the rebranding installed
    /// under its old name and that still have the content it wrote.
    /// Only the components `request` installs again are replaced: installing the MCP
    /// server alone keeps the old skill.
    fn legacy_files(&self, request: &InstallRequest, receipt: &Receipt) -> Vec<LegacyFile> {
        let target = request.target;
        let mut roots: Vec<PathBuf> = Vec::new();
        if request.install_skill {
            if let Some(parent) = self.skill_parent(target) {
                roots.extend(LEGACY_NAMES.map(|name| parent.join(name)));
            }
        }
        if target == Target::Chatgpt && self.replaces_legacy_plugin(request, receipt) {
            roots.extend(LEGACY_NAMES.map(|name| self.paths.home.join(legacy_plugin_folder(name))));
        }
        let exports = self.state_root().join("exports");
        let legacy_exports = LEGACY_NAMES.map(|name| format!("{name}-{}-", target.id()));
        let export_replaced = |name: &str| {
            (request.install_skill && name.ends_with(".skill.zip"))
                || (request.install_mcp && name.ends_with(".mcpb"))
        };
        receipt
            .files
            .iter()
            .map(|(path, hash)| (PathBuf::from(path), hash))
            .filter(|(path, _)| {
                roots.iter().any(|root| path.starts_with(root))
                    || (path.parent() == Some(exports.as_path())
                        && path
                            .file_name()
                            .and_then(|n| n.to_str())
                            .is_some_and(|n| {
                                legacy_exports.iter().any(|prefix| n.starts_with(prefix)) && export_replaced(n)
                            }))
            })
            .filter(|(path, hash)| hash_file(path).is_ok_and(|actual| &actual == *hash))
            .map(|(path, hash)| LegacyFile { path, hash: hash.clone() })
            .collect()
    }
    fn validate(&self, request: &InstallRequest) -> AppResult<()> {
        if !request.install_mcp && !request.install_skill {
            return Err(AppError::validation(
                "Select the MCP server, the skill, or both",
            ));
        }
        if request.profile.is_empty()
            || request.profile.len() > 80
            || !request
                .profile
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-'))
        {
            return Err(AppError::validation(
                "Choose a valid saved assistant profile",
            ));
        }
        for path in [
            &self.paths.home,
            &self.paths.codex_home,
            &self.paths.claude_home,
            &self.paths.claude_config,
            &self.paths.appdata,
            &self.paths.data_dir,
        ] {
            if !path.is_absolute() {
                return Err(AppError::validation(
                    "Assistant installation paths must be absolute",
                ));
            }
        }
        if request.install_mcp && (!self.source_exe.is_absolute() || !self.source_exe.is_file()) {
            return Err(AppError::validation(
                "The installed Qamille executable is unavailable",
            ));
        }
        Ok(())
    }
}

fn display(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn hash_value(value: &Value) -> String {
    hash(&serde_json::to_vec(value).unwrap_or_default())
}
fn hash_file(path: &Path) -> AppResult<String> {
    let mut file = File::open(path)?;
    let mut digest = Sha256::new();
    let mut buffer = vec![0u8; 1024 * 1024];
    loop {
        let n = file.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        digest.update(&buffer[..n]);
    }
    Ok(format!("{:x}", digest.finalize()))
}
fn read_optional(path: &Path) -> AppResult<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}
fn owned_file_available(receipt: &Receipt, path: &Path) -> bool {
    receipt
        .files
        .get(&display(path))
        .is_some_and(|expected| hash_file(path).is_ok_and(|actual| &actual == expected))
}
fn stale() -> AppError {
    AppError::new(
        "installation_changed",
        "The executable, profile or client files changed after the preview. Review a fresh preview before installing.",
    )
}
fn conflict() -> AppError {
    AppError::new(
        "installation_conflict",
        "A different or locally modified Qamille integration already exists. It was preserved; choose another client or resolve that entry before installing.",
    )
}
fn action(before: Option<&[u8]>, after: &[u8]) -> &'static str {
    match before {
        None => "create",
        Some(b) if b == after => "unchanged",
        _ => "update",
    }
}
fn safe_relative(value: &str) -> AppResult<&Path> {
    let path = Path::new(value);
    if path.is_absolute()
        || path
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err(AppError::validation(
            "The integration contains an invalid relative path",
        ));
    }
    Ok(path)
}
fn add_owned(
    files: &mut Vec<PlannedFile>,
    receipt: &mut Receipt,
    path: PathBuf,
    kind: &'static str,
    after: Vec<u8>,
) -> AppResult<()> {
    let before = read_optional(&path)?;
    if let Some(bytes) = &before {
        if bytes != &after && receipt.files.get(&display(&path)) != Some(&hash(bytes)) {
            return Err(conflict());
        }
    }
    receipt.files.insert(display(&path), hash(&after));
    files.push(PlannedFile {
        path,
        kind,
        before,
        after,
    });
    Ok(())
}
fn configuration_entry(target: Target, bytes: &[u8]) -> AppResult<Option<Value>> {
    named_configuration_entry(target, bytes, "qamille", "QAMILLE_DATA")
}
/// The MCP entry a version released before a rebranding installed under `name`.
fn legacy_configuration_entry(target: Target, bytes: &[u8], name: &str) -> AppResult<Option<Value>> {
    named_configuration_entry(target, bytes, name, &legacy_data_variable(name))
}
fn named_configuration_entry(
    target: Target,
    bytes: &[u8],
    name: &str,
    data_variable: &str,
) -> AppResult<Option<Value>> {
    if target == Target::Codex {
        let text = std::str::from_utf8(bytes)
            .map_err(|_| AppError::validation("Codex configuration is not UTF-8"))?;
        let doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| {
            AppError::validation(
                "Codex configuration contains invalid TOML; original file preserved",
            )
        })?;
        let Some(item) = doc.get("mcp_servers").and_then(|v| v.get(name)) else {
            return Ok(None);
        };
        // Compare only the managed transport entry. Extra caller policy fields are preserved.
        let command = item
            .get("command")
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let args = item
            .get("args")
            .and_then(|v| v.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>())
            .unwrap_or_default();
        let env = item
            .get("env")
            .and_then(|v| v.get(data_variable))
            .and_then(|v| v.as_str())
            .unwrap_or_default();
        let mut entry = json!({"type":"stdio","command":command,"args":args,"env":{}});
        entry["env"][data_variable] = json!(env);
        Ok(Some(entry))
    } else {
        let root: Value = serde_json::from_slice(bytes).map_err(|_| {
            AppError::validation(
                "Assistant configuration contains invalid JSON; original file preserved",
            )
        })?;
        if !root.is_object() {
            return Err(AppError::validation(
                "Assistant configuration must be a JSON object",
            ));
        }
        Ok(root.get("mcpServers").and_then(|v| v.get(name)).cloned())
    }
}
fn merge_configuration(
    target: Target,
    before: Option<&[u8]>,
    server: &Value,
    previous_hash: Option<&str>,
) -> AppResult<Vec<u8>> {
    let existing = before
        .map(|b| configuration_entry(target, b))
        .transpose()?
        .flatten();
    if existing
        .as_ref()
        .is_some_and(|old| old != server && previous_hash != Some(hash_value(old).as_str()))
    {
        return Err(conflict());
    }
    // The entry the old version installed goes away with this install, but only if it
    // is still exactly what that version wrote (same rule as the current entry).
    let mut legacy = Vec::new();
    for name in LEGACY_NAMES {
        let entry = before.map(|b| legacy_configuration_entry(target, b, name)).transpose()?.flatten();
        if entry.is_some_and(|old| previous_hash == Some(hash_value(&old).as_str())) {
            legacy.push(name);
        }
    }
    if existing.as_ref() == Some(server) && legacy.is_empty() {
        return Ok(before.unwrap().to_vec());
    }
    if target == Target::Codex {
        let text = std::str::from_utf8(before.unwrap_or_default())
            .map_err(|_| AppError::validation("Codex configuration is not UTF-8"))?;
        let mut doc = text.parse::<toml_edit::DocumentMut>().map_err(|_| {
            AppError::validation(
                "Codex configuration contains invalid TOML; original file preserved",
            )
        })?;
        if doc.get("mcp_servers").is_some_and(|v| !v.is_table_like()) {
            return Err(AppError::validation("mcp_servers must be a TOML table"));
        }
        if doc.get("mcp_servers").is_none() {
            doc["mcp_servers"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        if let Some(table) = doc["mcp_servers"].as_table_like_mut() {
            for name in &legacy {
                table.remove(name);
            }
        }
        if doc["mcp_servers"].get("qamille").is_none() {
            doc["mcp_servers"]["qamille"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        let entry = &mut doc["mcp_servers"]["qamille"];
        entry["command"] = toml_edit::value(server["command"].as_str().unwrap());
        let mut args = toml_edit::Array::new();
        for arg in server["args"].as_array().unwrap() {
            args.push(arg.as_str().unwrap());
        }
        entry["args"] = toml_edit::value(args);
        if entry.get("env").is_some_and(|v| !v.is_table_like()) {
            return Err(AppError::validation(
                "The Qamille MCP environment must be a TOML table",
            ));
        }
        if entry.get("env").is_none() {
            entry["env"] = toml_edit::Item::Table(toml_edit::Table::new());
        }
        entry["env"]["QAMILLE_DATA"] =
            toml_edit::value(server["env"]["QAMILLE_DATA"].as_str().unwrap());
        Ok(doc.to_string().into_bytes())
    } else {
        let mut root = before
            .map(serde_json::from_slice::<Value>)
            .transpose()?
            .unwrap_or_else(|| json!({}));
        if !root.is_object() || root.get("mcpServers").is_some_and(|v| !v.is_object()) {
            return Err(AppError::validation("mcpServers must be a JSON object"));
        }
        if root.get("mcpServers").is_none() {
            root["mcpServers"] = json!({});
        }
        for name in &legacy {
            root["mcpServers"].as_object_mut().unwrap().remove(*name);
        }
        root["mcpServers"]["qamille"] = server.clone();
        Ok(serde_json::to_vec_pretty(&root)?)
    }
}
fn merge_marketplace(before: Option<&[u8]>, replace_legacy: bool) -> AppResult<Vec<u8>> {
    let mut root=before.map(serde_json::from_slice::<Value>).transpose().map_err(|_|AppError::validation("The personal plugin catalog is invalid JSON; original file preserved"))?.unwrap_or_else(||json!({"name":"qamille-local","interface":{"displayName":"Qamille"},"plugins":[]}));
    if !root.is_object() || !root["name"].is_string() || !root["plugins"].is_array() {
        return Err(AppError::validation(
            "The personal plugin catalog must have a name and a plugins array",
        ));
    }
    let desired = json!({"name":"qamille","source":{"source":"local","path":"./.codex/plugins/qamille-local"},"policy":{"installation":"AVAILABLE","authentication":"ON_INSTALL"},"category":"Productivity"});
    // A catalog created before the rebranding: same catalog, new name.
    let mut renamed = false;
    if replace_legacy && LEGACY_NAMES.iter().any(|name| root["name"] == format!("{name}-local")) {
        root["name"] = json!("qamille-local");
        if root["interface"]["displayName"] == "Qemille" || root["interface"]["displayName"] == "Sparklone" {
            root["interface"]["displayName"] = json!("Qamille");
        }
        renamed = true;
    }
    let entries = root["plugins"].as_array_mut().unwrap();
    // The old plugin entry, only when it still points at the old managed folder.
    let before_count = entries.len();
    entries.retain(|p| {
        !(replace_legacy
            && LEGACY_NAMES.iter().any(|name| {
                p["name"] == *name
                    && p["source"]["source"] == "local"
                    && p["source"]["path"] == format!("./{}", legacy_plugin_folder(name))
            }))
    });
    let cleaned = entries.len() != before_count || renamed;
    if let Some(existing) = entries.iter().find(|p| p["name"] == "qamille") {
        if existing["source"] != desired["source"] {
            return Err(conflict());
        }
        if !cleaned {
            return Ok(before.unwrap().to_vec());
        }
    } else {
        entries.push(desired);
    }
    Ok(serde_json::to_vec_pretty(&root)?)
}
/// Removes the empty folders between `file` and the old-installation folder holding it,
/// that folder included; never goes above it.
fn remove_empty_parents(file: &Path, roots: &[PathBuf]) {
    let Some(root) = roots.iter().find(|root| file.starts_with(root)) else {
        return;
    };
    let mut current = file.parent();
    while let Some(folder) = current {
        if !folder.starts_with(root) || std::fs::remove_dir(folder).is_err() {
            break;
        }
        current = folder.parent();
    }
}
fn unique_suffix() -> String {
    format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    )
}
fn backup_path(path: &Path) -> AppResult<PathBuf> {
    let filename = path
        .file_name()
        .ok_or_else(|| AppError::storage("Installation file has no filename"))?
        .to_string_lossy();
    Ok(path.with_file_name(format!("{filename}.qamille-backup-{}", unique_suffix())))
}

// On Windows the nonshared handle excludes writers and path replacement, including
// clients that do not honor our application lock. Publication never replaces a file
// created by another process while the old version is moved to its backup.
fn replacement_options() -> OpenOptions {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::{
            Foundation::{GENERIC_READ, GENERIC_WRITE},
            Storage::FileSystem::DELETE,
        };
        options
            .access_mode(GENERIC_READ | GENERIC_WRITE | DELETE)
            .share_mode(0);
    }
    options
}

#[cfg(windows)]
fn rename_open(file: &File, _source: &Path, target: &Path) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_RENAME_INFO, FileRenameInfo, SetFileInformationByHandle,
    };
    let name = crate::infrastructure::win_path::verbatim(target)?;
    let size = std::mem::size_of::<FILE_RENAME_INFO>() + name.len() * 2;
    // u64 storage provides the alignment needed by FILE_RENAME_INFO and its tail.
    let mut buffer = vec![0u64; size.div_ceil(8)];
    let info = buffer.as_mut_ptr().cast::<FILE_RENAME_INFO>();
    // SAFETY: the aligned allocation holds the header plus the complete UTF-16 name;
    // the file handle remains owned by the caller throughout this synchronous call.
    let success = unsafe {
        (*info).FileNameLength = (name.len() * 2) as u32;
        std::ptr::copy_nonoverlapping(
            name.as_ptr(),
            std::ptr::addr_of_mut!((*info).FileName).cast::<u16>(),
            name.len(),
        );
        SetFileInformationByHandle(
            file.as_raw_handle(),
            FileRenameInfo,
            info.cast(),
            size as u32,
        )
    };
    if success == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(not(windows))]
fn rename_open(_file: &File, source: &Path, target: &Path) -> std::io::Result<()> {
    // link is a no-replace publication on Unix as well: EEXIST preserves the winner.
    std::fs::hard_link(source, target)?;
    std::fs::remove_file(source)
}

fn recovery_marker(root: &Path, record: &RecoveryRecord) -> AppResult<PathBuf> {
    std::fs::create_dir_all(root)?;
    let path = root.join(format!("{}.json", unique_suffix()));
    let staged = path.with_extension("tmp");
    let mut marker = replacement_options().create_new(true).open(&staged)?;
    marker.write_all(&serde_json::to_vec(record)?)?;
    marker.sync_all()?;
    rename_open(&marker, &staged, &path)?;
    Ok(path)
}

fn recover_file(marker: &Path, record: &RecoveryRecord) -> AppResult<()> {
    let filename = record
        .target
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| AppError::storage("Invalid interrupted integration destination"))?;
    if !record.target.is_absolute()
        || record
            .target
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
        || record.backup.parent() != record.target.parent()
        || record
            .backup
            .file_name()
            .and_then(|name| name.to_str())
            .is_none_or(|name| {
                !name.starts_with(&format!("{filename}.qamille-backup-"))
                    && !LEGACY_NAMES.iter().any(|legacy| name.starts_with(&format!("{filename}.{legacy}-backup-")))
            })
        || record.before_hash.len() != 64
        || !record
            .before_hash
            .bytes()
            .all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(AppError::storage(
            "Invalid interrupted integration recovery record; existing files were preserved",
        ));
    }
    // A published update or an external winner always takes precedence. Backups
    // remain available, but may never replace a file created after interruption.
    if record.target.try_exists()? {
        std::fs::remove_file(marker)?;
        return Ok(());
    }
    let mut backup = replacement_options().open(&record.backup)?;
    let mut bytes = Vec::new();
    backup.read_to_end(&mut bytes)?;
    if hash(&bytes) != record.before_hash {
        return Err(AppError::storage(
            "An interrupted integration backup was modified; it was preserved for manual recovery",
        ));
    }
    if let Err(error) = rename_open(&backup, &record.backup, &record.target) {
        if !record.target.try_exists()? {
            return Err(error.into());
        }
    }
    std::fs::remove_file(marker)?;
    Ok(())
}

fn conditional_write(
    planned: &PlannedFile,
    backups: &mut Vec<String>,
    recovery_root: &Path,
) -> AppResult<()> {
    let parent = planned
        .path
        .parent()
        .ok_or_else(|| AppError::storage("Installation file has no parent directory"))?;
    std::fs::create_dir_all(parent)?;
    let temp = parent.join(format!(".qamille-install-{}.tmp", unique_suffix()));
    let mut marker = None;
    let result = (|| -> AppResult<()> {
        let mut staged = replacement_options().create_new(true).open(&temp)?;
        staged.write_all(&planned.after)?;
        staged.sync_all()?;
        let mut current = match replacement_options().open(&planned.path) {
            Ok(file) => Some(file),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => {
                return Err(AppError::new(
                    "installation_changed",
                    format!(
                        "The client configuration is in use; close the assistant and preview again. {error}"
                    ),
                ));
            }
        };
        #[cfg(not(windows))]
        if let Some(file) = &current {
            file.try_lock().map_err(|error| {
                AppError::new("installation_changed", format!("The client configuration is in use; close the assistant and preview again. {error}"))
            })?;
        }
        let actual = if let Some(file) = &mut current {
            let mut bytes = Vec::new();
            file.read_to_end(&mut bytes)?;
            Some(bytes)
        } else {
            None
        };
        if actual != planned.before {
            return Err(stale());
        }
        let backup = if let Some(file) = &current {
            let path = backup_path(&planned.path)?;
            marker = Some(recovery_marker(
                recovery_root,
                &RecoveryRecord {
                    target: planned.path.clone(),
                    backup: path.clone(),
                    before_hash: hash(planned.before.as_deref().unwrap()),
                },
            )?);
            rename_open(file, &planned.path, &path)?;
            backups.push(display(&path));
            Some(path)
        } else {
            None
        };
        if let Err(error) = rename_open(&staged, &temp, &planned.path) {
            if let (Some(file), Some(backup)) = (&current, &backup) {
                // Restore only if the path is still vacant; an external update wins.
                let _ = rename_open(file, backup, &planned.path);
            }
            return Err(AppError::new(
                "installation_changed",
                format!(
                    "The client configuration changed during installation; its new version was preserved. Preview again. {error}"
                ),
            ));
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    // Keep the flushed marker if an interruption/error left the destination
    // vacant. The next status, preview or install restores the verified backup.
    if planned.path.try_exists().unwrap_or(false) {
        if let Some(marker) = marker {
            let _ = std::fs::remove_file(marker);
        }
    }
    result
}
fn rollback(files: &[&PlannedFile], recovery_root: &Path) {
    for file in files.iter().rev() {
        if read_optional(&file.path).ok().flatten().as_deref() != Some(file.after.as_slice()) {
            continue;
        }
        match &file.before {
            Some(bytes) => {
                let _ = conditional_write(
                    &PlannedFile {
                        path: file.path.clone(),
                        kind: file.kind,
                        before: Some(file.after.clone()),
                        after: bytes.clone(),
                    },
                    &mut Vec::new(),
                    recovery_root,
                );
            }
            None => {
                let remove = || -> AppResult<()> {
                    let mut current = replacement_options().open(&file.path)?;
                    let mut bytes = Vec::new();
                    current.read_to_end(&mut bytes)?;
                    if bytes != file.after {
                        return Err(stale());
                    }
                    let displaced = backup_path(&file.path)?;
                    rename_open(&current, &file.path, &displaced)?;
                    drop(current);
                    std::fs::remove_file(displaced)?;
                    Ok(())
                };
                let _ = remove();
            }
        }
    }
}
fn install_binary(plan: &BinaryPlan) -> AppResult<()> {
    let parent = plan.target.parent().unwrap();
    std::fs::create_dir_all(parent)?;
    // A separate binary lock permits large copies without blocking config merges.
    let guard = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(parent.join(".binary.lock"))?;
    guard.lock()?;
    if plan.target.exists() {
        if hash_file(&plan.target)? == plan.hash {
            return Ok(());
        }
        return Err(conflict());
    }
    let temp = parent.join(format!(".qamille-binary-{}.tmp", unique_suffix()));
    let result = (|| -> AppResult<()> {
        let mut source = File::open(&plan.source)
            .map_err(|e| AppError::storage(format!("Cannot open the Qamille executable: {e}")))?;
        let mut output = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .map_err(|e| AppError::storage(format!("Cannot stage the managed executable: {e}")))?;
        std::io::copy(&mut source, &mut output)
            .map_err(|e| AppError::storage(format!("Cannot copy the managed executable: {e}")))?;
        output.sync_all()?;
        drop(output);
        if hash_file(&temp)? != plan.hash {
            return Err(stale());
        }
        OpenOptions::new().write(true).open(&temp)?.sync_all()?;
        if plan.target.exists() {
            if hash_file(&plan.target)? == plan.hash {
                std::fs::remove_file(&temp)?;
                return Ok(());
            }
            return Err(conflict());
        }
        // A content-addressed path is never changed once published.
        std::fs::rename(&temp, &plan.target).map_err(|e| {
            AppError::storage(format!("Cannot publish the managed executable: {e}"))
        })?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture {
        root: PathBuf,
        installer: Installer,
    }
    impl Fixture {
        fn new() -> Self {
            let root = std::env::temp_dir()
                .join(format!("qamille-assistant-install-{}", unique_suffix()));
            std::fs::create_dir_all(&root).unwrap();
            let source = root.join("qamille-desktop.exe");
            std::fs::write(&source, b"synthetic executable").unwrap();
            let paths = InstallPaths {
                home: root.join("home"),
                codex_home: root.join("custom-codex"),
                claude_home: root.join("home/.claude"),
                claude_config: root.join("home/.claude.json"),
                appdata: root.join("roaming"),
                data_dir: root.join("data"),
            };
            Self {
                root,
                installer: Installer::new(paths, source),
            }
        }
        fn request(&self, target: Target) -> InstallRequest {
            InstallRequest {
                target,
                profile: "default".into(),
                install_mcp: true,
                install_skill: true,
            }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }
    #[test]
    fn codex_install_preserves_comments_other_servers_and_uses_explicit_data_path() {
        let f = Fixture::new();
        let path = f.installer.config_path(Target::Codex).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let original=b"# Keep my settings\nmodel = 'custom-model'\n[mcp_servers.other]\ncommand = 'other.exe'\n";
        std::fs::write(&path, original).unwrap();
        let request = f.request(Target::Codex);
        let preview = f.installer.preview(&request).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
        let result = f.installer.install(&request, &preview.fingerprint).unwrap();
        assert!(!result.backups.is_empty());
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# Keep my settings"));
        assert!(content.contains("custom-model"));
        assert!(content.contains("other.exe"));
        let entry = configuration_entry(Target::Codex, content.as_bytes())
            .unwrap()
            .unwrap();
        assert_eq!(
            entry["env"]["QAMILLE_DATA"],
            display(&f.installer.paths.data_dir)
        );
        assert_eq!(entry["args"][2], "default");
        assert!(Path::new(entry["command"].as_str().unwrap()).is_file());
        assert!(
            f.installer
                .paths
                .home
                .join(".agents/skills/qamille/SKILL.md")
                .is_file()
        );
        let again = f.installer.preview(&request).unwrap();
        assert!(again.changes.iter().all(|c| c.action == "unchanged"));
        assert!(
            f.installer
                .install(&request, &again.fingerprint)
                .unwrap()
                .backups
                .is_empty()
        );
    }
    #[test]
    fn json_install_preserves_unrelated_servers_and_fails_on_foreign_entry() {
        let f = Fixture::new();
        let path = f.installer.config_path(Target::ClaudeCode).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            br#"{"theme":"dark","mcpServers":{"other":{"command":"other.exe"}}}"#,
        )
        .unwrap();
        let req = f.request(Target::ClaudeCode);
        let p = f.installer.preview(&req).unwrap();
        f.installer.install(&req, &p.fingerprint).unwrap();
        let value: Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["theme"], "dark");
        assert_eq!(value["mcpServers"]["other"]["command"], "other.exe");
        let mut changed = value;
        changed["mcpServers"]["qamille"]["command"] = json!("foreign.exe");
        let bytes = serde_json::to_vec(&changed).unwrap();
        std::fs::write(&path, &bytes).unwrap();
        assert_eq!(
            f.installer.preview(&req).unwrap_err().code,
            "installation_conflict"
        );
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    /// Turns a fresh Claude Code installation into the one a version released before
    /// the rebranding left: old entry name, data variable, skill folder and record.
    fn make_legacy_claude_code(f: &Fixture, legacy_name: &str) {
        let config = f.installer.config_path(Target::ClaudeCode).unwrap();
        let mut value: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
        let mut entry = value["mcpServers"].as_object_mut().unwrap().remove("qamille").unwrap();
        let data = entry["env"].as_object_mut().unwrap().remove("QAMILLE_DATA").unwrap();
        entry["env"][legacy_data_variable(legacy_name)] = data;
        value["mcpServers"][legacy_name] = entry.clone();
        std::fs::write(&config, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
        let skills = f.installer.skill_parent(Target::ClaudeCode).unwrap();
        std::fs::rename(skills.join("qamille"), skills.join(legacy_name)).unwrap();
        let mut receipt = f.installer.receipt(Target::ClaudeCode).unwrap();
        receipt.managed_by = legacy_name.into();
        receipt.server_hash = Some(hash_value(&entry));
        let new_root = display(&skills.join("qamille"));
        let old_root = display(&skills.join(legacy_name));
        receipt.files = receipt
            .files
            .into_iter()
            .map(|(path, hash)| (path.replacen(&new_root, &old_root, 1), hash))
            .collect();
        std::fs::write(
            f.installer.receipt_path(Target::ClaudeCode),
            serde_json::to_vec_pretty(&receipt).unwrap(),
        )
        .unwrap();
    }
    #[test]
    fn an_installation_from_before_the_rebranding_is_replaced_not_duplicated() {
        for legacy_name in LEGACY_NAMES {
            let f = Fixture::new();
            let config = f.installer.config_path(Target::ClaudeCode).unwrap();
            std::fs::create_dir_all(config.parent().unwrap()).unwrap();
            std::fs::write(&config, br#"{"mcpServers":{"other":{"command":"other.exe"}}}"#).unwrap();
            let req = f.request(Target::ClaudeCode);
            let p = f.installer.preview(&req).unwrap();
            f.installer.install(&req, &p.fingerprint).unwrap();
            make_legacy_claude_code(&f, legacy_name);
            let skills = f.installer.skill_parent(Target::ClaudeCode).unwrap();

            // The old record is accepted, and the preview lists the old files it removes.
            let p = f.installer.preview(&req).unwrap();
            assert!(p.changes.iter().any(|c| c.kind == "previous_version" && c.action == "delete"));
            f.installer.install(&req, &p.fingerprint).unwrap();

            let value: Value = serde_json::from_slice(&std::fs::read(&config).unwrap()).unwrap();
            assert!(value["mcpServers"].get(legacy_name).is_none());
            assert!(value["mcpServers"]["qamille"]["env"]["QAMILLE_DATA"].is_string());
            assert_eq!(value["mcpServers"]["other"]["command"], "other.exe");
            assert!(!skills.join(legacy_name).exists());
            assert!(skills.join("qamille/SKILL.md").exists());
            assert_eq!(f.installer.receipt(Target::ClaudeCode).unwrap().managed_by, "qamille");
            let status = f.installer.status().unwrap();
            let claude = status.iter().find(|s| s.id == Target::ClaudeCode).unwrap();
            assert!(claude.mcp_installed && claude.skill_installed);
        }
    }
    #[test]
    fn old_files_changed_by_the_user_or_not_reinstalled_are_kept() {
        let f = Fixture::new();
        let req = f.request(Target::ClaudeCode);
        let p = f.installer.preview(&req).unwrap();
        f.installer.install(&req, &p.fingerprint).unwrap();
        make_legacy_claude_code(&f, "sparklone");
        let skills = f.installer.skill_parent(Target::ClaudeCode).unwrap();
        let old_skill = skills.join("sparklone").join("SKILL.md");
        // MCP only: the old skill stays, it is not being replaced.
        let mcp_only = InstallRequest { install_skill: false, ..f.request(Target::ClaudeCode) };
        let p = f.installer.preview(&mcp_only).unwrap();
        f.installer.install(&mcp_only, &p.fingerprint).unwrap();
        assert!(old_skill.exists());
        // Edited by the user: kept even when the skill is installed again.
        std::fs::write(&old_skill, b"my own notes").unwrap();
        let p = f.installer.preview(&req).unwrap();
        f.installer.install(&req, &p.fingerprint).unwrap();
        assert_eq!(std::fs::read(&old_skill).unwrap(), b"my own notes");
        assert!(skills.join("qamille/SKILL.md").exists());
    }
    #[test]
    fn invalid_configuration_and_stale_preview_never_overwrite_user_files() {
        let f = Fixture::new();
        let req = f.request(Target::Codex);
        let path = f.installer.config_path(Target::Codex).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"invalid = [").unwrap();
        assert!(f.installer.preview(&req).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), b"invalid = [");
        std::fs::write(&path, b"model = 'first'\n").unwrap();
        let p = f.installer.preview(&req).unwrap();
        std::fs::write(&path, b"model = 'second'\n").unwrap();
        assert_eq!(
            f.installer.install(&req, &p.fingerprint).unwrap_err().code,
            "installation_changed"
        );
        assert_eq!(std::fs::read(&path).unwrap(), b"model = 'second'\n");
    }
    #[test]
    fn named_profiles_must_exist_and_install_does_not_change_permissions() {
        let f = Fixture::new();
        let mut req = f.request(Target::ClaudeDesktop);
        req.profile = "limited".into();
        assert!(f.installer.preview(&req).is_err());
        std::fs::create_dir_all(&f.installer.paths.data_dir).unwrap();
        let config = br#"{"profiles":{"limited":{"access":"ReadOnly","account_ids":[2]}}}"#;
        std::fs::write(f.installer.paths.data_dir.join("mcp-config.json"), config).unwrap();
        let p = f.installer.preview(&req).unwrap();
        assert!(p.requires_import);
        f.installer.install(&req, &p.fingerprint).unwrap();
        assert_eq!(
            std::fs::read(f.installer.paths.data_dir.join("mcp-config.json")).unwrap(),
            config
        );
        let entry = configuration_entry(
            Target::ClaudeDesktop,
            &std::fs::read(f.installer.config_path(Target::ClaudeDesktop).unwrap()).unwrap(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(entry["args"][2], "limited");
    }
    #[test]
    fn prepared_chatgpt_catalog_preserves_other_plugins_and_is_not_reported_installed() {
        let f = Fixture::new();
        let path = f.installer.config_path(Target::Chatgpt).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path,br#"{"name":"my-local","plugins":[{"name":"other","source":{"source":"local","path":"./other"}}]}"#).unwrap();
        let req = f.request(Target::Chatgpt);
        let p = f.installer.preview(&req).unwrap();
        f.installer.install(&req, &p.fingerprint).unwrap();
        let catalog: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(catalog["name"], "my-local");
        assert_eq!(catalog["plugins"].as_array().unwrap().len(), 2);
        let status = f
            .installer
            .status()
            .unwrap()
            .into_iter()
            .find(|s| s.id == Target::Chatgpt)
            .unwrap();
        assert!(status.mcp_prepared && status.skill_prepared && status.requires_import);
        assert!(!status.mcp_installed && !status.skill_installed);
    }
    #[test]
    fn skill_only_installs_without_executable_or_saved_mcp_profile() {
        let mut f = Fixture::new();
        f.installer.source_exe = f.root.join("missing.exe");
        f.installer.paths.claude_home = f.root.join("custom-claude");
        f.installer.paths.claude_config = f.root.join("custom-claude/.claude.json");
        let req = InstallRequest {
            target: Target::ClaudeCode,
            profile: "unconfigured".into(),
            install_mcp: false,
            install_skill: true,
        };
        let p = f.installer.preview(&req).unwrap();
        assert!(
            !p.changes
                .iter()
                .any(|c| c.kind == "executable" || c.kind == "mcp_configuration")
        );
        f.installer.install(&req, &p.fingerprint).unwrap();
        assert!(
            f.installer
                .paths
                .claude_home
                .join("skills/qamille/SKILL.md")
                .is_file()
        );
        assert!(
            !f.installer
                .paths
                .home
                .join(".claude/skills/qamille/SKILL.md")
                .exists()
        );
        assert!(!f.installer.paths.claude_config.exists());
    }
    #[test]
    fn status_detects_removed_binary_and_missing_export_artifacts() {
        let f = Fixture::new();
        let req = f.request(Target::Codex);
        let p = f.installer.preview(&req).unwrap();
        let result = f.installer.install(&req, &p.fingerprint).unwrap();
        let binary = result
            .paths
            .iter()
            .find(|p| p.ends_with("qamille-desktop.exe"))
            .unwrap();
        let status = f
            .installer
            .status()
            .unwrap()
            .into_iter()
            .find(|s| s.id == Target::Codex)
            .unwrap();
        assert!(status.mcp_installed && status.skill_installed);
        std::fs::remove_file(binary).unwrap();
        let status = f
            .installer
            .status()
            .unwrap()
            .into_iter()
            .find(|s| s.id == Target::Codex)
            .unwrap();
        assert!(!status.mcp_installed && !status.mcp_prepared && status.skill_installed);
        let req = f.request(Target::ClaudeCowork);
        let p = f.installer.preview(&req).unwrap();
        let result = f.installer.install(&req, &p.fingerprint).unwrap();
        for path in result
            .paths
            .iter()
            .filter(|p| p.ends_with(".mcpb") || p.ends_with(".skill.zip"))
        {
            std::fs::remove_file(path).unwrap();
        }
        let status = f
            .installer
            .status()
            .unwrap()
            .into_iter()
            .find(|s| s.id == Target::ClaudeCowork)
            .unwrap();
        assert!(!status.mcp_prepared && !status.skill_prepared);
    }
    #[test]
    fn codex_inline_tables_remain_valid_toml_after_merge() {
        let f = Fixture::new();
        let req = f.request(Target::Codex);
        let path = f.installer.config_path(Target::Codex).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(
            &path,
            b"# inline remains valid\nmcp_servers = { other = { command = 'other.exe' } }\n",
        )
        .unwrap();
        let p = f.installer.preview(&req).unwrap();
        f.installer.install(&req, &p.fingerprint).unwrap();
        let content = std::fs::read_to_string(path).unwrap();
        assert!(
            content.parse::<toml_edit::DocumentMut>().is_ok(),
            "{content}"
        );
        assert!(content.contains("other.exe"));
    }

    #[test]
    fn adding_chatgpt_components_keeps_the_existing_component() {
        for mcp_first in [true, false] {
            let f = Fixture::new();
            let mut req = f.request(Target::Chatgpt);
            req.install_mcp = mcp_first;
            req.install_skill = !mcp_first;
            let p = f.installer.preview(&req).unwrap();
            f.installer.install(&req, &p.fingerprint).unwrap();
            let folder = f
                .installer
                .paths
                .home
                .join(".codex/plugins/qamille-local");
            let component = folder.join(if mcp_first {
                ".mcp.json"
            } else {
                "skills/qamille/SKILL.md"
            });
            let before = std::fs::read(&component).unwrap();
            req.install_mcp = !mcp_first;
            req.install_skill = mcp_first;
            let p = f.installer.preview(&req).unwrap();
            f.installer.install(&req, &p.fingerprint).unwrap();
            assert_eq!(std::fs::read(component).unwrap(), before);
            let manifest: Value = serde_json::from_slice(
                &std::fs::read(folder.join(".codex-plugin/plugin.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(manifest["mcpServers"], "./.mcp.json");
            assert_eq!(manifest["skills"], "./skills/");
            let status = f
                .installer
                .status()
                .unwrap()
                .into_iter()
                .find(|s| s.id == Target::Chatgpt)
                .unwrap();
            assert!(status.mcp_prepared && status.skill_prepared);
        }
    }

    #[test]
    fn conditional_install_preserves_a_newer_client_configuration() {
        let f = Fixture::new();
        let path = f.root.join("client.json");
        std::fs::write(&path, b"external update").unwrap();
        let planned = PlannedFile {
            path: path.clone(),
            kind: "mcp_configuration",
            before: Some(b"original".to_vec()),
            after: b"install".to_vec(),
        };
        assert_eq!(
            conditional_write(&planned, &mut Vec::new(), &f.root.join("recovery"))
                .unwrap_err()
                .code,
            "installation_changed"
        );
        assert_eq!(std::fs::read(path).unwrap(), b"external update");
    }

    #[test]
    fn interrupted_publication_restores_only_a_verified_backup_to_a_vacant_path() {
        for external_winner in [false, true] {
            let f = Fixture::new();
            let path = f.installer.config_path(Target::Codex).unwrap();
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            let original = b"model = 'original'\n";
            std::fs::write(&path, original).unwrap();
            let backup = backup_path(&path).unwrap();
            let marker = recovery_marker(
                &f.installer.state_root().join("recovery"),
                &RecoveryRecord {
                    target: path.clone(),
                    backup: backup.clone(),
                    before_hash: hash(original),
                },
            )
            .unwrap();
            // Reproduce termination after moving the old file, before publishing.
            std::fs::rename(&path, &backup).unwrap();
            let external = b"model = 'external'\n";
            if external_winner {
                std::fs::write(&path, external).unwrap();
            }
            f.installer.status().unwrap();
            assert_eq!(
                std::fs::read(&path).unwrap(),
                if external_winner {
                    external.as_slice()
                } else {
                    original.as_slice()
                }
            );
            assert!(!marker.exists());
            if external_winner {
                assert_eq!(std::fs::read(&backup).unwrap(), original);
            }
        }
    }

    #[test]
    fn interrupted_publication_refuses_a_modified_backup() {
        let f = Fixture::new();
        let path = f.installer.config_path(Target::Codex).unwrap();
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let backup = backup_path(&path).unwrap();
        std::fs::write(&backup, b"modified backup").unwrap();
        let marker = recovery_marker(
            &f.installer.state_root().join("recovery"),
            &RecoveryRecord {
                target: path.clone(),
                backup: backup.clone(),
                before_hash: hash(b"original"),
            },
        )
        .unwrap();
        assert!(f.installer.preview(&f.request(Target::Codex)).is_err());
        assert!(!path.exists());
        assert!(marker.exists());
        assert_eq!(std::fs::read(&backup).unwrap(), b"modified backup");
    }

    #[cfg(windows)]
    #[test]
    fn client_file_is_protected_while_staging_and_external_replacement_wins() {
        let f = Fixture::new();
        let path = f.root.join("client.json");
        let staged_path = f.root.join("staged.json");
        std::fs::write(&path, b"original").unwrap();
        let current = replacement_options().open(&path).unwrap();
        assert!(std::fs::write(&path, b"concurrent write").is_err());
        assert!(std::fs::remove_file(&path).is_err());
        let backup = backup_path(&path).unwrap();
        rename_open(&current, &path, &backup).unwrap();
        std::fs::write(&path, b"external update").unwrap();
        let mut staged = replacement_options()
            .create_new(true)
            .open(&staged_path)
            .unwrap();
        staged.write_all(b"install").unwrap();
        staged.sync_all().unwrap();
        assert!(rename_open(&staged, &staged_path, &path).is_err());
        assert!(rename_open(&current, &backup, &path).is_err());
        drop(current);
        drop(staged);
        assert_eq!(std::fs::read(&path).unwrap(), b"external update");
        assert_eq!(std::fs::read(&backup).unwrap(), b"original");
    }

    #[test]
    fn backups_work_beyond_the_classic_windows_path_limit() {
        // std::fs handles long paths itself; the handle rename must do the same.
        let f = Fixture::new();
        let deep = f.root.join("a".repeat(120)).join("b".repeat(120));
        std::fs::create_dir_all(&deep).unwrap();
        let path = deep.join("client.json");
        std::fs::write(&path, b"original").unwrap();
        let current = replacement_options().open(&path).unwrap();
        let backup = backup_path(&path).unwrap();
        assert!(backup.as_os_str().len() > 260);
        rename_open(&current, &path, &backup).unwrap();
        drop(current);
        assert_eq!(std::fs::read(&backup).unwrap(), b"original");
        assert!(!path.exists());
    }
}
