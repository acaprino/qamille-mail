//! MCP calendar orchestration. Provider I/O stays outside repository transactions.
use crate::{
    application::{AppError, AppResult, ServiceContext, ports::GOOGLE_SECRET},
    domain::{
        calendar as dates,
        model::{Access, Event, Rsvp, Store},
        reminders,
    },
    infrastructure::{google_calendar::GoogleCalendar, oauth},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashMap, HashSet},
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::PathBuf,
    sync::{Arc, Mutex, atomic::AtomicBool},
};

impl From<dates::CalendarError> for AppError {
    fn from(error: dates::CalendarError) -> Self {
        AppError::validation(error.0)
    }
}

#[derive(Default, Serialize, Deserialize)]
struct CalendarState {
    #[serde(default)]
    local_extra: BTreeMap<String, Value>,
    #[serde(default)]
    remote: BTreeMap<String, RemoteProjection>,
    #[serde(default)]
    sync_tokens: BTreeMap<String, String>,
    #[serde(default)]
    creations: BTreeMap<String, Creation>,
    #[serde(default)]
    series_operations: BTreeMap<String, SeriesOperation>,
    #[serde(default)]
    view_seen: BTreeMap<String, Vec<String>>,
}
#[derive(Serialize, Deserialize)]
struct RemoteProjection {
    local_id: u64,
    account_id: u32,
    calendar_id: String,
    event: Value,
}
#[derive(Serialize, Deserialize)]
struct Creation {
    hash: String,
    status: String,
    result: Value,
}
#[derive(Clone, Serialize, Deserialize)]
struct SeriesOperation {
    account_id: u32,
    calendar_id: String,
    master: Value,
    instance: Value,
    status: String,
    new_event_id: String,
    result: Value,
}

struct StateFile {
    path: PathBuf,
    _guard: File,
    state: CalendarState,
}
impl StateFile {
    fn open() -> AppResult<Self> {
        let root = Store::data_dir();
        std::fs::create_dir_all(&root)?;
        let guard = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(root.join("calendar-state.lock"))?;
        guard.lock()?;
        let path = root.join("calendar-state.json");
        let state = match std::fs::read(&path) {
            Ok(b) => serde_json::from_slice(&b).map_err(|_| {
                AppError::storage("Calendar metadata is damaged; original file preserved")
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => CalendarState::default(),
            Err(e) => return Err(e.into()),
        };
        Ok(Self {
            path,
            _guard: guard,
            state,
        })
    }
    fn save(&self) -> AppResult<()> {
        let tmp = self
            .path
            .with_extension(format!("{}.tmp", std::process::id()));
        let mut file = File::create(&tmp)?;
        file.write_all(&serde_json::to_vec_pretty(&self.state)?)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(tmp, &self.path)?;
        Ok(())
    }
}

fn hash(value: &Value) -> String {
    format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).unwrap_or_default())
    )
}
fn str_arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args[key].as_str().filter(|s| !s.is_empty())
}
fn req<'a>(args: &'a Value, key: &str) -> AppResult<&'a str> {
    str_arg(args, key).ok_or_else(|| AppError::validation(format!("{key} is required")))
}
fn num(args: &Value, key: &str) -> AppResult<u64> {
    args[key]
        .as_u64()
        .or_else(|| args[key].as_str()?.parse().ok())
        .ok_or_else(|| AppError::validation(format!("{key} must be a positive ID")))
}
fn account_id(args: &Value) -> AppResult<u32> {
    u32::try_from(num(args, "account_id")?).map_err(|_| AppError::validation("Invalid account_id"))
}
fn zone(context: &ServiceContext) -> AppResult<String> {
    let s = context.read()?;
    Ok(context.clock.regional(&s.settings).time_zone)
}
fn calendar(args: &Value) -> &str {
    str_arg(args, "calendar_id").unwrap_or("local")
}
fn allowed(id: &str, allow: Option<&[String]>) -> AppResult<()> {
    if allow.is_some_and(|a| !a.iter().any(|v| v == id)) {
        Err(AppError::new(
            "permission_denied",
            "Calendar is outside this MCP profile",
        ))
    } else {
        Ok(())
    }
}
fn key(account: u32, cal: &str, id: &str) -> String {
    format!("{account}:{}:{cal}{id}", cal.len())
}

/// Legacy writers must not treat a cached provider event as an independent local event.
pub fn is_remote_projection(id: u64) -> AppResult<bool> {
    Ok(StateFile::open()?
        .state
        .remote
        .values()
        .any(|p| p.local_id == id))
}

/// Read-only calendar access for meeting reminders. It never projects provider
/// events, never saves `calendar-state.json` and never updates the workspace.
pub struct ReminderCalendarReader {
    context: Arc<ServiceContext>,
    /// Access tokens per account, so a reminder read is one request to Google.
    tokens: Mutex<HashMap<u32, CachedToken>>,
}
struct CachedToken {
    /// The refresh credential it came from: a new calendar consent gets a new token.
    refresh: String,
    access: String,
    until: i64,
}
/// A cached access token is replaced this long before Google says it expires.
const TOKEN_MARGIN_SECS: i64 = 300;

impl ReminderCalendarReader {
    pub fn new(context: Arc<ServiceContext>) -> Self {
        Self { context, tokens: Mutex::default() }
    }

    fn access_token(&self, store: &Store, account: u32) -> AppResult<String> {
        let refresh = self.context.secrets.get(&format!("calendar:{account}"))?;
        let now = self.context.clock.now();
        if let Ok(tokens) = self.tokens.lock()
            && let Some(token) = tokens.get(&account).filter(|t| t.refresh == refresh && now < t.until)
        {
            return Ok(token.access.clone());
        }
        let fresh = calendar_tokens(&self.context, store, account)?;
        let lifetime = i64::try_from(fresh.expires_in).unwrap_or(0);
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.insert(
                account,
                CachedToken {
                    refresh,
                    access: fresh.access.clone(),
                    until: now.saturating_add(lifetime).saturating_sub(TOKEN_MARGIN_SECS),
                },
            );
        }
        Ok(fresh.access)
    }

    fn forget_token(&self, account: u32) {
        if let Ok(mut tokens) = self.tokens.lock() {
            tokens.remove(&account);
        }
    }
}
impl crate::application::meeting_reminders::ReminderCalendar for ReminderCalendarReader {
    fn local(&self, store: &Store, from: i64, to: i64) -> AppResult<Vec<reminders::ReminderCandidate>> {
        let z = self.context.clock.regional(&store.settings).time_zone;
        // Damaged calendar metadata must not silence local reminders: without
        // it no event counts as all-day and no projection is skipped.
        let state = StateFile::open().map(|file| file.state).unwrap_or_default();
        let projected = projected_ids(&state);
        Ok(store
            .events
            .iter()
            .filter(|e| e.end > from && e.start < to && !projected.contains(&e.id))
            .map(|e| {
                let all_day = stored_all_day(state.local_extra.get(&e.id.to_string()), e, &z);
                reminders::local_candidate(e, all_day)
            })
            .collect())
    }
    fn google_accounts(&self, store: &Store) -> AppResult<Vec<u32>> {
        let mut accounts = Vec::new();
        for account in store.accounts.iter().filter(|a| {
            a.resolved_provider_id() == "gmail" && !a.oauth_client_id.is_empty()
        }) {
            if !self.context.secrets.get(&format!("calendar:{}", account.id))?.is_empty() {
                accounts.push(account.id);
            }
        }
        Ok(accounts)
    }
    fn google(
        &self,
        store: &Store,
        account: u32,
        from: i64,
        to: i64,
    ) -> AppResult<Vec<reminders::ReminderCandidate>> {
        let z = self.context.clock.regional(&store.settings).time_zone;
        let api = GoogleCalendar::new(self.access_token(store, account)?);
        let params = vec![
            ("timeMin", dates::rfc3339(from)?),
            ("timeMax", dates::rfc3339(to)?),
            ("singleEvents", "true".into()),
            ("orderBy", "startTime".into()),
            ("maxResults", "50".into()),
        ];
        // Every visible calendar of the account, not only the primary one; if the
        // list cannot be read the primary calendar is still checked.
        let mut calendars: Vec<String> = api
            .request("GET", "/users/me/calendarList", &[("maxResults", "50".into())], None, None)
            .ok()
            .map(|list| {
                list["items"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|c| c["selected"].as_bool() != Some(false) && c["accessRole"].as_str() != Some("freeBusyReader"))
                    .filter_map(|c| c["id"].as_str().map(String::from))
                    .take(25)
                    .collect()
            })
            .unwrap_or_default();
        if calendars.is_empty() {
            calendars.push("primary".into());
        }
        // Without readable metadata the chips still show, without a local link.
        let state = StateFile::open().map(|file| file.state).unwrap_or_default();
        let mut candidates = Vec::new();
        let mut last_error = None;
        let mut read = false;
        for calendar in &calendars {
            let result = api
                .request("GET", &GoogleCalendar::event_path(calendar, None), &params, None, None)
                // A rejected or expired token is replaced on the next read.
                .inspect_err(|_| self.forget_token(account));
            let result = match result {
                Ok(result) => result,
                // One unreadable secondary calendar must not hide the others.
                Err(error) => {
                    last_error = Some(error);
                    continue;
                }
            };
            read = true;
            candidates.extend(result["items"].as_array().into_iter().flatten().filter_map(|event| {
                let mut candidate = reminders::google_candidate(event, account, &z)?;
                candidate.local_event_id = linked_local_event(&state, account, event);
                Some(candidate)
            }));
        }
        match last_error {
            Some(error) if !read => Err(error),
            _ => Ok(candidates),
        }
    }
}

/// Local ids of cached provider events, which are not independent local events.
fn projected_ids(state: &CalendarState) -> HashSet<u64> {
    state.remote.values().map(|p| p.local_id).collect()
}

/// A stored all-day date counts only while it still matches the event's own times.
fn stored_all_day(extra: Option<&Value>, event: &Event, z: &str) -> bool {
    extra.is_some_and(|x| {
        !x["start"]["date"].is_null()
            && dates::google_timestamp(&x["start"], z).ok() == Some(event.start)
            && dates::google_timestamp(&x["end"], z).ok() == Some(event.end)
    })
}

/// The local copy of a Google occurrence: its own projection, otherwise the
/// projection of its recurring series. Projections are keyed by the account's
/// calendar id, not `primary`, so only the account is compared.
fn linked_local_event(state: &CalendarState, account: u32, event: &Value) -> Option<u64> {
    let projection = |id: &Value| {
        state
            .remote
            .values()
            .find(|p| p.account_id == account && id.is_string() && p.event["id"] == *id)
    };
    projection(&event["id"])
        .or_else(|| projection(&event["recurringEventId"]))
        .map(|p| p.local_id)
}

/// The local copy Qamille keeps of a Google event, so a recording started from
/// that event can link to it. It answers `None` when the event was never
/// projected, and also when the metadata cannot be read: a join must not fail
/// because of it, the recording just starts unlinked.
pub fn local_event_id_for(account_id: u32, google_event_id: &str) -> Option<u64> {
    if google_event_id.is_empty() {
        return None;
    }
    let file = StateFile::open().ok()?;
    linked_local_event(&file.state, account_id, &json!({ "id": google_event_id }))
}

/// Checks that account `id` may read its calendar and exchanges its separate
/// refresh credential for an access token.
fn calendar_tokens(context: &ServiceContext, store: &Store, id: u32) -> AppResult<oauth::Tokens> {
    let account = context.account(store, id)?;
    if account.resolved_provider_id() != "gmail" || account.oauth_client_id.is_empty() {
        return Err(AppError::new(
            "sign_in_required",
            "Connect this account with Google OAuth, then authorize_calendar",
        ));
    }
    let separate = context.secrets.get(&format!("calendar:{id}"))?;
    if separate.is_empty() {
        return Err(AppError::new(
            "calendar_consent_required",
            "Run authorize_calendar to grant calendar access for this account",
        ));
    }
    // Separate refresh credential prevents calendar consent from disturbing mail access.
    oauth::refresh(
        oauth::TOKEN_URL,
        &account.oauth_client_id,
        &crate::infrastructure::google_oauth_config::secret_for_client(
            &account.oauth_client_id,
            &context.secrets.get(GOOGLE_SECRET)?,
        ),
        &separate,
    )
    .map_err(AppError::external)
}

fn gateway(context: &ServiceContext, id: u32) -> AppResult<GoogleCalendar> {
    let store = context.read()?;
    let tokens = calendar_tokens(context, &store, id)?;
    let repository = context.repository.clone();
    let initial_access = store.settings.mcp_access;
    Ok(
        GoogleCalendar::new(tokens.access).with_authorization(Arc::new(move || {
            let current = repository.read()?;
            if current
                .account(id)
                .is_none_or(|account| account.resolved_provider_id() != "gmail")
                || current.settings.mcp_access != initial_access
            {
                return Err(AppError::new(
                    "permission_denied",
                    "Calendar authorization changed before the provider request",
                ));
            }
            Ok(())
        })),
    )
}

const NAMES: &[&str] = &[
    "list_calendars",
    "authorize_calendar",
    "sync_calendar",
    "list_events",
    "search_events",
    "read_event",
    "create_event",
    "update_event",
    "delete_event",
    "respond_to_event",
    "find_free_slots",
    "import_events",
    "export_events",
    "get_calendar_operation",
];
pub fn handles(name: &str) -> bool {
    NAMES.contains(&name)
}
pub fn required_access(name: &str) -> Access {
    if matches!(
        name,
        "authorize_calendar"
            | "create_event"
            | "update_event"
            | "delete_event"
            | "respond_to_event"
            | "import_events"
    ) {
        Access::Send
    } else {
        Access::ReadOnly
    }
}

pub fn catalog() -> Vec<Value> {
    let s = |desc: &str| json!({"type":"string","description":desc});
    NAMES.iter().map(|name|{
        let mut props=json!({
            "account_id":{"type":["string","integer"],"description":"Required for Google calendars"},"calendar_id":s("local or exact Google calendar ID"),
            "id":{"type":["string","integer"]},"limit":{"type":"integer","minimum":1,"maximum":100},"cursor":s("Opaque continuation from this query"),
            "from":s("RFC3339 with offset or YYYY-MM-DD"),"to":s("Exclusive range end"),"query":s("Text to search"),
            "title":s("Event title"),"description":s("Notes"),"location":s("Location"),"meeting_link":s("Optional HTTPS link for local events"),"start":s("RFC3339 with offset, or date for all-day"),"end":s("Exclusive end"),
            "minutes":{"type":"integer","minimum":1,"maximum":1440},"time_zone":s("IANA zone"),"all_day":{"type":"boolean"},"online":{"type":"boolean"},
            "attendees":{"type":["array","string"],"items":{"type":"string"}},"recurrence":{"type":"array","items":{"type":"string"}},"reminders":{"type":"object"},
            "send_updates":{"type":"string","enum":["all","externalOnly","none"]},"expected_revision":s("Revision from read_event"),"idempotency_key":s("Required for creations"),
            "scope":{"type":"string","enum":["instance","series","following"]},"rsvp":{"type":"string","enum":["accepted","tentative","declined"]},
            "buffer_minutes":{"type":"integer","minimum":0,"maximum":1440},"calendar_ids":{"type":"array","maxItems":50,"items":{"type":"string"}},
            "work_start":s("Working day start HH:MM"),"work_end":s("Working day end HH:MM"),"weekdays":{"type":"array","maxItems":7,"items":{"type":"integer","minimum":1,"maximum":7}},
            "ics":s("iCalendar text, maximum 1 MiB"),"handle":s("Managed ICS file handle; use either handle or ics"),"apply":{"type":"boolean","description":"false previews an ICS import"}
        });
        if let Some(obj)=props.as_object_mut(){obj.retain(|k,_|match *name {
            "list_calendars"=>matches!(k.as_str(),"account_id"|"limit"|"cursor"),
            "authorize_calendar"=>k=="account_id",
            "sync_calendar"=>matches!(k.as_str(),"account_id"|"calendar_id"),
            "list_events"|"search_events"|"export_events"=>matches!(k.as_str(),"account_id"|"calendar_id"|"from"|"to"|"query"|"limit"|"cursor"),
            "read_event"|"get_calendar_operation"=>matches!(k.as_str(),"account_id"|"calendar_id"|"id"),
            "find_free_slots"=>matches!(k.as_str(),"account_id"|"calendar_id"|"calendar_ids"|"date"|"from"|"to"|"minutes"|"buffer_minutes"|"work_start"|"work_end"|"weekdays"|"time_zone"),
            "delete_event"=>matches!(k.as_str(),"account_id"|"calendar_id"|"id"|"expected_revision"|"scope"|"send_updates"),
            "respond_to_event"=>matches!(k.as_str(),"account_id"|"calendar_id"|"id"|"expected_revision"|"rsvp"|"send_updates"),
            "import_events"=>matches!(k.as_str(),"account_id"|"calendar_id"|"ics"|"handle"|"apply"|"idempotency_key"|"send_updates"),
            _=>!matches!(k.as_str(),"cursor"|"limit"|"query"|"from"|"to"|"calendar_ids"|"rsvp"|"buffer_minutes"|"ics"|"handle"|"apply"|"work_start"|"work_end"|"weekdays")
        });}
        if *name=="find_free_slots"{props["date"]=s("YYYY-MM-DD in the requested/app time zone, restricted to configured work hours");}
        let required:Vec<&str>=match *name{"create_event"=>vec!["title","start","idempotency_key"],"update_event"|"delete_event"=>vec!["id","expected_revision"],"respond_to_event"=>vec!["id","expected_revision","rsvp"],"read_event"|"get_calendar_operation"=>vec!["id"],"authorize_calendar"=>vec!["account_id"],_=>vec![]};
        let read=matches!(required_access(name),Access::ReadOnly);
        let desc=if *name=="export_events"{"Export events as an ICS managed file handle (not inline content).".to_string()}else if *name=="import_events"{"Preview or apply an ICS import from a managed handle or inline ics text. Never reads arbitrary paths.".to_string()}else{format!("Qamille calendar: {}. Google requires explicit calendar consent; local uses the desktop calendar. Mutations require revision/idempotency as declared.",name.replace('_'," "))};
        json!({"name":name,"description":desc,"inputSchema":{"type":"object","properties":props,"required":required,"additionalProperties":false},"outputSchema":{"type":"object"},"annotations":{"readOnlyHint":read,"destructiveHint":*name=="delete_event","idempotentHint":*name=="create_event"||read}})
    }).collect()
}

pub fn call(context: Arc<ServiceContext>, name: &str, args: &Value) -> AppResult<Value> {
    call_scoped(context, name, args, None)
}
pub fn call_scoped(
    context: Arc<ServiceContext>,
    name: &str,
    args: &Value,
    allow: Option<&[String]>,
) -> AppResult<Value> {
    call_inner(context, name, args, allow, true)
}
/// The desktop user already has direct access to their workspace. MCP profiles
/// restrict external assistants, not deliberate calendar edits in the desktop UI.
pub fn call_desktop(context: Arc<ServiceContext>, name: &str, args: &Value) -> AppResult<Value> {
    call_inner(context, name, args, None, false)
}
fn call_inner(
    context: Arc<ServiceContext>,
    name: &str,
    args: &Value,
    allow: Option<&[String]>,
    enforce_access: bool,
) -> AppResult<Value> {
    if !handles(name) {
        return Err(AppError::new("unknown_tool", "Unknown calendar tool"));
    }
    let mut selected_args = args.clone();
    if !matches!(name, "list_calendars" | "authorize_calendar") {
        let prefs = super::tools::scheduling_preferences(&context)?;
        if selected_args.get("calendar_id").is_none() {
            selected_args["calendar_id"] = prefs["default_calendar_id"].clone();
        }
        if selected_args.get("account_id").is_none() && !prefs["default_account_id"].is_null() {
            selected_args["account_id"] = prefs["default_account_id"].clone();
        }
    }
    let args = &selected_args;
    let have = context.read()?.settings.mcp_access;
    let needed = if calendar(args) == "local"
        && matches!(
            name,
            "create_event" | "update_event" | "delete_event" | "import_events"
        ) {
        Access::Triage
    } else {
        required_access(name)
    };
    if enforce_access
        && crate::infrastructure::mcp_config::rank(have)
            < crate::infrastructure::mcp_config::rank(needed)
    {
        return Err(AppError::new(
            "permission_denied",
            "Calendar changes require Send access",
        ));
    }
    if name == "list_calendars" {
        return list_calendars(&context, args, allow);
    }
    if name == "authorize_calendar" {
        if allow.is_some_and(|a| a.iter().all(|c| c == "local")) {
            return Err(AppError::new(
                "permission_denied",
                "This profile has no remote calendars",
            ));
        }
        let id = account_id(args)?;
        let store = context.read()?;
        let account = store
            .account(id)
            .ok_or_else(|| AppError::missing("Account"))?;
        if account.resolved_provider_id() != "gmail" || account.oauth_client_id.is_empty() {
            return Err(AppError::new(
                "sign_in_required",
                "Use Google OAuth sign-in in Settings first",
            ));
        }
        // A consent whose token could not be saved is not started; credentials of another
        // Windows user count as none, and saving the token sets them aside.
        context.secrets.check_writable()?;
        let (email, tokens) = oauth::sign_in_with_scope(
            &account.oauth_client_id,
            &crate::infrastructure::google_oauth_config::secret_for_client(&account.oauth_client_id, &context.secrets.get_for_sign_in(GOOGLE_SECRET)?),
            &AtomicBool::new(false),
            oauth::CALENDAR_SCOPE,
            &account.email,
        )
        .map_err(AppError::external)?;
        if !email.eq_ignore_ascii_case(&account.email) {
            return Err(AppError::validation(
                "Calendar sign-in used a different email; no credentials were changed",
            ));
        }
        let latest = context.read()?;
        if latest.account(id).is_none() || latest.settings.mcp_access != have {
            return Err(AppError::new(
                "permission_denied",
                "Calendar authorization changed during browser consent; credentials were not saved",
            ));
        }
        context
            .secrets
            .set(&format!("calendar:{id}"), &tokens.refresh)?;
        return Ok(json!({"account_id":id,"status":"authorized","provider_id":"google"}));
    }
    let cal = calendar(args);
    allowed(cal, allow)?;
    if cal != "local" && context.read()?.account(account_id(args)?).is_none() {
        return Err(AppError::new(
            "permission_denied",
            "Account is outside this profile or no longer exists",
        ));
    }
    match name {
        "list_events" | "search_events" => list_events(&context, args, false),
        "read_event" => read_event(&context, args),
        "create_event" => create_event(&context, args),
        "update_event" | "delete_event" | "respond_to_event" => mutate_event(&context, name, args),
        "sync_calendar" => sync_calendar(&context, args),
        "find_free_slots" => find_slots(&context, args, allow),
        "export_events" => {
            let result = list_events(&context, args, true)?;
            let items = result["items"].as_array().cloned().unwrap_or_default();
            let ics = dates::export_ics(&items, &zone(&context)?)?;
            let account_ids = if cal == "local" {
                Vec::new()
            } else {
                vec![account_id(args)?]
            };
            let file = super::files::write_scoped(
                "qamille-events.ics",
                "text/calendar",
                ics.as_bytes(),
                &account_ids,
            )?;
            Ok(json!({"file":file,"count":items.len(),"meta":result["meta"]}))
        }
        "import_events" => import_events(&context, args),
        "get_calendar_operation" => {
            let state = StateFile::open()?;
            let record = state
                .state
                .series_operations
                .get(req(args, "id")?)
                .ok_or_else(|| AppError::missing("Calendar operation"))?;
            if record.calendar_id != cal || record.account_id != account_id(args)? {
                return Err(AppError::new(
                    "permission_denied",
                    "Operation belongs to another calendar/account",
                ));
            }
            Ok(json!({"operation_id":args["id"],"status":record.status,"result":record.result}))
        }
        _ => Err(AppError::new("unknown_tool", "Unknown calendar tool")),
    }
}

fn list_calendars(
    context: &ServiceContext,
    args: &Value,
    allow: Option<&[String]>,
) -> AppResult<Value> {
    let store = context.read()?;
    let z = context.clock.regional(&store.settings).time_zone;
    let mut items = Vec::new();
    let mut errors = Vec::new();
    if allowed("local", allow).is_ok() {
        items.push(json!({"id":"local","provider_id":"local","name":"Qamille","time_zone":z,"writable":true,"supports_recurrence":false}));
    }
    let only = args
        .get("account_id")
        .map(|_| account_id(args))
        .transpose()?;
    for account in store
        .accounts
        .iter()
        .filter(|a| only.is_none_or(|id| id == a.id) && a.resolved_provider_id() == "gmail")
    {
        let result = (|| -> AppResult<Vec<Value>> {
            let api = gateway(context, account.id)?;
            let mut all = Vec::new();
            let mut cursor = None;
            loop {
                let params = cursor
                    .take()
                    .map(|v| vec![("pageToken", v)])
                    .unwrap_or_default();
                let page = api.request("GET", "/users/me/calendarList", &params, None, None)?;
                for item in page["items"].as_array().into_iter().flatten() {
                    let id = item["id"].as_str().unwrap_or("");
                    if allowed(id, allow).is_ok() {
                        all.push(json!({"id":id,"provider_id":"google","account_id":account.id,"name":item["summary"],"time_zone":item["timeZone"],"writable":matches!(item["accessRole"].as_str(),Some("owner"|"writer")),"role":item["accessRole"],"supports_recurrence":true}));
                    }
                }
                match page["nextPageToken"].as_str() {
                    Some(next) => cursor = Some(next.into()),
                    None => break,
                };
                if all.len() > 10000 {
                    return Err(AppError::external(
                        "Too many calendars; narrow account selection",
                    ));
                }
            }
            Ok(all)
        })();
        match result {
            Ok(mut found) => items.append(&mut found),
            Err(e) => {
                errors.push(json!({"account_id":account.id,"code":e.code,"message":e.message}))
            }
        }
    }
    items.sort_by_key(|v| {
        (
            v["account_id"].as_u64().unwrap_or(0),
            v["id"].as_str().unwrap_or("").to_string(),
        )
    });
    let fingerprint = hash(&json!({"account_id":only,"items":items}));
    let offset = if let Some(cursor) = str_arg(args, "cursor") {
        let (prefix, offset) = cursor
            .split_once(':')
            .ok_or_else(|| AppError::validation("Invalid calendar cursor"))?;
        if prefix != fingerprint {
            return Err(AppError::new(
                "cursor_expired",
                "Calendar list changed; start again",
            ));
        }
        offset
            .parse::<usize>()
            .map_err(|_| AppError::validation("Invalid calendar cursor"))?
    } else {
        0
    };
    let total = items.len();
    let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
    let page: Vec<_> = items.into_iter().skip(offset).take(limit).collect();
    let next =
        (offset + page.len() < total).then(|| format!("{fingerprint}:{}", offset + page.len()));
    Ok(
        json!({"items":page,"total":total,"next_cursor":next,"has_more":next.is_some(),"errors":errors,"meta":{"coverage":if errors.is_empty(){"complete_for_scope"}else{"partial"}}}),
    )
}

fn local_value(event: &Event, extra: Option<&Value>, z: &str) -> AppResult<Value> {
    let mut v = extra.cloned().unwrap_or_else(|| json!({}));
    v["id"] = json!(event.id.to_string());
    v["calendar_id"] = json!("local");
    v["provider_id"] = json!("local");
    v["summary"] = json!(event.title);
    v["title"] = json!(event.title);
    if !stored_all_day(extra, event, z) {
        v["start"] = json!({"dateTime":dates::rfc3339(event.start)?,"timeZone":z});
        v["end"] = json!({"dateTime":dates::rfc3339(event.end)?,"timeZone":z});
    }
    v["location"] = json!(event.location);
    v["description"] = json!(event.notes);
    v["attendees"] = json!(
        event
            .attendees
            .iter()
            .map(|s| json!({"email":s}))
            .collect::<Vec<_>>()
    );
    v["hangoutLink"] = json!(event.meeting_link);
    v["revision"] = json!(hash(&json!({"event":event,"extra":extra})));
    Ok(v)
}

fn read_event(context: &ServiceContext, args: &Value) -> AppResult<Value> {
    let cal = calendar(args);
    if cal == "local" {
        let id = num(args, "id")?;
        let state = StateFile::open()?;
        if state.state.remote.values().any(|v| v.local_id == id) {
            return Err(AppError::validation(
                "This is a remote event: use its account_id, calendar_id and provider event ID",
            ));
        }
        let store = context.read()?;
        let e = store
            .events
            .iter()
            .find(|e| e.id == id)
            .ok_or_else(|| AppError::missing("Event"))?;
        return local_value(
            e,
            state.state.local_extra.get(&id.to_string()),
            &context.clock.regional(&store.settings).time_zone,
        );
    }
    let id = req(args, "id")?;
    let api = gateway(context, account_id(args)?)?;
    let mut result = api.request(
        "GET",
        &GoogleCalendar::event_path(cal, Some(id)),
        &[],
        None,
        None,
    )?;
    result["revision"] = result["etag"].clone();
    result["calendar_id"] = json!(cal);
    result["provider_id"] = json!("google");
    Ok(result)
}

fn range(context: &ServiceContext, args: &Value) -> AppResult<(i64, i64)> {
    let z = zone(context)?;
    let from = str_arg(args, "from")
        .map(|s| dates::timestamp(s, &z))
        .transpose()?
        .unwrap_or_else(|| context.clock.now());
    let to = str_arg(args, "to")
        .map(|s| dates::timestamp(s, &z))
        .transpose()?
        .unwrap_or(from + 7 * 86400);
    if to <= from {
        return Err(AppError::validation("Range end must be after its start"));
    }
    Ok((from, to))
}
fn list_events(context: &ServiceContext, args: &Value, all: bool) -> AppResult<Value> {
    let cal = calendar(args);
    let (from, to) = range(context, args)?;
    let z = zone(context)?;
    let limit = args["limit"].as_u64().unwrap_or(20).clamp(1, 100) as usize;
    let query = str_arg(args, "query").unwrap_or("");
    if cal == "local" {
        let state = StateFile::open()?;
        let store = context.read()?;
        let projected = projected_ids(&state.state);
        let mut all_items: Vec<_> = store
            .events
            .iter()
            .filter(|e| e.end > from && e.start < to && !projected.contains(&e.id))
            .filter(|e| {
                query.is_empty()
                    || format!("{} {} {}", e.title, e.notes, e.location)
                        .to_lowercase()
                        .contains(&query.to_lowercase())
            })
            .map(|e| local_value(e, state.state.local_extra.get(&e.id.to_string()), &z))
            .collect::<AppResult<Vec<_>>>()?;
        all_items.sort_by_key(|e| e["start"].to_string());
        let fingerprint = hash(&json!({"from":from,"to":to,"query":query,"items":all_items}));
        let offset = if let Some(c) = str_arg(args, "cursor") {
            let (f, n) = c
                .split_once(':')
                .ok_or_else(|| AppError::validation("Invalid calendar cursor"))?;
            if f != fingerprint {
                return Err(AppError::new(
                    "cursor_expired",
                    "Calendar changed; start this query again",
                ));
            }
            n.parse::<usize>()
                .map_err(|_| AppError::validation("Invalid calendar cursor"))?
        } else {
            0
        };
        let total = all_items.len();
        let count = if all { total } else { limit };
        let items: Vec<_> = all_items.into_iter().skip(offset).take(count).collect();
        let next = (offset + items.len() < total)
            .then(|| format!("{fingerprint}:{}", offset + items.len()));
        return Ok(
            json!({"items":items,"total":total,"next_cursor":next,"has_more":next.is_some(),"meta":{"coverage":"complete_for_scope","source":"local","time_zone":z}}),
        );
    }
    if args["cached"].as_bool() == Some(true) {
        let account = account_id(args)?;
        let state = StateFile::open()?;
        let items: Vec<_> = state
            .state
            .remote
            .values()
            .filter(|p| p.account_id == account && p.calendar_id == cal)
            .filter(|p| {
                dates::google_timestamp(&p.event["start"], &z).is_ok_and(|s| s < to)
                    && dates::google_timestamp(&p.event["end"], &z).is_ok_and(|e| e > from)
            })
            .map(|p| {
                let mut v = p.event.clone();
                v["revision"] = v["etag"].clone();
                v["calendar_id"] = json!(cal);
                v["provider_id"] = json!("google");
                v
            })
            .collect();
        return Ok(json!({"items":items,"meta":{"source":"cache","coverage":"partial"}}));
    }
    let api = gateway(context, account_id(args)?)?;
    let mut params = vec![
        ("timeMin", dates::rfc3339(from)?),
        ("timeMax", dates::rfc3339(to)?),
        ("singleEvents", "true".into()),
        ("orderBy", "startTime".into()),
        (
            "maxResults",
            if all { "100".into() } else { limit.to_string() },
        ),
    ];
    if !query.is_empty() {
        params.push(("q", query.into()));
    }
    if let Some(c) = str_arg(args, "cursor") {
        params.push(("pageToken", c.into()));
    }
    let mut items = Vec::new();
    let mut pages = 0;
    let view_key = hash(
        &json!({"account":account_id(args)?,"calendar":cal,"from":from,"to":to,"query":query}),
    );
    if str_arg(args, "cursor").is_none() {
        let mut state = StateFile::open()?;
        state.state.view_seen.insert(view_key.clone(), Vec::new());
        state.save()?;
    }
    loop {
        let result = api.request(
            "GET",
            &GoogleCalendar::event_path(cal, None),
            &params,
            None,
            None,
        )?;
        for e in result["items"].as_array().into_iter().flatten() {
            let mut e = e.clone();
            e["revision"] = e["etag"].clone();
            e["calendar_id"] = json!(cal);
            e["provider_id"] = json!("google");
            project(context, account_id(args)?, cal, &e)?;
            items.push(e);
        }
        let next = result["nextPageToken"].as_str();
        let mut state = StateFile::open()?;
        let seen = state.state.view_seen.entry(view_key.clone()).or_default();
        for e in result["items"].as_array().into_iter().flatten() {
            if let Some(id) = e["id"].as_str() {
                if !seen.iter().any(|v| v == id) {
                    seen.push(id.to_string());
                }
            }
        }
        let stale: Vec<String> = if next.is_none() && query.is_empty() {
            let seen = seen.clone();
            state
                .state
                .remote
                .values()
                .filter(|p| p.account_id == account_id(args).unwrap_or(0) && p.calendar_id == cal)
                .filter(|p| {
                    dates::google_timestamp(&p.event["start"], &z).is_ok_and(|s| s < to)
                        && dates::google_timestamp(&p.event["end"], &z).is_ok_and(|e| e > from)
                })
                .filter_map(|p| p.event["id"].as_str())
                .filter(|id| !seen.iter().any(|s| s == id))
                .map(String::from)
                .collect()
        } else {
            Vec::new()
        };
        if next.is_none() {
            state.state.view_seen.remove(&view_key);
        }
        state.save()?;
        drop(state);
        for id in stale {
            remove_projection(context, account_id(args)?, cal, &id)?;
        }
        if !all || next.is_none() {
            return Ok(
                json!({"items":items,"next_cursor":next,"has_more":next.is_some(),"meta":{"coverage":if next.is_some(){"partial"}else{"complete_for_scope"},"source":"google","time_zone":z}}),
            );
        }
        pages += 1;
        if pages >= 100 {
            return Err(AppError::new(
                "range_too_large",
                "Export exceeds 10,000 events; choose a smaller range",
            ));
        }
        params.retain(|(k, _)| *k != "pageToken");
        params.push(("pageToken", next.unwrap().into()));
    }
}

fn event_body(context: &ServiceContext, args: &Value, base: Option<&Value>) -> AppResult<Value> {
    let z = str_arg(args, "time_zone")
        .map(String::from)
        .unwrap_or(zone(context)?);
    z.parse::<chrono_tz::Tz>()
        .map_err(|_| AppError::validation("Choose an IANA time zone"))?;
    let mut body = base.cloned().unwrap_or_else(|| json!({}));
    if let Some(title) = str_arg(args, "title") {
        body["summary"] = json!(title);
    }
    if body["summary"].as_str().is_none_or(|s| s.trim().is_empty()) {
        return Err(AppError::validation("Event title is required"));
    }
    let all_day = args["all_day"]
        .as_bool()
        .unwrap_or(!body["start"]["date"].is_null());
    if let Some(start) = str_arg(args, "start") {
        body["start"] = dates::google_time(start, &z, all_day)?;
    }
    if let Some(end) = str_arg(args, "end") {
        body["end"] = dates::google_time(end, &z, all_day)?;
    } else if args.get("start").is_some() && base.is_none() {
        let start = dates::google_timestamp(&body["start"], &z)?;
        if all_day {
            return Err(AppError::validation(
                "All-day events need an exclusive end date",
            ));
        }
        let minutes = args["minutes"].as_u64().unwrap_or(30);
        if !(1..=1440).contains(&minutes) {
            return Err(AppError::validation("minutes must be from 1 to 1440"));
        }
        body["end"] = json!({"dateTime":dates::rfc3339(start+minutes as i64*60)?,"timeZone":z});
    }
    let start = dates::google_timestamp(&body["start"], &z)?;
    let end = dates::google_timestamp(&body["end"], &z)?;
    if end <= start {
        return Err(AppError::validation("Event end must be after start"));
    }
    for (a, b) in [
        ("description", "description"),
        ("location", "location"),
        ("recurrence", "recurrence"),
        ("reminders", "reminders"),
    ] {
        if let Some(v) = args.get(a) {
            body[b] = v.clone();
        }
    }
    if let Some(attendees) = args.get("attendees") {
        let addresses: Vec<String> = match attendees {
            Value::String(s) => s
                .split(',')
                .map(|v| v.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect(),
            Value::Array(a) => a
                .iter()
                .map(|v| {
                    v.as_str()
                        .map(String::from)
                        .ok_or_else(|| AppError::validation("Attendees must be email addresses"))
                })
                .collect::<AppResult<_>>()?,
            _ => {
                return Err(AppError::validation(
                    "Attendees must be an array of addresses",
                ));
            }
        };
        if addresses
            .iter()
            .any(|s| !s.contains('@') || s.contains(['\r', '\n']))
        {
            return Err(AppError::validation("Invalid attendee address"));
        }
        body["attendees"] = json!(
            addresses
                .iter()
                .map(|a| json!({"email":a}))
                .collect::<Vec<_>>()
        );
    }
    if args["online"].as_bool() == Some(true) {
        body["conferenceData"] = json!({"createRequest":{"requestId":hash(args),"conferenceSolutionKey":{"type":"hangoutsMeet"}}});
    }
    if let Some(r) = body["recurrence"].as_array() {
        if r.iter().any(|v| {
            v.as_str().is_none_or(|s| {
                let property = s
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .split(';')
                    .next()
                    .unwrap_or("");
                s.contains(['\r', '\n'])
                    || !s.contains(':')
                    || !matches!(property, "RRULE" | "EXRULE" | "RDATE" | "EXDATE")
            })
        }) {
            return Err(AppError::validation(
                "recurrence must contain RFC5545 recurrence properties",
            ));
        }
    }
    if calendar(args) == "local" {
        if let Some(link) = args.get("meeting_link") {
            if !link.is_null()
                && link
                    .as_str()
                    .is_none_or(|s| !s.starts_with("https://") || s.contains(['\r', '\n']))
            {
                return Err(AppError::validation(
                    "Meeting links must start with https://",
                ));
            }
            body["hangoutLink"] = link.clone();
        }
    }
    for k in [
        "revision",
        "calendar_id",
        "provider_id",
        "title",
        "etag",
        "created",
        "updated",
        "htmlLink",
        "iCalUID",
        "creator",
        "organizer",
        "kind",
        "recurringEventId",
        "originalStartTime",
    ] {
        body.as_object_mut().unwrap().remove(k);
    }
    Ok(body)
}

fn updates(args: &Value, body: Option<&Value>) -> AppResult<String> {
    let default = if body
        .and_then(|v| v["attendees"].as_array())
        .is_some_and(|a| !a.is_empty())
    {
        "all"
    } else {
        "none"
    };
    let value = str_arg(args, "send_updates").unwrap_or(default);
    if !matches!(value, "all" | "externalOnly" | "none") {
        return Err(AppError::validation(
            "send_updates must be all, externalOnly, or none",
        ));
    }
    Ok(value.into())
}

fn create_event(context: &ServiceContext, args: &Value) -> AppResult<Value> {
    let idem = req(args, "idempotency_key")?;
    if idem.len() > 256 {
        return Err(AppError::validation("idempotency_key is too long"));
    }
    let cal = calendar(args);
    let account = if cal == "local" { 0 } else { account_id(args)? };
    let idkey = key(account, cal, idem);
    let fingerprint = hash(args);
    let body = event_body(context, args, None)?;
    if cal == "local" {
        if body["recurrence"].as_array().is_some_and(|v| !v.is_empty()) {
            return Err(AppError::new(
                "unsupported_capability",
                "Local calendars do not expand recurring events; use Google Calendar",
            ));
        }
        if !body["conferenceData"].is_null() {
            return Err(AppError::new(
                "unsupported_capability",
                "Google Meet requires a Google calendar",
            ));
        }
    }
    {
        let mut state = StateFile::open()?;
        if let Some(record) = state.state.creations.get(&idkey) {
            if record.hash != fingerprint {
                return Err(AppError::new(
                    "idempotency_conflict",
                    "This key was used for different event content",
                ));
            }
            if record.status == "complete" {
                return Ok(record.result.clone());
            }
            if cal == "local" {
                return Err(AppError::new(
                    "operation_unknown",
                    "A previous creation was interrupted; inspect the local calendar before retrying",
                ));
            }
        }
        state.state.creations.insert(
            idkey.clone(),
            Creation {
                hash: fingerprint.clone(),
                status: "pending".into(),
                result: Value::Null,
            },
        );
        state.save()?;
    }
    let mut result = if cal == "local" {
        if body["recurrence"].as_array().is_some_and(|v| !v.is_empty()) {
            return Err(AppError::new(
                "unsupported_capability",
                "Local calendars do not expand recurring events; use Google Calendar",
            ));
        }
        if !body["conferenceData"].is_null() {
            return Err(AppError::new(
                "unsupported_capability",
                "Google Meet requires a Google calendar",
            ));
        }
        let z = zone(context)?;
        let mut event = Event::default();
        event.title = body["summary"].as_str().unwrap_or_default().into();
        event.start = dates::google_timestamp(&body["start"], &z)?;
        event.end = dates::google_timestamp(&body["end"], &z)?;
        event.location = body["location"].as_str().unwrap_or_default().into();
        event.notes = body["description"].as_str().unwrap_or_default().into();
        event.meeting_link = body["hangoutLink"].as_str().map(String::from);
        event.attendees = body["attendees"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["email"].as_str().map(String::from))
            .collect();
        let mut state = StateFile::open()?;
        let event = context.update(|s| {
            event.id = s.new_id();
            s.events.push(event.clone());
            Ok(event)
        })?;
        state
            .state
            .local_extra
            .insert(event.id.to_string(), body.clone());
        state.save()?;
        local_value(&event, Some(&body), &z)?
    } else {
        let api = gateway(context, account)?;
        let mut body = body;
        let deterministic = format!(
            "s{}",
            hash(&json!({"account":account,"calendar":cal,"key":idem}))
        );
        body["id"] = json!(deterministic);
        let params = vec![
            ("sendUpdates", updates(args, Some(&body))?),
            ("conferenceDataVersion", "1".into()),
        ];
        let mut remote = match api.request(
            "POST",
            &GoogleCalendar::event_path(cal, None),
            &params,
            Some(&body),
            None,
        ) {
            Err(e) if e.code == "already_exists" => api.request(
                "GET",
                &GoogleCalendar::event_path(cal, Some(&deterministic)),
                &[],
                None,
                None,
            )?,
            other => other?,
        };
        remote["revision"] = remote["etag"].clone();
        remote["calendar_id"] = json!(cal);
        remote["provider_id"] = json!("google");
        project(context, account, cal, &remote)?;
        remote
    };
    result["status"] = json!("created");
    let mut state = StateFile::open()?;
    state.state.creations.insert(
        idkey,
        Creation {
            hash: fingerprint,
            status: "complete".into(),
            result: result.clone(),
        },
    );
    state.save()?;
    Ok(result)
}

fn check_revision(args: &Value, current: &Value) -> AppResult<()> {
    let expected = req(args, "expected_revision")?;
    if current["revision"].as_str() != Some(expected) {
        return Err(AppError::new(
            "revision_conflict",
            format!(
                "Event changed; read it again. Current revision: {}",
                current["revision"]
            ),
        ));
    }
    Ok(())
}
fn mutate_event(context: &ServiceContext, name: &str, args: &Value) -> AppResult<Value> {
    if str_arg(args, "scope") == Some("following") {
        return following_events(context, name, args);
    }
    let current = read_event(context, args)?;
    check_revision(args, &current)?;
    let cal = calendar(args);
    let recurring = current["recurrence"]
        .as_array()
        .is_some_and(|a| !a.is_empty())
        || !current["recurringEventId"].is_null();
    if recurring {
        let scope = req(args, "scope")?;
        if scope == "instance" && current["recurringEventId"].is_null() {
            return Err(AppError::validation(
                "Read a specific recurring instance before changing that instance",
            ));
        }
    }
    if cal == "local" {
        let prepared = if name == "update_event" {
            Some(event_body(context, args, Some(&current))?)
        } else {
            None
        };
        let id = num(args, "id")?;
        let mut state = StateFile::open()?;
        let result = context.update(|store| {
            let at = store
                .events
                .iter()
                .position(|e| e.id == id)
                .ok_or_else(|| AppError::missing("Event"))?;
            let fresh = local_value(
                &store.events[at],
                state.state.local_extra.get(&id.to_string()),
                &context.clock.regional(&store.settings).time_zone,
            )?;
            check_revision(args, &fresh)?;
            if name == "delete_event" {
                store.events.remove(at);
                return Ok(json!({"id":id.to_string(),"status":"deleted"}));
            }
            if name == "respond_to_event" {
                return Err(AppError::new(
                    "unsupported_capability",
                    "Local events do not send invitation responses",
                ));
            }
            let body = prepared
                .clone()
                .ok_or_else(|| AppError::validation("Event edit is missing"))?;
            if body["recurrence"].as_array().is_some_and(|v| !v.is_empty()) {
                return Err(AppError::new(
                    "unsupported_capability",
                    "Use a Google calendar for recurring events",
                ));
            }
            if !body["conferenceData"].is_null() {
                return Err(AppError::new(
                    "unsupported_capability",
                    "Google Meet requires a Google calendar",
                ));
            }
            let z = context.clock.regional(&store.settings).time_zone;
            let event = &mut store.events[at];
            event.title = body["summary"].as_str().unwrap_or_default().into();
            event.start = dates::google_timestamp(&body["start"], &z)?;
            event.end = dates::google_timestamp(&body["end"], &z)?;
            event.location = body["location"].as_str().unwrap_or_default().into();
            event.notes = body["description"].as_str().unwrap_or_default().into();
            event.meeting_link = body["hangoutLink"].as_str().map(String::from);
            event.attendees = body["attendees"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|v| v["email"].as_str().map(String::from))
                .collect();
            state.state.local_extra.insert(id.to_string(), body.clone());
            local_value(event, Some(&body), &z)
        })?;
        if name == "delete_event" {
            state.state.local_extra.remove(&id.to_string());
        }
        state.save()?;
        return Ok(result);
    }
    let account = account_id(args)?;
    let api = gateway(context, account)?;
    let mut id = req(args, "id")?.to_string();
    let mut base = current.clone();
    if str_arg(args, "scope") == Some("series") {
        if let Some(series) = current["recurringEventId"].as_str() {
            id = series.into();
            base = api.request(
                "GET",
                &GoogleCalendar::event_path(cal, Some(&id)),
                &[],
                None,
                None,
            )?;
            if base["etag"] != args["expected_revision"] {
                return Err(AppError::new(
                    "revision_conflict",
                    "Read the series master and use its revision for a series change",
                ));
            }
        }
    }
    let params = vec![
        ("sendUpdates", updates(args, Some(&base))?),
        ("conferenceDataVersion", "1".into()),
    ];
    if name == "delete_event" {
        api.request(
            "DELETE",
            &GoogleCalendar::event_path(cal, Some(&id)),
            &params,
            None,
            base["etag"].as_str(),
        )?;
        remove_projection(context, account, cal, &id)?;
        return Ok(json!({"id":id,"status":"deleted"}));
    }
    let body = if name == "respond_to_event" {
        let status = req(args, "rsvp")?;
        if !matches!(status, "accepted" | "tentative" | "declined") {
            return Err(AppError::validation("Invalid RSVP"));
        }
        let mut attendees = base["attendees"]
            .as_array()
            .cloned()
            .ok_or_else(|| AppError::validation("This event has no invitation for this account"))?;
        let mut found = false;
        for attendee in &mut attendees {
            if attendee["self"].as_bool() == Some(true) {
                attendee["responseStatus"] = json!(status);
                found = true;
            }
        }
        if !found {
            return Err(AppError::validation("This account is not an attendee"));
        }
        json!({"attendees":attendees})
    } else {
        event_body(context, args, Some(&base))?
    };
    let mut result = api.request(
        "PATCH",
        &GoogleCalendar::event_path(cal, Some(&id)),
        &params,
        Some(&body),
        base["etag"].as_str(),
    )?;
    result["revision"] = result["etag"].clone();
    result["calendar_id"] = json!(cal);
    result["provider_id"] = json!("google");
    project(context, account, cal, &result)?;
    Ok(result)
}

fn following_events(context: &ServiceContext, name: &str, args: &Value) -> AppResult<Value> {
    if calendar(args) == "local" {
        return Err(AppError::new(
            "unsupported_capability",
            "Local calendars have no recurring series",
        ));
    }
    if !matches!(name, "update_event" | "delete_event") {
        return Err(AppError::validation(
            "Following scope is only valid for an update or deletion",
        ));
    }
    let account = account_id(args)?;
    let cal = calendar(args);
    let operation_id = hash(&json!({"tool":name,"arguments":args}));
    let cached = StateFile::open()?
        .state
        .series_operations
        .get(&operation_id)
        .cloned();
    let api = gateway(context, account)?;
    let mut operation = if let Some(op) = cached {
        if op.status == "complete" {
            return Ok(op.result);
        }
        op
    } else {
        let instance = read_event(context, args)?;
        check_revision(args, &instance)?;
        let master_id = instance["recurringEventId"].as_str().ok_or_else(|| {
            AppError::validation("Select a specific recurring instance to split at that occurrence")
        })?;
        let master = api.request(
            "GET",
            &GoogleCalendar::event_path(cal, Some(master_id)),
            &[],
            None,
            None,
        )?;
        let rules = master["recurrence"]
            .as_array()
            .ok_or_else(|| AppError::validation("Series has no recurrence rules"))?;
        if rules.len() != 1 || rules[0].as_str().is_none_or(|r| !r.starts_with("RRULE:")) {
            return Err(AppError::new(
                "unsupported_capability",
                "Split each RRULE/exception set explicitly; following edits currently support a single RRULE",
            ));
        }
        let op = SeriesOperation {
            account_id: account,
            calendar_id: cal.into(),
            master,
            instance,
            status: "prepared".into(),
            new_event_id: format!("s{operation_id}"),
            result: Value::Null,
        };
        let mut state = StateFile::open()?;
        state
            .state
            .series_operations
            .insert(operation_id.clone(), op.clone());
        state.save()?;
        op
    };
    let result = (|| -> AppResult<Value> {
        let master_id = operation.master["id"]
            .as_str()
            .ok_or_else(|| AppError::validation("Series ID missing"))?;
        let original = if operation.instance["originalStartTime"].is_object() {
            &operation.instance["originalStartTime"]
        } else {
            &operation.instance["start"]
        };
        let boundary = dates::google_timestamp(original, &zone(context)?)?;
        let cutoff = chrono::DateTime::from_timestamp(boundary - 1, 0)
            .ok_or_else(|| AppError::validation("Series boundary out of range"))?
            .format("%Y%m%dT%H%M%SZ")
            .to_string();
        let old_rule = operation.master["recurrence"][0].as_str().unwrap();
        let parts: Vec<&str> = old_rule
            .strip_prefix("RRULE:")
            .unwrap()
            .split(';')
            .collect();
        let remaining_count = if let Some(count) = parts
            .iter()
            .find_map(|p| p.strip_prefix("COUNT="))
            .and_then(|n| n.parse::<usize>().ok())
        {
            let mut prior = 0usize;
            let mut page = None;
            loop {
                let mut params = vec![
                    ("timeMax", dates::rfc3339(boundary)?),
                    ("maxResults", "1000".into()),
                    ("showDeleted", "true".into()),
                ];
                if let Some(token) = page {
                    params.push(("pageToken", token));
                }
                let result = api.request(
                    "GET",
                    &format!(
                        "{}/instances",
                        GoogleCalendar::event_path(cal, Some(master_id))
                    ),
                    &params,
                    None,
                    None,
                )?;
                prior += result["items"].as_array().map_or(0, Vec::len);
                if prior > 100000 {
                    return Err(AppError::validation("Series too large to split safely"));
                }
                match result["nextPageToken"].as_str() {
                    Some(v) => page = Some(v.to_string()),
                    None => break,
                }
            }
            Some(count.saturating_sub(prior))
        } else {
            None
        };
        // Validate every replacement field before the first provider mutation.
        // In particular, an invalid end/time zone or an exhausted COUNT must
        // never truncate the existing series before failing.
        let replacement = if name == "update_event" {
            Some(following_replacement(context, args, &operation, remaining_count)?)
        } else {
            None
        };
        let truncate_updates = updates(args, Some(&operation.master))?;
        if operation.status == "prepared" {
            let mut truncated: Vec<_> = parts
                .iter()
                .filter(|p| !p.starts_with("UNTIL=") && !p.starts_with("COUNT="))
                .map(|p| p.to_string())
                .collect();
            truncated.push(format!("UNTIL={cutoff}"));
            let truncated = format!("RRULE:{}", truncated.join(";"));
            let params = vec![("sendUpdates", truncate_updates)];
            let fresh = api.request(
                "GET",
                &GoogleCalendar::event_path(cal, Some(master_id)),
                &[],
                None,
                None,
            )?;
            if fresh["recurrence"] != json!([truncated]) {
                if fresh["etag"] != operation.master["etag"] {
                    return Err(AppError::new(
                        "revision_conflict",
                        "Series changed before it could be split; no later event was created",
                    ));
                }
                api.request(
                    "PATCH",
                    &GoogleCalendar::event_path(cal, Some(master_id)),
                    &params,
                    Some(&json!({"recurrence":[truncated]})),
                    operation.master["etag"].as_str(),
                )?;
            }
            operation.status = "original_truncated".into();
            save_series_operation(&operation_id, &operation)?;
        }
        if name == "delete_event" {
            return Ok(
                json!({"operation_id":operation_id,"status":"complete","deleted_from":operation.instance["originalStartTime"],"series_id":master_id}),
            );
        }
        let (body, params) = replacement.expect("an update prepared its replacement before truncation");
        let created = match api.request(
            "POST",
            &GoogleCalendar::event_path(cal, None),
            &params,
            Some(&body),
            None,
        ) {
            Err(e) if e.code == "already_exists" => api.request(
                "GET",
                &GoogleCalendar::event_path(cal, Some(&operation.new_event_id)),
                &[],
                None,
                None,
            )?,
            other => other?,
        };
        project(context, account, cal, &created)?;
        Ok(
            json!({"operation_id":operation_id,"status":"complete","original_series_id":master_id,"following_series":created}),
        )
    })();
    match result {
        Ok(value) => {
            operation.status = "complete".into();
            operation.result = value.clone();
            save_series_operation(&operation_id, &operation)?;
            Ok(value)
        }
        Err(error) => {
            let phase = operation.status.clone();
            operation.result = json!({"error":{"code":error.code,"message":error.message}});
            save_series_operation(&operation_id, &operation)?;
            Err(AppError::new(
                if phase == "prepared" {
                    error.code
                } else {
                    "calendar_partial_operation"
                },
                format!(
                    "{} Calendar operation {operation_id}, phase {phase}. Inspect get_calendar_operation or repeat exactly this request to resume.",
                    error.message
                ),
            ))
        }
    }
}
fn following_replacement(
    context: &ServiceContext,
    args: &Value,
    operation: &SeriesOperation,
    remaining_count: Option<usize>,
) -> AppResult<(Value, Vec<(&'static str, String)>)> {
    let mut body = operation.master.clone();
    body["start"] = operation.instance["start"].clone();
    body["end"] = operation.instance["end"].clone();
    let mut rules: Vec<_> = operation.master["recurrence"][0]
        .as_str()
        .and_then(|r| r.strip_prefix("RRULE:"))
        .ok_or_else(|| AppError::validation("Series has no supported recurrence rule"))?
        .split(';')
        .filter(|p| !p.starts_with("COUNT="))
        .map(String::from)
        .collect();
    if let Some(count) = remaining_count {
        if count == 0 {
            return Err(AppError::validation("No remaining occurrences to create"));
        }
        rules.push(format!("COUNT={count}"));
    }
    body["recurrence"] = json!([format!("RRULE:{}", rules.join(";"))]);
    body = event_body(context, args, Some(&body))?;
    body["id"] = json!(operation.new_event_id);
    for field in [
        "etag", "created", "updated", "htmlLink", "iCalUID", "organizer", "creator",
        "sequence", "recurringEventId", "originalStartTime", "status",
    ] {
        body.as_object_mut().unwrap().remove(field);
    }
    let params = vec![
        ("sendUpdates", updates(args, Some(&body))?),
        ("conferenceDataVersion", "1".into()),
    ];
    Ok((body, params))
}
fn save_series_operation(id: &str, operation: &SeriesOperation) -> AppResult<()> {
    let mut state = StateFile::open()?;
    state
        .state
        .series_operations
        .insert(id.into(), operation.clone());
    state.save()
}

fn project(context: &ServiceContext, account: u32, cal: &str, remote: &Value) -> AppResult<()> {
    let id = remote["id"]
        .as_str()
        .ok_or_else(|| AppError::external("Google event has no ID"))?;
    if remote["status"].as_str() == Some("cancelled") {
        return remove_projection(context, account, cal, id);
    }
    let z = zone(context)?;
    let start = dates::google_timestamp(&remote["start"], &z)?;
    let end = dates::google_timestamp(&remote["end"], &z)?;
    let mut state = StateFile::open()?;
    let k = key(account, cal, id);
    let existing = state.state.remote.get(&k).map(|p| p.local_id);
    let local_id = context.update(|store| {
        let local_id = existing.unwrap_or_else(|| store.new_id());
        let mut event = Event {
            id: local_id,
            title: remote["summary"].as_str().unwrap_or("(untitled)").into(),
            start,
            end,
            location: remote["location"].as_str().unwrap_or_default().into(),
            notes: remote["description"].as_str().unwrap_or_default().into(),
            meeting_link: remote["hangoutLink"].as_str().map(String::from),
            conference: if remote["conferenceData"].is_object() {
                "googleMeet".into()
            } else {
                String::new()
            },
            ..Default::default()
        };
        event.attendees = remote["attendees"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|v| v["email"].as_str().map(String::from))
            .collect();
        event.rsvp = remote["attendees"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|a| a["self"].as_bool() == Some(true))
            .and_then(|a| a["responseStatus"].as_str())
            .map(|s| match s {
                "accepted" => Rsvp::Accepted,
                "declined" => Rsvp::Declined,
                "tentative" => Rsvp::Maybe,
                _ => Rsvp::None,
            })
            .unwrap_or_default();
        if let Some(at) = store.events.iter().position(|e| e.id == local_id) {
            store.events[at] = event;
        } else {
            store.events.push(event);
        }
        Ok(local_id)
    })?;
    state.state.remote.insert(
        k,
        RemoteProjection {
            local_id,
            account_id: account,
            calendar_id: cal.into(),
            event: remote.clone(),
        },
    );
    state.save()
}
fn remove_projection(context: &ServiceContext, account: u32, cal: &str, id: &str) -> AppResult<()> {
    let mut state = StateFile::open()?;
    if let Some(p) = state.state.remote.remove(&key(account, cal, id)) {
        context.update(|s| {
            s.events.retain(|e| e.id != p.local_id);
            Ok(())
        })?;
        state.save()?;
    }
    Ok(())
}
/// One provider sync at a time in this process: a full scan removes projections
/// it did not see, so a concurrent sync of the same calendar must not run.
static SYNC_LOCK: Mutex<()> = Mutex::new(());

fn sync_calendar(context: &ServiceContext, args: &Value) -> AppResult<Value> {
    let cal = calendar(args);
    if cal == "local" {
        return Ok(json!({"calendar_id":"local","status":"complete","source":"local"}));
    }
    let _sync = SYNC_LOCK.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let account = account_id(args)?;
    let api = gateway(context, account)?;
    let token_key = key(account, cal, "");
    let token = StateFile::open()?
        .state
        .sync_tokens
        .get(&token_key)
        .cloned();
    let known_before: HashSet<String> = StateFile::open()?
        .state
        .remote
        .values()
        .filter(|p| p.account_id == account && p.calendar_id == cal)
        .filter_map(|p| p.event["id"].as_str().map(String::from))
        .collect();
    let mut reset = token.is_none();
    let mut params = vec![("maxResults", "100".into()), ("showDeleted", "true".into())];
    if let Some(token) = token {
        params.push(("syncToken", token));
    }
    let mut count = 0;
    let mut seen = Vec::new();
    let next_token;
    loop {
        let result = match api.request(
            "GET",
            &GoogleCalendar::event_path(cal, None),
            &params,
            None,
            None,
        ) {
            Err(e) if e.code == "sync_token_expired" && !reset => {
                params.retain(|(k, _)| *k != "syncToken" && *k != "pageToken");
                reset = true;
                continue;
            }
            other => other?,
        };
        for event in result["items"].as_array().into_iter().flatten() {
            project(context, account, cal, event)?;
            count += 1;
            if let Some(id) = event["id"].as_str() {
                seen.push(id.to_string());
            }
        }
        if count > 100000 {
            return Err(AppError::new(
                "range_too_large",
                "Calendar sync exceeded its safety limit; narrow this account's calendar",
            ));
        }
        if let Some(next) = result["nextPageToken"].as_str() {
            params.retain(|(k, _)| *k != "pageToken");
            params.push(("pageToken", next.into()));
        } else {
            next_token = result["nextSyncToken"].as_str().map(String::from);
            break;
        }
    }
    // Only remove old remote projections after a complete replacement scan.
    if reset {
        let stale: Vec<_> = StateFile::open()?
            .state
            .remote
            .values()
            .filter(|p| {
                p.account_id == account
                    && p.calendar_id == cal
                    && !seen.iter().any(|id| p.event["id"].as_str() == Some(id))
            })
            .filter_map(|p| p.event["id"].as_str().map(String::from))
            // An event written while the scan ran is newer than the scan, not stale.
            .filter(|id| known_before.contains(id))
            .collect();
        for id in stale {
            remove_projection(context, account, cal, &id)?;
        }
    }
    if let Some(token) = next_token {
        let mut state = StateFile::open()?;
        state.state.sync_tokens.insert(token_key, token);
        state.save()?;
    }
    Ok(
        json!({"calendar_id":cal,"account_id":account,"status":"complete","updated":count,"full_resync":reset,"updated_at":context.clock.now()}),
    )
}

fn find_slots(
    context: &ServiceContext,
    args: &Value,
    allow: Option<&[String]>,
) -> AppResult<Value> {
    let mut args = args.clone();
    let prefs = super::tools::scheduling_preferences(context)?;
    let z = str_arg(&args, "time_zone")
        .map(String::from)
        .unwrap_or(zone(context)?);
    z.parse::<chrono_tz::Tz>()
        .map_err(|_| AppError::validation("Choose an IANA time zone"))?;
    let work_start = str_arg(&args, "work_start")
        .or_else(|| prefs["work_start"].as_str())
        .unwrap_or("09:00")
        .to_string();
    let work_end = str_arg(&args, "work_end")
        .or_else(|| prefs["work_end"].as_str())
        .unwrap_or("18:00")
        .to_string();
    let weekdays = args
        .get("weekdays")
        .or_else(|| prefs.get("weekdays"))
        .and_then(Value::as_array)
        .map(|values| {
            values
                .iter()
                .map(|v| {
                    v.as_u64()
                        .and_then(|d| u32::try_from(d).ok())
                        .ok_or_else(|| AppError::validation("weekdays must contain integers 1–7"))
                })
                .collect::<AppResult<Vec<_>>>()
        })
        .transpose()?
        .unwrap_or_else(|| vec![1, 2, 3, 4, 5]);
    if let Some(date) = str_arg(&args, "date").map(String::from) {
        let day = chrono::NaiveDate::parse_from_str(&date, "%Y-%m-%d")
            .map_err(|_| AppError::validation("date must use YYYY-MM-DD"))?;
        let until = day
            .succ_opt()
            .ok_or_else(|| AppError::validation("Date is out of range"))?;
        args["from"] = json!(dates::rfc3339(
            crate::domain::regional::day_start(day, &z).map_err(AppError::validation)?
        )?);
        args["to"] = json!(dates::rfc3339(
            crate::domain::regional::day_start(until, &z).map_err(AppError::validation)?
        )?);
    }
    let from = str_arg(&args, "from")
        .map(|v| dates::timestamp(v, &z))
        .transpose()?
        .unwrap_or(context.clock.now());
    let to = str_arg(&args, "to")
        .map(|v| dates::timestamp(v, &z))
        .transpose()?
        .unwrap_or(from + 7 * 86400);
    if to <= from || to.saturating_sub(from) > 366 * 86400 {
        return Err(AppError::validation(
            "Choose an increasing range of at most 366 days",
        ));
    }
    let mut busy = Vec::new();
    let mut errors = Vec::new();
    let mut calendars = args["calendar_ids"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(String::from)
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|| vec![calendar(&args).into()]);
    calendars.sort();
    calendars.dedup();
    for cal in &calendars {
        allowed(cal, allow)?;
    }
    if calendars.iter().any(|c| c == "local") {
        let state = StateFile::open()?;
        busy.extend(
            context
                .read()?
                .events
                .into_iter()
                .filter(|e| !state.state.remote.values().any(|p| p.local_id == e.id))
                .map(|e| (e.start, e.end)),
        );
    }
    let remote: Vec<_> = calendars.iter().filter(|c| c.as_str() != "local").collect();
    if !remote.is_empty() {
        let api = gateway(context, account_id(&args)?)?;
        let result=api.request("POST","/freeBusy",&[],Some(&json!({"timeMin":dates::rfc3339(from)?,"timeMax":dates::rfc3339(to)?,"timeZone":z,"items":remote.iter().map(|id|json!({"id":id})).collect::<Vec<_>>()})),None)?;
        for cal in remote {
            let value = &result["calendars"][cal];
            if value["errors"].as_array().is_some_and(|a| !a.is_empty()) || value.is_null() {
                errors.push(json!({"calendar_id":cal,"status":"unknown","errors":value["errors"]}));
                continue;
            }
            for b in value["busy"].as_array().into_iter().flatten() {
                busy.push((
                    dates::timestamp(b["start"].as_str().unwrap_or(""), "UTC")?,
                    dates::timestamp(b["end"].as_str().unwrap_or(""), "UTC")?,
                ));
            }
        }
    }
    let slots = dates::working_slots(
        busy,
        from,
        to,
        &z,
        &work_start,
        &work_end,
        &weekdays,
        args["minutes"].as_u64().unwrap_or(30).min(u32::MAX as u64) as u32,
        args["buffer_minutes"].as_u64().unwrap_or(0).min(1440) as u32,
    )?;
    Ok(
        json!({"slots":slots,"errors":errors,"meta":{"coverage":if errors.is_empty(){"complete_for_scope"}else{"partial"},"checked_at":context.clock.now(),"calendar_ids":calendars}}),
    )
}

fn import_events(context: &ServiceContext, args: &Value) -> AppResult<Value> {
    let text;
    let ics = match (str_arg(args, "ics"), str_arg(args, "handle")) {
        (Some(_), Some(_)) => {
            return Err(AppError::validation(
                "Choose either an ICS file handle or inline ics text",
            ));
        }
        (Some(ics), None) => ics,
        (None, Some(handle)) => {
            let ids = context
                .read()?
                .accounts
                .iter()
                .map(|a| a.id)
                .collect::<Vec<_>>();
            let path = super::files::resolve_scoped(handle, Some(&ids))?;
            let mut bytes = Vec::new();
            File::open(path)?
                .take(1024 * 1024 + 1)
                .read_to_end(&mut bytes)?;
            if bytes.len() > 1024 * 1024 {
                return Err(AppError::validation("ICS input exceeds 1 MiB"));
            }
            text = String::from_utf8(bytes)
                .map_err(|_| AppError::validation("ICS file must contain UTF-8 text"))?;
            text.as_str()
        }
        (None, None) => {
            return Err(AppError::validation(
                "An ICS file handle or inline ics text is required",
            ));
        }
    };
    if ics.len() > 1024 * 1024 {
        return Err(AppError::validation("ICS input exceeds 1 MiB"));
    }
    let events = parse_ics(ics)?;
    if args["apply"].as_bool() != Some(true) {
        return Ok(
            json!({"preview":true,"count":events.len(),"events":events,"warnings":["Review calendar, dates and send_updates before applying"]}),
        );
    }
    let idem = req(args, "idempotency_key")?;
    let mut results = Vec::new();
    for (index, event) in events.iter().enumerate() {
        let mut event = event.clone();
        for k in ["account_id", "calendar_id", "send_updates"] {
            if let Some(v) = args.get(k) {
                event[k] = v.clone();
            }
        }
        event["idempotency_key"] = json!(format!("{idem}:{index}"));
        match create_event(context, &event) {
            Ok(value) => results.push(json!({"status":"succeeded","event":value})),
            Err(error) => {
                results.push(json!({"status":"failed","code":error.code,"message":error.message}))
            }
        }
    }
    Ok(json!({"preview":false,"results":results}))
}
/// Undoes ICS text escaping in one pass, so an escaped backslash followed by `n` stays two characters.
fn unescape_ics_text(value: &str) -> String {
    let mut text = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            text.push(c);
            continue;
        }
        match chars.next() {
            Some('n' | 'N') => text.push('\n'),
            Some(escaped @ (',' | ';' | '\\')) => text.push(escaped),
            Some(other) => {
                text.push('\\');
                text.push(other);
            }
            None => text.push('\\'),
        }
    }
    text
}
fn parse_ics(input: &str) -> AppResult<Vec<Value>> {
    // Lines end in CRLF or, in files written by hand, a bare LF; a line starting with a space or tab continues the one before.
    let unfolded = input.replace("\r\n", "\n").replace("\n ", "").replace("\n\t", "");
    let mut events = Vec::new();
    let mut current: Option<Value> = None;
    // Depth inside components nested in an event (a VALARM): their properties are not the event's.
    let mut nested = 0usize;
    for line in unfolded.lines() {
        let line = line.trim_end_matches('\r');
        if current.is_some() {
            if line.starts_with("BEGIN:") && line != "BEGIN:VEVENT" {
                nested += 1;
                continue;
            }
            if nested > 0 {
                if line.starts_with("END:") {
                    nested -= 1;
                }
                continue;
            }
        }
        if line == "BEGIN:VEVENT" {
            if current.is_some() {
                return Err(AppError::validation("Nested VEVENT"));
            }
            current = Some(json!({}));
            continue;
        }
        if line == "END:VEVENT" {
            let event = current
                .take()
                .ok_or_else(|| AppError::validation("Unexpected END:VEVENT"))?;
            if event["start"].is_null() || event["end"].is_null() || event["title"].is_null() {
                return Err(AppError::validation(
                    "ICS events need SUMMARY, DTSTART and DTEND",
                ));
            }
            events.push(event);
            continue;
        }
        let Some(event) = current.as_mut() else {
            continue;
        };
        let Some((field, value)) = line.split_once(':') else {
            continue;
        };
        let (name, params) = field.split_once(';').unwrap_or((field, ""));
        let text = || unescape_ics_text(value);
        match name {
            "SUMMARY" => event["title"] = json!(text()),
            "DESCRIPTION" => event["description"] = json!(text()),
            "LOCATION" => event["location"] = json!(text()),
            "DTSTART" | "DTEND" => {
                let key = if name == "DTSTART" { "start" } else { "end" };
                if params.split(';').any(|p| p == "VALUE=DATE") {
                    let date = chrono::NaiveDate::parse_from_str(value, "%Y%m%d")
                        .map_err(|_| AppError::validation("Invalid all-day ICS date"))?;
                    event[key] = json!(date.format("%Y-%m-%d").to_string());
                    event["all_day"] = json!(true);
                } else if value.ends_with('Z') {
                    let dt = chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%SZ")
                        .map_err(|_| AppError::validation("Invalid UTC ICS timestamp"))?;
                    event[key] = json!(dt.and_utc().to_rfc3339());
                } else {
                    let tz = params
                        .split(';')
                        .find_map(|p| p.strip_prefix("TZID="))
                        .ok_or_else(|| {
                            AppError::validation("Floating ICS times need an explicit TZID")
                        })?;
                    let date = chrono::NaiveDateTime::parse_from_str(value, "%Y%m%dT%H%M%S")
                        .map_err(|_| AppError::validation("Invalid ICS date-time"))?;
                    let ts = crate::domain::regional::local_timestamp(date, tz)
                        .map_err(AppError::validation)?;
                    event[key] = json!(dates::rfc3339(ts)?);
                    event["time_zone"] = json!(tz);
                }
            }
            "RRULE" | "EXRULE" | "RDATE" | "EXDATE" => {
                if event["recurrence"].is_null() {
                    event["recurrence"] = json!([]);
                }
                event["recurrence"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(format!("{field}:{value}")));
            }
            "ATTENDEE" => {
                let address = value
                    .strip_prefix("mailto:")
                    .or_else(|| value.strip_prefix("MAILTO:"))
                    .unwrap_or(value);
                if event["attendees"].is_null() {
                    event["attendees"] = json!([]);
                }
                event["attendees"]
                    .as_array_mut()
                    .unwrap()
                    .push(json!(address));
            }
            _ => {}
        }
    }
    if current.is_some() {
        return Err(AppError::validation("Unclosed VEVENT"));
    }
    if events.is_empty() {
        return Err(AppError::validation("ICS contains no VEVENT entries"));
    }
    if events.len() > 100 {
        return Err(AppError::validation(
            "Import at most 100 events per request",
        ));
    }
    Ok(events)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_google_occurrence_links_to_its_own_or_its_series_projection() {
        let projection = |local_id, account_id, event: Value| RemoteProjection {
            local_id,
            account_id,
            calendar_id: "me@example.com".into(),
            event,
        };
        let mut state = CalendarState::default();
        state.remote.insert("a".into(), projection(10, 1, json!({"id": "single"})));
        state.remote.insert("b".into(), projection(11, 1, json!({"id": "weekly"})));
        state.remote.insert("c".into(), projection(12, 2, json!({"id": "weekly"})));
        state.remote.insert("d".into(), projection(13, 1, json!({})));
        let instance = json!({"id": "weekly_20261103T090000Z", "recurringEventId": "weekly"});
        assert_eq!(linked_local_event(&state, 1, &json!({"id": "single"})), Some(10));
        assert_eq!(linked_local_event(&state, 1, &instance), Some(11), "an occurrence links to its series");
        assert_eq!(linked_local_event(&state, 2, &instance), Some(12), "only the same account counts");
        assert_eq!(linked_local_event(&state, 1, &json!({"id": "other"})), None, "a missing id never matches");
    }
    fn calendar_context() -> ServiceContext {
        use crate::infrastructure::{gateways::*, repository::MemoryRepository};
        let mut store = Store::default();
        store.settings.time_zone = "UTC".into();
        ServiceContext { compute: std::sync::Arc::new(crate::infrastructure::fake_compute::FakeCompute::default()),
            events: Arc::new(crate::application::ports::NullSink),
            repository: Arc::new(MemoryRepository::new(store)),
            mail: Arc::new(GmailGateway::new(PathBuf::new())),
            secrets: Arc::new(KeyringSecrets),
            clock: Arc::new(SystemClock),
            attachments: Arc::new(LocalAttachments),
        }
    }
    #[test]
    fn following_replacement_validates_all_fields_before_series_truncation() {
        let context = calendar_context();
        let operation = SeriesOperation {
            account_id: 1,
            calendar_id: "synthetic".into(),
            master: json!({"id":"master","etag":"revision","summary":"Recurring",
                "start":{"dateTime":"2026-11-03T09:00:00Z"},
                "end":{"dateTime":"2026-11-03T10:00:00Z"},
                "recurrence":["RRULE:FREQ=DAILY;COUNT=5"]}),
            instance: json!({"start":{"dateTime":"2026-11-05T09:00:00Z"},
                "end":{"dateTime":"2026-11-05T10:00:00Z"}}),
            status: "prepared".into(),
            new_event_id: "replacement".into(),
            result: Value::Null,
        };
        for invalid in [
            json!({"end":"2026-11-05T08:00:00Z"}),
            json!({"time_zone":"invalid/zone"}),
            json!({"attendees":["invalid"]}),
            json!({"recurrence":["invalid"]}),
            json!({"send_updates":"invalid"}),
        ] {
            assert!(following_replacement(&context, &invalid, &operation, Some(3)).is_err());
        }
        assert!(following_replacement(&context, &json!({}), &operation, Some(0)).is_err());
        let (body, params) = following_replacement(&context, &json!({}), &operation, Some(3)).unwrap();
        assert_eq!(body["id"], "replacement");
        assert_eq!(body["recurrence"], json!(["RRULE:FREQ=DAILY;COUNT=3"]));
        assert_eq!(body["start"], operation.instance["start"]);
        assert!(body.get("etag").is_none());
        assert!(params.iter().any(|(key, value)| *key == "sendUpdates" && value == "none"));
        assert_eq!(operation.master["recurrence"], json!(["RRULE:FREQ=DAILY;COUNT=5"]));
    }
    #[test]
    fn ics_round_trip_preserves_dates_and_escaped_notes() {
        let source = json!({"id":"test","summary":"Hi, all","start":{"date":"2026-10-25"},"end":{"date":"2026-10-26"},"description":"line1\nline2"});
        let ics = dates::export_ics(&[source], "Europe/Rome").unwrap();
        let events = parse_ics(&ics).unwrap();
        assert_eq!(events[0]["start"], "2026-10-25");
        assert_eq!(events[0]["description"], "line1\nline2");
    }
    #[test]
    fn ics_recurring_round_trip_preserves_civil_time_zone_across_dst() {
        let source = json!({"id":"weekly","summary":"Weekly","start":{"dateTime":"2026-10-23T09:00:00+02:00","timeZone":"Europe/Rome"},"end":{"dateTime":"2026-10-23T10:00:00+02:00","timeZone":"Europe/Rome"},"recurrence":["RRULE:FREQ=WEEKLY;COUNT=3"]});
        let exported = dates::export_ics(&[source], "UTC").unwrap();
        assert!(exported.contains("DTSTART;TZID=Europe/Rome:20261023T090000"));
        let parsed = parse_ics(&exported).unwrap();
        assert_eq!(parsed[0]["time_zone"], "Europe/Rome");
        assert_eq!(parsed[0]["start"], "2026-10-23T07:00:00+00:00");
        assert_eq!(parsed[0]["recurrence"][0], "RRULE:FREQ=WEEKLY;COUNT=3");
    }
    #[test]
    fn profile_checks_actual_calendar_identity() {
        let ids = vec!["work@example.com".into()];
        assert!(allowed("local", Some(&ids)).is_err());
        assert!(allowed("work@example.com", Some(&ids)).is_ok());
    }
    #[test]
    fn calendar_ids_cannot_collide_by_concatenation() {
        assert_ne!(key(1, "ab", "c"), key(1, "a", "bc"));
    }
    fn ics_event(lines: &[&str]) -> String {
        let mut text = String::from("BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\n");
        for line in lines {
            text.push_str(line);
            text.push_str("\r\n");
        }
        text.push_str("END:VEVENT\r\nEND:VCALENDAR\r\n");
        text
    }
    #[test]
    fn an_ics_reminder_does_not_replace_the_event_text() {
        let ics = ics_event(&[
            "SUMMARY:Riunione", "DESCRIPTION:Ordine del giorno", "LOCATION:Sala 1",
            "DTSTART:20261104T100000Z", "DTEND:20261104T110000Z",
            "BEGIN:VALARM", "ACTION:DISPLAY", "DESCRIPTION:Reminder", "SUMMARY:Alarm", "TRIGGER:-PT15M", "END:VALARM",
        ]);
        let event = &parse_ics(&ics).unwrap()[0];
        assert_eq!(event["title"], "Riunione");
        assert_eq!(event["description"], "Ordine del giorno");
    }
    #[test]
    fn an_ics_with_lf_only_lines_still_unfolds_long_values() {
        let ics = "BEGIN:VEVENT\nSUMMARY:Titolo molto\n  lungo continuato\nDTSTART:20261104T100000Z\nDTEND:20261104T110000Z\nEND:VEVENT\n";
        assert_eq!(parse_ics(ics).unwrap()[0]["title"], "Titolo molto lungo continuato");
    }
    #[test]
    fn an_ics_value_type_of_date_time_is_not_an_all_day_date() {
        let ics = ics_event(&["SUMMARY:X", "DTSTART;VALUE=DATE-TIME:20261104T100000Z", "DTEND;VALUE=DATE-TIME:20261104T110000Z"]);
        let event = &parse_ics(&ics).unwrap()[0];
        assert_eq!(event["start"], "2026-11-04T10:00:00+00:00");
        assert!(event["all_day"].is_null());
    }
    #[test]
    fn an_escaped_backslash_before_n_is_not_a_line_break() {
        // In the file the text is C:, two backslashes, then nuovo: an escaped backslash followed by plain letters.
        let ics = ics_event(&["SUMMARY:C:\\\\nuovo", "DTSTART:20261104T100000Z", "DTEND:20261104T110000Z"]);
        assert_eq!(parse_ics(&ics).unwrap()[0]["title"], "C:\\nuovo");
    }
    #[test]
    fn ics_rejects_floating_times_and_incomplete_events() {
        assert!(parse_ics("BEGIN:VEVENT\nSUMMARY:test\nDTSTART:20261102T090000\nDTEND:20261102T100000\nEND:VEVENT").is_err());
        assert!(parse_ics("BEGIN:VEVENT\nSUMMARY:test\nEND:VEVENT").is_err());
    }
}
