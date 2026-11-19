//! Meeting reminder chips: periodic, read-only checks of the calendars and the
//! commands a chip offers. The desktop adapter owns the window and the browser.
use super::{AppError, AppResult, ServiceContext, meetings::MeetingService};
use crate::domain::meeting::{CaptureState, StartMeeting};
use crate::domain::model::Store;
use crate::domain::reminders::{self, ReminderCandidate, ReminderChip};
use serde::Serialize;
use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, Mutex};

/// Google is read at most once a minute per account.
pub const GOOGLE_REFRESH_SECS: i64 = 60;

/// Read-only calendar access. Implementations never write the workspace or
/// calendar metadata. `store` is the workspace the check already read, so a
/// check parses it once.
pub trait ReminderCalendar: Send + Sync {
    fn local(&self, store: &Store, from: i64, to: i64) -> AppResult<Vec<ReminderCandidate>>;
    /// Gmail accounts that already granted calendar access.
    fn google_accounts(&self, store: &Store) -> AppResult<Vec<u32>>;
    fn google(&self, store: &Store, account: u32, from: i64, to: i64) -> AppResult<Vec<ReminderCandidate>>;
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ReminderView {
    pub chips: Vec<ReminderChip>,
    /// A recording is already in progress: "Join and record" is unavailable.
    pub recording: bool,
    pub dark: bool,
    pub locale: String,
    pub time_zone: String,
}

#[derive(Debug, Default)]
pub struct GoogleRefresh {
    /// New, distinct errors for the main window.
    pub notices: Vec<String>,
    /// The cached Google events changed: the chips should be checked now.
    pub updated: bool,
}

pub struct ReminderTick {
    pub view: ReminderView,
    pub changed: bool,
    /// New, distinct errors for the main window.
    pub notices: Vec<String>,
}

struct GoogleRead {
    read_at: i64,
    lead: u32,
    items: Vec<ReminderCandidate>,
}

#[derive(Default)]
struct State {
    handled: HashSet<String>,
    google: BTreeMap<u32, GoogleRead>,
    errors: BTreeMap<String, String>,
    view: ReminderView,
    /// The last check's candidates and lead time, so closing a chip makes room
    /// for the next meeting at once.
    candidates: Vec<ReminderCandidate>,
    lead: u32,
}

#[derive(Clone)]
pub struct MeetingReminderService {
    context: Arc<ServiceContext>,
    calendar: Arc<dyn ReminderCalendar>,
    meetings: MeetingService,
    state: Arc<Mutex<State>>,
    /// One check at a time, each from a fresh time and workspace, so an older
    /// result never replaces a newer one.
    ticking: Arc<Mutex<()>>,
    /// Recording start ids are idempotent per app session only, so a chip shown
    /// again after a restart can still start a new recording.
    session: String,
}

impl MeetingReminderService {
    pub fn new(
        context: Arc<ServiceContext>,
        calendar: Arc<dyn ReminderCalendar>,
        meetings: MeetingService,
    ) -> Self {
        let session = context.clock.now_ms().to_string();
        Self { context, calendar, meetings, state: Arc::default(), ticking: Arc::default(), session }
    }

    fn lock(&self) -> AppResult<std::sync::MutexGuard<'_, State>> {
        self.state.lock().map_err(|_| unavailable())
    }

    /// Read the Google calendars that are due, at most once a minute each.
    /// Runs on its own worker so a slow network never delays local chips.
    pub fn refresh_google(&self) -> AppResult<GoogleRefresh> {
        let store = self.context.read_shared()?;
        let mut refresh = GoogleRefresh::default();
        if !store.settings.meeting_reminders {
            self.lock()?.google.clear();
            return Ok(refresh);
        }
        let accounts = match self.calendar.google_accounts(&store) {
            Ok(accounts) => {
                let mut state = self.lock()?;
                state.errors.remove("google");
                let cached = state.google.len();
                state.google.retain(|account, _| accounts.contains(account));
                refresh.updated = state.google.len() != cached;
                accounts
            }
            Err(error) => {
                // Keep the cached events; the account list is read again next time.
                note(&mut *self.lock()?, &mut refresh.notices, "google".into(), error.message);
                return Ok(refresh);
            }
        };
        let lead = reminders::effective_lead(store.settings.reminder_lead_minutes);
        for account in accounts {
            let now = self.context.clock.now();
            let due = !self.lock()?.google.get(&account).is_some_and(|r| {
                r.lead == lead && now >= r.read_at && now - r.read_at < GOOGLE_REFRESH_SECS
            });
            if !due {
                continue;
            }
            let (from, to) = reminders::read_range(now, lead);
            let result = self.calendar.google(&store, account, from, to);
            let source = format!("google:{account}");
            let mut state = self.lock()?;
            match result {
                Ok(items) => {
                    state.errors.remove(&source);
                    state.google.insert(account, GoogleRead { read_at: now, lead, items });
                    refresh.updated = true;
                }
                Err(error) => {
                    if !matches!(error.code, "sign_in_required" | "calendar_consent_required") {
                        note(&mut state, &mut refresh.notices, source, error.message);
                    }
                    // Keep the last events and wait a minute before retrying.
                    let read = state
                        .google
                        .entry(account)
                        .or_insert(GoogleRead { read_at: now, lead, items: Vec::new() });
                    read.read_at = now;
                    read.lead = lead;
                }
            }
        }
        Ok(refresh)
    }

    /// Recompute the chips from the local calendar and the last Google reads.
    /// Never touches the network and never writes.
    pub fn tick(&self) -> AppResult<ReminderTick> {
        let _ticking = self.ticking.lock().map_err(|_| unavailable())?;
        let store = self.context.read_shared()?;
        let settings = &store.settings;
        let regional = self.context.clock.regional(settings);
        let lead = reminders::effective_lead(settings.reminder_lead_minutes);
        let now = self.context.clock.now();
        let (from, to) = reminders::read_range(now, lead);
        let local = settings.meeting_reminders.then(|| self.calendar.local(&store, from, to));
        let mut notices = Vec::new();
        let mut state = self.lock()?;
        // Read under the lock, so a recording a chip has just started is never
        // overwritten by an older reading.
        let recording = self.recording();
        let mut candidates = Vec::new();
        match local {
            Some(Ok(items)) => {
                state.errors.remove("local");
                candidates.extend(items);
            }
            Some(Err(error)) => note(&mut state, &mut notices, "local".into(), error.message),
            None => {}
        }
        if settings.meeting_reminders {
            for read in state.google.values() {
                candidates.extend(read.items.iter().cloned());
            }
        }
        let view = ReminderView {
            chips: reminders::select(&candidates, now, lead, &state.handled),
            recording,
            dark: settings.dark,
            locale: regional.locale,
            time_zone: regional.time_zone,
        };
        state.candidates = candidates;
        state.lead = lead;
        let changed = view != state.view;
        state.view = view.clone();
        Ok(ReminderTick { view, changed, notices })
    }

    pub fn view(&self) -> AppResult<ReminderView> {
        Ok(self.lock()?.view.clone())
    }

    /// A visible chip, with the link and event the engine chose for it. Its
    /// link passed `reminders::valid_link` when the chip was selected.
    pub fn chip(&self, key: &str) -> AppResult<ReminderChip> {
        self.lock()?
            .view
            .chips
            .iter()
            .find(|c| c.key == key)
            .cloned()
            .ok_or_else(|| AppError::new("reminder_missing", "This meeting reminder is no longer available"))
    }

    /// Start recording `chip`, taken before the link opened, so closing the chip
    /// meanwhile cannot cancel the recording.
    pub fn record_chip(&self, chip: &ReminderChip) -> AppResult<()> {
        self.record_meeting(&chip.key, chip.title.clone(), chip.local_event_id)
    }

    /// Start recording for a meeting identified by `key`; a repeated call with the
    /// same key in this session starts one recording.
    pub fn record_meeting(&self, key: &str, title: String, event_id: Option<u64>) -> AppResult<()> {
        // The devices chosen in Settings count here too, whichever way the recording starts.
        let settings = self.context.read_shared()?.settings.clone();
        let (microphone, system) = reminders::recording_devices_with(
            &self.meetings.devices()?,
            &settings.meeting_microphone,
            &settings.meeting_system_audio,
        );
        if microphone.is_none() && system.is_none() {
            return Err(AppError::validation("No audio device is available"));
        }
        self.meetings.start(StartMeeting {
            request_id: reminders::start_request_id(&format!("{}:{}", self.session, key)),
            title,
            event_id,
            microphone,
            system,
        })?;
        self.lock()?.view.recording = true;
        Ok(())
    }

    /// Hide the chip for the rest of this session; a meeting waiting for room
    /// takes its place at once.
    pub fn dismiss(&self, key: &str) -> AppResult<ReminderView> {
        let mut state = self.lock()?;
        state.handled.insert(key.to_string());
        let now = self.context.clock.now();
        state.view.chips = reminders::select(&state.candidates, now, state.lead, &state.handled);
        Ok(state.view.clone())
    }

    fn recording(&self) -> bool {
        self.meetings.capture_state().is_ok_and(|state| {
            matches!(
                state,
                CaptureState::Preparing | CaptureState::Recording | CaptureState::Paused | CaptureState::Saving
            )
        })
    }
}

fn unavailable() -> AppError {
    AppError::external("Meeting reminders are unavailable")
}

/// Each distinct error reaches the main window once, however many sources
/// (accounts, the local calendar) report it.
fn note(state: &mut State, notices: &mut Vec<String>, source: String, message: String) {
    if state.errors.get(&source) == Some(&message) {
        return;
    }
    let known = state.errors.values().any(|m| *m == message) || notices.contains(&message);
    state.errors.insert(source, message.clone());
    if !known {
        notices.push(message);
    }
}
