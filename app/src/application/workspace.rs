use super::{
    AppError, AppResult, ServiceContext,
    ports::{AI_KEY, GOOGLE_SECRET},
};
use crate::domain::model::{Account, Draft, Event, Meeting, Settings, Signature};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, sync::Arc};

pub type RegionalSnapshot = super::ports::RegionalSettings;

#[derive(Serialize)]
pub struct WorkspaceSnapshot {
    pub regional: RegionalSnapshot,
    pub undo_actions: Vec<crate::domain::undo::UndoAction>,
    pub accounts: Vec<Account>,
    pub counts: BTreeMap<String, usize>,
    pub labels: Vec<String>,
    pub settings: Settings,
    pub has_ai_key: bool,
    pub has_google_secret: bool,
    pub signatures: Vec<Signature>,
    pub drafts: Vec<Draft>,
}
#[derive(Deserialize)]
pub struct SettingsUpdate {
    pub settings: Settings,
    pub ai_key: Option<String>,
    pub google_client_secret: Option<String>,
}

/// A message the Inbox counter counts: in the Inbox, unread, not done and not snoozed now. The one rule
/// of the sidebar's counter and of the taskbar's (`snapshot`, `inbox_unread`).
fn counts_as_inbox_unread(e: &crate::domain::model::Email, now: i64) -> bool {
    e.folder == "Inbox" && !e.read && !e.done && !e.is_snoozed(now)
}

#[derive(Clone)]
pub struct WorkspaceService {
    context: Arc<ServiceContext>,
}
impl WorkspaceService {
    pub fn new(context: Arc<ServiceContext>) -> Self {
        Self { context }
    }
    pub fn snapshot(&self) -> AppResult<WorkspaceSnapshot> {
        let store = crate::domain::undo::project_shared(self.context.read_shared()?);
        let now = self.context.clock.now();
        // One pass over the mail for every folder count.
        let mut counts: BTreeMap<String, usize> = ["Inbox", "Tasks", "Snoozed", "Sent", "Archive", "Trash"]
            .into_iter()
            .map(|folder| (folder.to_string(), 0))
            .collect();
        let mut add = |folder: &str| *counts.get_mut(folder).expect("known folder") += 1;
        for e in &store.emails {
            let hidden = matches!(e.folder.as_str(), "Trash" | "Spam");
            match e.folder.as_str() {
                "Inbox" if counts_as_inbox_unread(e, now) => add("Inbox"),
                "Sent" => add("Sent"),
                "Archive" => add("Archive"),
                "Trash" => add("Trash"),
                _ => {}
            }
            if e.is_task() && !e.done && !hidden {
                add("Tasks");
            }
            if e.is_snoozed(now) && !hidden {
                add("Snoozed");
            }
        }
        counts.insert("Drafts".into(), store.drafts.len());
        // Credentials that do not open here (a data folder from another Windows user or computer)
        // must not hide the mail and settings: those keys are simply not usable.
        let has_ai_key = self.context.secrets.get(AI_KEY).is_ok_and(|key| !key.is_empty());
        let has_google_secret = self.context.secrets.get(GOOGLE_SECRET).is_ok_and(|secret| !secret.is_empty());
        Ok(WorkspaceSnapshot {
            regional: self.context.clock.regional(&store.settings),
            labels: store.all_labels(),
            undo_actions: store.undo_actions.clone(),
            accounts: store.accounts.clone(),
            counts,
            settings: store.settings.clone(),
            has_ai_key,
            has_google_secret,
            signatures: store.signatures.clone(),
            drafts: store.drafts.clone(),
        })
    }
    /// The unread mail of the Inbox, the number of `counts["Inbox"]` of `snapshot` (the sidebar's): the
    /// taskbar overlay and the tray tooltip show it (desktop `taskbar.rs`). Same store view, same rule.
    pub fn inbox_unread(&self) -> AppResult<usize> {
        let store = crate::domain::undo::project_shared(self.context.read_shared()?);
        let now = self.context.clock.now();
        Ok(store.emails.iter().filter(|e| counts_as_inbox_unread(e, now)).count())
    }
    /// The accounts whose indexing is under way (`IndexProgress::in_progress`): the taskbar's progress
    /// bar follows only these, so an account that failed, was switched off or removed below 100 %
    /// takes its bar away.
    pub fn indexing_accounts(&self) -> AppResult<Vec<u32>> {
        let store = self.context.read_shared()?;
        Ok(store.accounts.iter().filter(|a| a.search_index.in_progress()).map(|a| a.id).collect())
    }
    pub fn save_settings(&self, mut update: SettingsUpdate) -> AppResult<()> {
        if !update.settings.time_zone.is_empty() && update.settings.time_zone.parse::<chrono_tz::Tz>().is_err() {
            return Err(AppError::validation("Choose a recognized IANA time zone"));
        }
        if !update.settings.locale.is_empty() && !crate::domain::regional::valid_locale(&update.settings.locale) {
            return Err(AppError::validation("Choose a valid regional format"));
        }
        if !(5..=30).contains(&update.settings.undo_seconds) {
            return Err(AppError::validation(
                "Undo time must be between 5 and 30 seconds",
            ));
        }
        if !crate::domain::reminders::valid_lead(update.settings.reminder_lead_minutes) {
            return Err(AppError::validation(
                "Choose a meeting reminder time of 1, 2, 5 or 10 minutes",
            ));
        }
        {
            use crate::domain::meeting::{
                canonical_engine, valid_remote_model_name, valid_transcription_language, ENGINE_COMPUTE,
                ENGINE_EXTERNAL, LOCAL_SPEECH_MODELS,
            };
            // A window opened before the update may still send the old names.
            update.settings.meeting_engine = canonical_engine(&update.settings.meeting_engine).to_owned();
            let settings = &update.settings;
            if settings.meeting_engine != ENGINE_COMPUTE && settings.meeting_engine != ENGINE_EXTERNAL {
                return Err(AppError::validation(
                    "Scegli se trascrivere con il server di calcolo o con un servizio esterno compatibile OpenAI.",
                ));
            }
            if !valid_transcription_language(&settings.meeting_language) {
                return Err(AppError::validation("Lingua di trascrizione non valida."));
            }
            if !LOCAL_SPEECH_MODELS.contains(&settings.meeting_model.as_str()) {
                return Err(AppError::validation("Modello di trascrizione non supportato."));
            }
            if !valid_remote_model_name(&settings.meeting_remote_model) {
                return Err(AppError::validation("Nome del modello remoto non valido."));
            }
        }
        if update.settings.user_name.trim().is_empty() {
            return Err(AppError::validation("Enter your name"));
        }
        if let Some(key) = update.ai_key {
            self.context.secrets.set(AI_KEY, &key)?;
        }
        if let Some(secret) = update.google_client_secret {
            self.context.secrets.set(GOOGLE_SECRET, &secret)?;
        }
        self.context.update(|store| {
            // The link to the server is changed only by its own commands: a settings page
            // opened before a sign-in must not take it back.
            let mut settings = update.settings.clone();
            settings.compute_mode = store.settings.compute_mode;
            settings.compute_consented_urls = store.settings.compute_consented_urls.clone();
            settings.onboarding_done = store.settings.onboarding_done;
            settings.compute_ca_pem = store.settings.compute_ca_pem.clone();
            settings.compute_ca_url = store.settings.compute_ca_url.clone();
            settings.llm_consent = store.settings.llm_consent.clone();
            settings.mailbox_sync = store.settings.mailbox_sync;
            settings.mailbox_sync_consent = store.settings.mailbox_sync_consent.clone();
            store.settings = settings;
            Ok(())
        })?;
        self.context.cleanup_mail_cache()
    }
    pub fn events(&self) -> AppResult<Vec<Event>> {
        let mut events = self.context.read_shared()?.events.clone();
        events.sort_by_key(|e| e.start);
        Ok(events)
    }
    pub fn save_event(&self, mut event: Event) -> AppResult<Event> {
        if event.title.trim().is_empty() || event.end <= event.start {
            return Err(AppError::validation(
                "Enter a title and an end time after the start",
            ));
        }
        if let Some(url) = &event.meeting_link {
            if !url.starts_with("https://") {
                return Err(AppError::validation(
                    "Meeting links must start with https://",
                ));
            }
        }
        self.context.update(|store| {
            if event.id == 0 {
                event.id = store.new_id();
                store.events.push(event.clone());
            } else {
                *store
                    .events
                    .iter_mut()
                    .find(|e| e.id == event.id)
                    .ok_or_else(|| AppError::missing("Event"))? = event.clone();
            }
            Ok(event)
        })
    }
    pub fn delete_event(&self, id: u64) -> AppResult<()> {
        self.context.update(|s| {
            s.events.retain(|e| e.id != id);
            Ok(())
        })
    }
    pub fn meetings(&self) -> AppResult<Vec<Meeting>> {
        Ok(self.context.read_shared()?.meetings.clone())
    }
    pub fn save_meeting_notes(&self, id: u64, notes: String) -> AppResult<()> {
        self.context.update(|s| {
            s.meetings
                .iter_mut()
                .find(|m| m.id == id)
                .ok_or_else(|| AppError::missing("Meeting"))?
                .notes = notes;
            Ok(())
        })
    }
    pub fn save_signature(&self, mut signature: Signature) -> AppResult<Signature> {
        if signature.name.trim().is_empty() {
            return Err(AppError::validation("Enter a signature name"));
        }
        self.context.update(|store| {
            signature.modified = self.context.clock.now();
            if signature.id == 0 {
                signature.id = store.new_id();
                store.signatures.push(signature.clone());
            } else {
                *store
                    .signatures
                    .iter_mut()
                    .find(|s| s.id == signature.id)
                    .ok_or_else(|| AppError::missing("Signature"))? = signature.clone();
            }
            Ok(signature)
        })
    }
    pub fn delete_signature(&self, id: u64) -> AppResult<()> {
        self.context.update(|s| {
            s.remove_signature(id);
            Ok(())
        })
    }
}
