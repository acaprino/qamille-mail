//! Shared stdio host. Business operations are delegated to application services.
use crate::{
    application::{AppError, AppResult, ServiceContext, ports::StoreRepository},
    infrastructure::{
        gateways::*,
        mcp_config::{self, Profile},
        repository::JsonStoreRepository,
    },
    model::{Access, Store},
};
use serde_json::{Value, json};
use std::{
    fs::OpenOptions,
    io::{BufRead, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

const VERSIONS: &[&str] = &["2026-07-28", "2025-11-25", "2025-06-18"];
const MAX_INPUT: usize = 1024 * 1024;
const MAX_OUTPUT: usize = 256 * 1024;

pub fn context() -> AppResult<Arc<ServiceContext>> {
    let data_dir = Store::data_dir();
    // Task 4.3: import the JSON store on first use (idempotent), then follow
    // the same engine marker as the desktop process — two processes on two
    // engines would split the mail data.
    let migration = crate::infrastructure::store_migration::migrate_if_needed(&data_dir)?;
    let base: Arc<dyn StoreRepository> = if migration.activated {
        Arc::new(crate::infrastructure::sqlite_store::SqliteStoreRepository::open(
            crate::infrastructure::store_migration::sqlite_path(&data_dir),
        )?)
    } else {
        Arc::new(JsonStoreRepository::new(Store::path()))
    };
    let repository = crate::infrastructure::workspace_guard::modern(base, &data_dir)?;
    repository.read()?;
    // The same factory as the window (D9, A11): the session is only read from the vault, and
    // only for the compute routes.
    crate::infrastructure::compute_factory::configure(repository.clone(), Arc::new(KeyringSecrets));
    Ok(Arc::new(ServiceContext { compute: crate::infrastructure::compute_factory::routed(),
        events: Arc::new(crate::application::ports::NullSink),
        repository,
        mail: Arc::new(
            crate::infrastructure::provider_registry::ProviderRegistry::initial(
                Store::data_dir().join("attachments"),
            ),
        ),
        secrets: Arc::new(KeyringSecrets),
        clock: Arc::new(SystemClock),
        attachments: Arc::new(LocalAttachments),
    }))
}

/// Project account data before services see it and merge only visible accounts at commit.
struct ScopedRepository {
    base: Arc<dyn StoreRepository>,
    profile: Profile,
    name: Option<String>,
}
impl ScopedRepository {
    fn verify(&self, store: &Store) -> AppResult<()> {
        if let Some(name) = &self.name {
            let current = mcp_config::profile(name, store.settings.mcp_access)?;
            if serde_json::to_value(current)? != serde_json::to_value(&self.profile)? {
                return Err(AppError::new(
                    "permission_denied",
                    "MCP permissions changed during the operation",
                ));
            }
        }
        Ok(())
    }
    fn project(&self, store: &Store) -> AppResult<Store> {
        let mut value = serde_json::to_value(store)?;
        self.filter(&mut value)?;
        Ok(serde_json::from_value(value)?)
    }
    fn filter(&self, value: &mut Value) -> AppResult<()> {
        value["settings"]["mcp_access"] = serde_json::to_value(self.profile.access)?;
        if let Some(ids) = &self.profile.account_ids {
            for key in ["accounts", "emails", "drafts", "outbox"] {
                if let Some(items) = value[key].as_array_mut() {
                    items.retain(|item| account_id(key, item).is_some_and(|id| ids.contains(&id)));
                }
            }
            let email_ids: Vec<_> = value["emails"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|e| e["id"].as_u64())
                .collect();
            let draft_ids: Vec<_> = value["drafts"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|d| d["id"].as_u64())
                .collect();
            if let Some(actions) = value["undo_actions"].as_array_mut() {
                actions.retain(|a| {
                    a["operation"]["email_id"]
                        .as_u64()
                        .is_some_and(|id| email_ids.contains(&id))
                        || a["operation"]["draft_id"]
                            .as_u64()
                            .is_some_and(|id| draft_ids.contains(&id))
                });
            }
        }
        Ok(())
    }
}
impl StoreRepository for ScopedRepository {
    fn append_activity(&self, entry: &crate::domain::activity::NewActivity, at_ms: i64) -> AppResult<Option<i64>> {
        // The operation has already committed. Keep its audit even when permissions change
        // between commit and logging. The scoped wrapper exposes no unfiltered history.
        self.base.append_activity(entry, at_ms)
    }
    fn read(&self) -> AppResult<Store> {
        let store = self.base.read()?;
        self.verify(&store)?;
        self.project(&store)
    }
    fn transaction(&self, operation: &mut dyn FnMut(&mut Store) -> AppResult<()>) -> AppResult<()> {
        self.base.transaction(&mut |original| {
            self.verify(original)?;
            let mut visible = self.project(original)?;
            let visible_undo: Vec<_> = visible.undo_actions.iter().map(|a| a.id).collect();
            operation(&mut visible)?;
            let mut result = serde_json::to_value(&visible)?;
            let unfiltered = result.clone();
            self.filter(&mut result)?;
            if result != unfiltered {
                return Err(AppError::new(
                    "permission_denied",
                    "An operation referenced an account or calendar outside this profile",
                ));
            }
            let old = serde_json::to_value(&*original)?;
            if let Some(ids) = &self.profile.account_ids {
                for key in ["accounts", "emails", "drafts", "outbox"] {
                    if let Some(items) = old[key].as_array() {
                        let preserved = items
                            .iter()
                            .filter(|item| {
                                !account_id(key, item).is_some_and(|id| ids.contains(&id))
                            })
                            .cloned();
                        if let Some(target) = result[key].as_array_mut() {
                            target.extend(preserved);
                        }
                    }
                }
                if let Some(actions) = result["undo_actions"].as_array_mut() {
                    actions.extend(
                        old["undo_actions"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|a| {
                                a["id"]
                                    .as_u64()
                                    .is_some_and(|id| !visible_undo.contains(&id))
                            })
                            .cloned(),
                    );
                }
            }
            // The MCP profile never grants permission to elevate assistant access.
            result["settings"]["mcp_access"] = old["settings"]["mcp_access"].clone();
            *original = serde_json::from_value(result)?;
            Ok(())
        })
    }
    fn reconcile_storage(&self) -> AppResult<()> {
        self.base.reconcile_storage()
    }
    fn message_storage_bytes(
        &self,
        store: &Store,
    ) -> AppResult<std::collections::HashMap<u64, u64>> {
        self.base.message_storage_bytes(store)
    }
    fn draft_storage_bytes(
        &self,
        store: &Store,
    ) -> AppResult<std::collections::HashMap<u32, crate::domain::storage::DraftStorageBytes>> {
        self.base.draft_storage_bytes(store)
    }
}
fn account_id(key: &str, item: &Value) -> Option<u32> {
    let id = if key == "accounts" {
        &item["id"]
    } else if key == "outbox" {
        &item["draft"]["account_id"]
    } else {
        &item["account_id"]
    };
    id.as_u64().and_then(|n| u32::try_from(n).ok())
}

fn scoped(context: &Arc<ServiceContext>, profile: &Profile, name: &str) -> Arc<ServiceContext> {
    Arc::new(ServiceContext { compute: context.compute.clone(),
        events: Arc::new(crate::application::ports::NullSink),
        repository: Arc::new(ScopedRepository {
            base: context.repository.clone(),
            profile: profile.clone(),
            name: Some(name.into()),
        }),
        mail: Arc::new(super::grants::GuardedMail {
            inner: context.mail.clone(),
            repository: context.repository.clone(),
            name: name.into(),
            granted: profile.clone(),
        }),
        secrets: context.secrets.clone(),
        clock: context.clock.clone(),
        attachments: context.attachments.clone(),
    })
}

pub fn catalog() -> Vec<Value> {
    let mut tools = super::tools::catalog();
    tools.extend(super::calendar::catalog());
    tools.extend(super::outbox::catalog());
    tools.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    for tool in &mut tools {
        if tool.get("outputSchema").is_none() {
            tool["outputSchema"] = json!({"type":"object"});
        }
        let name = tool["name"].as_str().unwrap_or("");
        let access = required_access(name);
        let readonly = access == Access::ReadOnly;
        if !tool["annotations"].is_object() {
            tool["annotations"] = json!({});
        }
        let annotations = tool["annotations"].as_object_mut().unwrap();
        annotations.entry("readOnlyHint").or_insert(json!(readonly));
        annotations
            .entry("destructiveHint")
            .or_insert(json!(!readonly));
        annotations
            .entry("idempotentHint")
            .or_insert(json!(readonly));
        annotations.entry("openWorldHint").or_insert(json!(true));
    }
    tools
}
fn required_access(name: &str) -> Access {
    if super::calendar::handles(name) {
        super::calendar::required_access(name)
    } else if super::outbox::handles(name) {
        super::outbox::required_access(name)
    } else {
        super::tools::required_access(name)
    }
}

fn validate(value: &Value, schema: &Value, path: &str) -> AppResult<()> {
    if value.is_null() && schema["nullable"] == true {
        return Ok(());
    }
    let accepts = |kind: &str| match kind {
        "object" => value.is_object(),
        "array" => value.is_array(),
        "string" => value.is_string(),
        "integer" => value.is_i64() || value.is_u64(),
        "number" => value.is_number(),
        "boolean" => value.is_boolean(),
        "null" => value.is_null(),
        _ => true,
    };
    if let Some(kind) = schema["type"].as_str() {
        if !accepts(kind) {
            return Err(AppError::validation(format!("{path}: expected {kind}")));
        }
    }
    if let Some(kinds) = schema["type"].as_array() {
        if !kinds.iter().filter_map(Value::as_str).any(accepts) {
            return Err(AppError::validation(format!("{path}: invalid type")));
        }
    }
    if let Some(variants) = schema["anyOf"].as_array() {
        if !variants.iter().any(|s| validate(value, s, path).is_ok()) {
            return Err(AppError::validation(format!("{path}: invalid type")));
        }
    }
    if let Some(choices) = schema["enum"].as_array() {
        if !choices.contains(value) {
            return Err(AppError::validation(format!(
                "{path}: value is not in enum"
            )));
        }
    }
    if let Some(object) = value.as_object() {
        if let Some(required) = schema["required"].as_array() {
            for key in required.iter().filter_map(Value::as_str) {
                if !object.contains_key(key) {
                    return Err(AppError::validation(format!("{path}.{key} is required")));
                }
            }
        }
        for (key, item) in object {
            if let Some(spec) = schema["properties"].get(key) {
                validate(item, spec, &format!("{path}.{key}"))?;
            } else if schema["additionalProperties"] == false {
                return Err(AppError::validation(format!(
                    "{path}.{key}: unknown argument"
                )));
            }
        }
    }
    if let Some(items) = value.as_array() {
        if items.len() > schema["maxItems"].as_u64().unwrap_or(10000) as usize {
            return Err(AppError::validation(format!("{path}: too many items")));
        }
        if items.len() < schema["minItems"].as_u64().unwrap_or(0) as usize {
            return Err(AppError::validation(format!("{path}: too few items")));
        }
        if let Some(item_schema) = schema.get("items") {
            for item in items {
                validate(item, item_schema, path)?;
            }
        }
    }
    if let Some(number) = value.as_f64() {
        if schema["minimum"].as_f64().is_some_and(|min| number < min)
            || schema["maximum"].as_f64().is_some_and(|max| number > max)
        {
            return Err(AppError::validation(format!("{path}: number out of range")));
        }
    }
    if let Some(s) = value.as_str() {
        if s.len() > schema["maxLength"].as_u64().unwrap_or(MAX_INPUT as u64) as usize
            || s.chars().count() < schema["minLength"].as_u64().unwrap_or(0) as usize
        {
            return Err(AppError::validation(format!(
                "{path}: invalid string length"
            )));
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct Server {
    context: Arc<ServiceContext>,
    profile: String,
    legacy: Option<String>,
    initialized: bool,
    scoped_context: Arc<std::sync::Mutex<Option<(String, Arc<ServiceContext>)>>>,
}
impl Server {
    pub fn new(context: Arc<ServiceContext>, profile: String) -> Self {
        Self {
            context,
            profile,
            legacy: None,
            initialized: false,
            scoped_context: Arc::new(std::sync::Mutex::new(None)),
        }
    }
    pub fn profile_name(&self) -> &str {
        &self.profile
    }
    pub fn grant_fingerprint(&self) -> AppResult<String> {
        Ok(serde_json::to_string(&mcp_config::profile(
            &self.profile,
            self.context.read()?.settings.mcp_access,
        )?)?)
    }
    pub fn visible_account_ids(&self) -> AppResult<Vec<u32>> {
        let profile = mcp_config::profile(&self.profile, self.context.read()?.settings.mcp_access)?;
        Ok(scoped(&self.context, &profile, &self.profile)
            .read()?
            .accounts
            .iter()
            .map(|a| a.id)
            .collect())
    }
    pub fn call_tool(&self, name: &str, args: &Value) -> AppResult<Value> {
        let result = self.call_tool_inner(name, args);
        self.audit(name, result.is_ok());
        result
    }
    fn call_tool_inner(&self, name: &str, args: &Value) -> AppResult<Value> {
        let tool = catalog()
            .into_iter()
            .find(|tool| tool["name"] == name)
            .ok_or_else(|| AppError::new("unknown_tool", "Unknown MCP tool"))?;
        validate(args, &tool["inputSchema"], "arguments")?;
        let maximum = self.context.read()?.settings.mcp_access;
        let profile = mcp_config::profile(&self.profile, maximum)?;
        let needed = if matches!(
            name,
            "create_event" | "update_event" | "delete_event" | "import_events"
        ) {
            Access::Triage
        } else {
            required_access(name)
        };
        if mcp_config::rank(needed) > mcp_config::rank(profile.access) {
            return Err(AppError::new(
                "permission_denied",
                "Enable the required assistant access in Qamille Settings",
            ));
        }
        if matches!(
            name,
            "classify_email" | "generate_reply" | "transcribe_meeting" | "summarize_meeting"
        ) && !profile.ai_execute
        {
            return Err(AppError::new(
                "permission_denied",
                "Enable AI processing for this MCP profile in Settings",
            ));
        }
        if name == "delete_draft" && !profile.permanent_delete {
            return Err(AppError::new(
                "permission_denied",
                "This profile does not allow permanent draft deletion",
            ));
        }
        if name == "get_operation" && args["id"].as_str().is_some_and(|id| id.starts_with("job:")) {
            return super::jobs::get(
                args["id"].as_str().unwrap(),
                &self.profile,
                &self.grant_fingerprint()?,
                &self.visible_account_ids()?,
            );
        }
        let fingerprint = serde_json::to_string(&profile)?;
        if let Some(handle) = args["handle"].as_str() {
            super::files::check_profile(handle, &self.profile, &fingerprint)?;
        }
        if let Some(handles) = args["attachments"].as_array() {
            let replay = name == "send_email"
                && self
                    .context
                    .read()?
                    .outbox
                    .iter()
                    .any(|o| Some(o.key.as_str()) == args["idempotency_key"].as_str());
            if !replay {
                for handle in handles.iter().filter_map(Value::as_str) {
                    super::files::check_profile(handle, &self.profile, &fingerprint)?;
                }
            }
        }
        let context = {
            let mut cache = self
                .scoped_context
                .lock()
                .map_err(|_| AppError::storage("MCP context lock failed"))?;
            if cache.as_ref().is_none_or(|(key, _)| key != &fingerprint) {
                *cache = Some((fingerprint, scoped(&self.context, &profile, &self.profile)));
            }
            cache.as_ref().unwrap().1.clone()
        };
        let mut trusted_args = args.clone();
        if name == "send_email" {
            let replay = context
                .read()?
                .outbox
                .iter()
                .any(|o| Some(o.key.as_str()) == args["idempotency_key"].as_str());
            if let Some(handles) = args["attachments"].as_array().filter(|_| !replay) {
                let allowed = self.visible_account_ids()?;
                let paths = handles
                    .iter()
                    .map(|handle| {
                        super::files::resolve_scoped(
                            handle.as_str().ok_or_else(|| {
                                AppError::validation("Attachments must be file handles")
                            })?,
                            Some(&allowed),
                        )
                        .map(|p| p.to_string_lossy().into_owned())
                    })
                    .collect::<AppResult<Vec<_>>>()?;
                trusted_args["_attachment_paths"] = json!(paths);
            }
        }
        if super::outbox::handles(name)
            || matches!(name, "import_meeting" | "transcribe_meeting" | "summarize_meeting")
        {
            trusted_args["_source_profile"] = json!(self.profile);
        }
        let mut data = if super::calendar::handles(name) {
            super::calendar::call_scoped(context, name, args, profile.calendar_ids.as_deref())?
        } else if super::outbox::handles(name) {
            super::outbox::call(context, name, &trusted_args)?
        } else {
            super::tools::call(context, name, &trusted_args)?
        };
        if !data.is_object() {
            data = json!({"data":data});
        }
        if self.grant_fingerprint()? != serde_json::to_string(&profile)? {
            return Err(AppError::new(
                "permission_denied",
                "Permissions changed before the operation result could be returned",
            ));
        }
        bind_files(&data, &self.profile, &serde_json::to_string(&profile)?)?;
        if serde_json::to_vec(&data)?.len() > MAX_OUTPUT / 2 {
            return Err(AppError::new(
                "response_too_large",
                "Request fewer results or use the paginated content/file tools",
            ));
        }
        Ok(data)
    }
    pub(super) fn audit(&self, name: &str, ok: bool) {
        // Metadata only. Never write arguments, bodies, addresses, credentials or error text.
        let record = json!({"at":chrono::Utc::now().to_rfc3339(),"profile":self.profile,"tool":name,"success":ok});
        if let Ok(mut log) = OpenOptions::new()
            .append(true)
            .create(true)
            .open(Store::data_dir().join("mcp-activity.jsonl"))
        {
            let _ = log.lock();
            let _ = writeln!(log, "{record}");
        }
    }
    pub fn handle(&mut self, request: Value) -> Option<Value> {
        let id = request.get("id").cloned();
        let method = request["method"].as_str().unwrap_or("");
        if request["jsonrpc"] != "2.0"
            || method.is_empty()
            || id
                .as_ref()
                .is_some_and(|id| !id.is_string() && !id.is_i64() && !id.is_u64())
        {
            return Some(error(
                id.unwrap_or(Value::Null),
                -32600,
                "Invalid Request",
                None,
            ));
        }
        if method.starts_with("notifications/") {
            if method == "notifications/initialized" && self.legacy.is_some() {
                self.initialized = true;
            }
            return None;
        }
        let id = id?;
        let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
        if !params.is_object() {
            return Some(error(id, -32602, "params must be an object", None));
        }
        let modern = params["_meta"]["io.modelcontextprotocol/protocolVersion"].as_str();
        if let Some(version) = modern {
            if version != "2026-07-28" {
                return Some(error(
                    id,
                    -32022,
                    "Unsupported protocol version",
                    Some(json!({"supported":VERSIONS,"requested":version})),
                ));
            }
            if params["_meta"]
                .get("io.modelcontextprotocol/clientInfo")
                .is_some_and(|v| !v.is_object())
                || !params["_meta"]["io.modelcontextprotocol/clientCapabilities"].is_object()
            {
                return Some(error(
                    id,
                    -32602,
                    "Modern requests require clientCapabilities; clientInfo must be an object when supplied",
                    None,
                ));
            }
        } else if method != "initialize"
            && method != "ping"
            && (!self.initialized || self.legacy.is_none())
        {
            return Some(error(
                id,
                -32000,
                "Initialize the legacy connection or provide modern request metadata",
                None,
            ));
        }
        let result: Result<Value, Value> = match method {
            "initialize" => {
                let version = params["protocolVersion"].as_str().unwrap_or("");
                let selected = if VERSIONS[1..].contains(&version) {
                    version
                } else {
                    "2025-11-25"
                };
                self.legacy = Some(selected.into());
                self.initialized = false;
                Ok(
                    json!({"protocolVersion":selected,"capabilities":{"tools":{"listChanged":false},"resources":{}},"serverInfo":{"name":"qamille","version":env!("CARGO_PKG_VERSION")},"instructions":"Use account-scoped tools. Drafts never send implicitly. Sending requires Send access, a revision and an idempotency key. Read operation state to distinguish queued, sent and unknown."}),
                )
            }
            "server/discover" => Ok(
                json!({"supportedVersions":VERSIONS,"capabilities":{"tools":{},"resources":{}},"_meta":{"io.modelcontextprotocol/serverInfo":{"name":"qamille","version":env!("CARGO_PKG_VERSION")}},"instructions":"Qamille mail, drafts, outbox, calendars and meetings. Permissions are configured in the desktop app.","ttlMs":0,"cacheScope":"private"}),
            ),
            "ping" => Ok(json!({})),
            "tools/list" => {
                let tools = catalog();
                let offset = params["cursor"]
                    .as_str()
                    .map(|s| {
                        s.strip_prefix("tools:")
                            .and_then(|n| n.parse::<usize>().ok())
                    })
                    .unwrap_or(Some(0));
                match offset {
                    Some(offset) if offset <= tools.len() => {
                        let end = (offset + 40).min(tools.len());
                        let mut result = json!({"tools":&tools[offset..end]});
                        if end < tools.len() {
                            result["nextCursor"] = json!(format!("tools:{end}"));
                        }
                        Ok(result)
                    }
                    _ => Err(error(id.clone(), -32602, "Invalid tools cursor", None)),
                }
            }
            "tools/call" => match params["name"].as_str() {
                Some(name) => {
                    let args = params
                        .get("arguments")
                        .cloned()
                        .unwrap_or_else(|| json!({}));
                    let Some(definition) = catalog().into_iter().find(|tool| tool["name"] == name)
                    else {
                        return Some(error(id, -32602, "Unknown tool", None));
                    };
                    if let Err(validation) =
                        validate(&args, &definition["inputSchema"], "arguments")
                    {
                        return Some(error(id, -32602, &validation.message, None));
                    }
                    match super::jobs::run(self.clone(), name.into(), args) {
                        Ok(data) => Ok(
                            json!({"structuredContent":data,"content":[{"type":"text","text":serde_json::to_string(&data).unwrap_or_default()}],"isError":false}),
                        ),
                        Err(e) => {
                            let data = json!({"data":null,"meta":{},"error":{"code":e.code,"message":e.message,"retryable":matches!(e.code,"rate_limited"|"busy")}});
                            Ok(
                                json!({"structuredContent":data,"content":[{"type":"text","text":serde_json::to_string(&data).unwrap_or_default()}],"isError":true}),
                            )
                        }
                    }
                }
                None => Err(error(id.clone(), -32602, "Tool name is required", None)),
            },
            "resources/list" => Ok(json!({"resources":[]})),
            "resources/templates/list" => Ok(json!({"resourceTemplates":[]})),
            "resources/read" => {
                let uri = params["uri"].as_str().unwrap_or("");
                match uri.strip_prefix("qamille://files/") {
                    Some(handle) => match self
                        .call_tool("read_file", &json!({"handle":handle,"bytes":48*1024}))
                    {
                        Ok(data) => Ok(
                            json!({"contents":[{"uri":uri,"mimeType":"application/json","text":data.to_string()}]}),
                        ),
                        Err(e) => Err(error(id.clone(), -32602, &e.message, None)),
                    },
                    None => Err(error(id.clone(), -32602, "Unknown resource URI", None)),
                }
            }
            _ => Err(error(id.clone(), -32601, "Method not found", None)),
        };
        Some(match result {
            Ok(mut result) => {
                if modern.is_some() {
                    result["resultType"] = json!("complete");
                }
                json!({"jsonrpc":"2.0","id":id,"result":result})
            }
            Err(error) => error,
        })
    }
}
fn bind_files(value: &Value, profile: &str, grants: &str) -> AppResult<()> {
    match value {
        Value::Object(object) => {
            if let Some(handle) = object.get("handle").and_then(Value::as_str) {
                super::files::bind_profile(handle, profile, grants)?;
            }
            for value in object.values() {
                bind_files(value, profile, grants)?;
            }
        }
        Value::Array(values) => {
            for value in values {
                bind_files(value, profile, grants)?;
            }
        }
        _ => {}
    }
    Ok(())
}
fn error(id: Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut error = json!({"code":code,"message":message});
    if let Some(data) = data {
        error["data"] = data;
    }
    json!({"jsonrpc":"2.0","id":id,"error":error})
}

pub fn serve_io(
    input: impl BufRead,
    mut output: impl Write,
    mut server: Server,
) -> std::io::Result<()> {
    let mut input = input;
    loop {
        let mut line = Vec::new();
        let mut oversized = false;
        loop {
            let buffer = input.fill_buf()?;
            if buffer.is_empty() {
                break;
            }
            let end = buffer
                .iter()
                .position(|b| *b == b'\n')
                .map(|p| p + 1)
                .unwrap_or(buffer.len());
            if line.len() + end <= MAX_INPUT {
                line.extend_from_slice(&buffer[..end]);
            } else {
                oversized = true;
            }
            let done = buffer[end - 1] == b'\n';
            input.consume(end);
            if done {
                break;
            }
        }
        if line.is_empty() && !oversized {
            break;
        }
        let response = if oversized {
            Some(error(Value::Null, -32600, "Request exceeds 1 MiB", None))
        } else if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        } else {
            match serde_json::from_slice(&line) {
                Ok(request) => server.handle(request),
                Err(_) => Some(error(Value::Null, -32700, "Parse error", None)),
            }
        };
        if let Some(response) = response {
            serde_json::to_writer(&mut output, &response)?;
            output.write_all(b"\n")?;
            output.flush()?;
        }
    }
    Ok(())
}
pub fn serve() {
    let args: Vec<_> = std::env::args().collect();
    let profile = args
        .iter()
        .position(|a| a == "--mcp-profile")
        .and_then(|i| args.get(i + 1))
        .cloned()
        .unwrap_or_else(|| "default".into());
    match context() {
        Ok(context) => {
            let stop = Arc::new(AtomicBool::new(false));
            let worker_stop = stop.clone();
            let worker_context = context.clone();
            let worker = std::thread::spawn(move || worker_loop(worker_context, worker_stop));
            let result = serve_io(
                std::io::stdin().lock(),
                std::io::stdout().lock(),
                Server::new(context, profile),
            );
            stop.store(true, Ordering::Release);
            let _ = worker.join();
            if let Err(e) = result {
                eprintln!("MCP transport: {e}");
            }
        }
        Err(e) => eprintln!("MCP startup: {e}"),
    }
}
fn worker_loop(context: Arc<ServiceContext>, stop: Arc<AtomicBool>) {
    let root = Store::data_dir();
    let _ = std::fs::create_dir_all(&root);
    let Ok(lock) = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(root.join("mcp-worker.lock"))
    else {
        return;
    };
    loop {
        if stop.load(Ordering::Acquire) {
            return;
        }
        if lock.try_lock().is_ok() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let _ = std::fs::write(
        root.join("mcp-worker.json"),
        json!({"pid":std::process::id(),"started":chrono::Utc::now().to_rfc3339()}).to_string(),
    );
    let drafts = crate::application::drafts::DraftService::new(context.clone());
    let scheduler = crate::application::scheduler::SchedulerService::new(context.clone(), drafts);
    while !stop.load(Ordering::Acquire) {
        let _ = super::jobs::cleanup();
        if let Err(e) = super::outbox::tick(context.clone()) {
            eprintln!("Outbox worker: {}", e.code);
        }
        if let Err(e) = scheduler.tick() {
            eprintln!("Scheduler: {}", e.code);
        }
        for _ in 0..4 {
            if stop.load(Ordering::Acquire) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(250));
        }
    }
    let _ = std::fs::remove_file(root.join("mcp-worker.json"));
}
pub fn worker() {
    match context() {
        Ok(context) => worker_loop(context, Arc::new(AtomicBool::new(false))),
        Err(e) => eprintln!("Worker startup: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        domain::undo::{UndoAction, UndoOperation, UndoStatus},
        infrastructure::repository::MemoryRepository,
        model::{Account, Draft, Email},
    };
    fn fixture() -> Store {
        let mut store = Store::default();
        store.accounts = vec![
            Account {
                id: 1,
                email: "one@example.test".into(),
                ..Default::default()
            },
            Account {
                id: 2,
                email: "two@example.test".into(),
                ..Default::default()
            },
        ];
        store.emails = vec![
            Email {
                id: 10,
                account_id: 1,
                ..Default::default()
            },
            Email {
                id: 20,
                account_id: 2,
                ..Default::default()
            },
        ];
        store.drafts = vec![
            Draft {
                id: 11,
                account_id: 1,
                ..Default::default()
            },
            Draft {
                id: 21,
                account_id: 2,
                ..Default::default()
            },
        ];
        for (id, email_id) in [(30, 10), (40, 20)] {
            store.undo_actions.push(UndoAction {
                id,
                operation: UndoOperation::Mail {
                    email_id,
                    action: "archive".into(),
                    value: None,
                },
                subject: "synthetic".into(),
                created_at_ms: 0,
                expires_at_ms: i64::MAX,
                status: UndoStatus::Pending,
                error: None,
                retry_at_ms: None,
                attempts: 0,
            });
        }
        store
    }
    #[test]
    fn assistant_actions_reach_the_workspace_activity_log_through_both_wrappers() {
        use crate::domain::activity::{ActivityFilter, ActivitySource, NewActivity};
        let root = std::env::temp_dir().join(format!("qamille-mcp-audit-{}-{}",
            std::process::id(), std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()));
        let base = Arc::new(MemoryRepository::new(fixture()));
        let scope = ScopedRepository {
            base: crate::infrastructure::workspace_guard::modern(base.clone(), &root).unwrap(),
            name: None,
            profile: Profile { account_ids: Some(vec![1]), ..Default::default() },
        };
        let entry = NewActivity { source: ActivitySource::Assistant, account_id: Some(1), code: "test".into(), ..Default::default() };
        assert!(scope.append_activity(&entry, 50).unwrap().is_some());
        let history = base.activity_page(None, 5, ActivityFilter::All).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].source, ActivitySource::Assistant);
        assert!(scope.activity_page(None, 5, ActivityFilter::All).unwrap().is_empty());
        drop(scope);
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn scoped_commits_preserve_hidden_accounts_and_undo_actions() {
        let base = Arc::new(MemoryRepository::new(fixture()));
        let scope = ScopedRepository {
            base: base.clone(),
            name: None,
            profile: Profile {
                account_ids: Some(vec![1]),
                ..Default::default()
            },
        };
        let visible = scope.read().unwrap();
        assert_eq!(visible.accounts.len(), 1);
        assert_eq!(visible.undo_actions.len(), 1);
        scope
            .transaction(&mut |store| {
                store.emails[0].subject = "updated".into();
                store.undo_actions.clear();
                Ok(())
            })
            .unwrap();
        let stored = base.read().unwrap();
        assert_eq!(stored.accounts.len(), 2);
        assert_eq!(stored.email(20).unwrap().subject, "");
        assert_eq!(stored.undo_actions.len(), 1);
        assert_eq!(stored.undo_actions[0].id, 40);
    }
    #[test]
    fn scoped_repository_rolls_back_cross_account_writes() {
        let base = Arc::new(MemoryRepository::new(fixture()));
        let scope = ScopedRepository {
            base: base.clone(),
            name: None,
            profile: Profile {
                account_ids: Some(vec![1]),
                ..Default::default()
            },
        };
        let error = scope
            .transaction(&mut |store| {
                store.drafts.push(Draft {
                    id: 99,
                    account_id: 2,
                    ..Default::default()
                });
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.code, "permission_denied");
        assert_eq!(base.read().unwrap().drafts.len(), 2);
    }
    #[test]
    fn schema_validation_rejects_unknown_and_bad_batch_arguments() {
        let schema = json!({"type":"object","properties":{"ids":{"type":"array","items":{"type":"integer"},"minItems":1,"maxItems":2}},"required":["ids"],"additionalProperties":false});
        assert!(validate(&json!({"ids":[1,2]}), &schema, "args").is_ok());
        for value in [
            json!({"ids":[]}),
            json!({"ids":[1,2,3]}),
            json!({"ids":["1"]}),
            json!({"ids":[1],"secret":"x"}),
        ] {
            assert!(validate(&value, &schema, "args").is_err());
        }
    }
}
