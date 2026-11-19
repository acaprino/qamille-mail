use super::{
    AppError, AppResult, ServiceContext,
    ports::{AssistantGateway, SearchGateway, SearchHit},
};
use crate::domain::rules;
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub struct Answer {
    pub text: String,
    pub sources: Vec<SearchHit>,
    pub meeting_sources: Vec<crate::domain::meeting::MeetingHit>,
}
#[derive(Clone)]
pub struct AssistantService {
    context: Arc<ServiceContext>,
    assistant: Arc<dyn AssistantGateway>,
    search: Arc<dyn SearchGateway>,
    meetings: Option<super::meetings::MeetingService>,
}
impl AssistantService {
    pub fn new(
        context: Arc<ServiceContext>,
        assistant: Arc<dyn AssistantGateway>,
        search: Arc<dyn SearchGateway>,
    ) -> Self {
        Self {
            context,
            assistant,
            search,
            meetings: None,
        }
    }
    pub fn with_meetings(mut self, meetings: super::meetings::MeetingService) -> Self {
        self.meetings = Some(meetings);
        self
    }
    pub fn summarize(&self, id: u64) -> AppResult<String> {
        let store = self.context.read_shared()?;
        let email = super::mail::MailService::new(self.context.clone()).read(id)?;
        self.assistant.summarize(
            &self.context.settings(&store)?,
            "Short summary",
            &email.body,
        )
    }
    pub fn reply(&self, id: u64, instruction: &str) -> AppResult<String> {
        let store = self.context.read_shared()?;
        self.assistant.reply(
            &self.context.settings(&store)?,
            &super::mail::MailService::new(self.context.clone()).read(id)?,
            instruction,
        )
    }
    pub fn ask(&self, question: &str) -> AppResult<Answer> {
        if question.trim().is_empty() {
            return Err(AppError::validation("Enter a question"));
        }
        let store = self.context.read_shared()?;
        let mut sources = self.search.search(&store, question, 8)?;
        let mut context = String::new();
        let reader = super::mail::MailService::new(self.context.clone());
        for hit in &mut sources {
            if let Some(e) = store.email(hit.email_id) {
                // Never turn an embedding or old stored excerpt into a quotation.
                // Retrieve the current source when it is no longer on this device.
                let source = if e.remote_only {
                    reader.read(e.id)?
                } else {
                    e.clone()
                };
                if hit.passage.is_empty() || e.remote_only {
                    hit.passage = source.snippet(1800);
                }
                context.push_str(&format!(
                    "Message [{}] {} from {}: {}\n",
                    e.id, e.subject, e.from_addr, hit.passage
                ));
            }
        }
        for e in store
            .events
            .iter()
            .filter(|e| e.end >= self.context.clock.now())
        {
            context.push_str(&format!(
                "Event {} on {}: {}\n",
                e.title,
                rules::fmt_full(e.start),
                e.notes
            ));
        }
        let mut meeting_sources = vec![];
        if let Some(meetings) = &self.meetings {
            let stop_words = ["the", "and", "what", "about", "meeting", "meetings", "quale", "quali", "della", "delle", "degli", "abbiamo", "sono", "cosa", "come", "per", "con", "che", "una", "del", "nel"];
            let mut seen = std::collections::HashSet::new();
            for term in question.split(|c: char| !c.is_alphanumeric()).map(str::to_lowercase)
                .filter(|t| t.chars().count() >= 3 && !stop_words.contains(&t.as_str())).take(12) {
                for hit in meetings.search(&term)? {
                    let key = (hit.meeting_id, hit.kind.clone(), hit.version.clone(), hit.segment_id.clone(), hit.text.clone());
                    if meeting_sources.len() < 8 && seen.insert(key) { meeting_sources.push(hit); }
                }
            }
        }
        for hit in &meeting_sources {
            context.push_str(&format!("Meeting [{}] {} — fonte {} / versione {} / posizione audio {} ms: {}\n", hit.meeting_id, hit.title, hit.segment_id.as_deref().unwrap_or(&hit.kind), hit.version.as_deref().unwrap_or("legacy"), hit.at_ms.map(|t| t.to_string()).unwrap_or_else(|| "non disponibile".into()), hit.text));
        }
        Ok(Answer {
            text: self
                .assistant
                .ask(&self.context.settings(&store)?, question, &context)?,
            sources,
            meeting_sources,
        })
    }
    pub fn search(&self, query: &str) -> AppResult<Vec<SearchHit>> {
        self.search.search(&self.context.read()?, query, 30)
    }
    pub fn index(&self) -> AppResult<String> {
        const MAX_CONTENT: usize = 64 * 1024 * 1024;
        const MAX_BODY: usize = 2 * 1024 * 1024;
        if !self.search.installed() {
            return Err(AppError::validation(
                "Install the search model before indexing mail",
            ));
        }
        let store = self.context.read_shared()?;
        let since = self.context.clock.now().saturating_sub(90 * 86400);
        let mut candidates: Vec<_> = store
            .emails
            .iter()
            .filter(|email| {
                !email.storage_evicted
                    && !matches!(email.folder.as_str(), "Spam" | "Trash")
                    && !email.date_missing
                    && email.date >= since
            })
            .collect();
        candidates.sort_by_key(|email| std::cmp::Reverse((email.date, email.id)));
        candidates.truncate(10_000);
        let mut loaded = Vec::new();
        let mut used = 0usize;
        let mut skipped = 0usize;
        let mut accounts = std::collections::HashMap::new();
        for email in candidates {
            if used >= MAX_CONTENT {
                skipped += 1;
                continue;
            }
            let content = if email.remote_only {
                if !accounts.contains_key(&email.account_id) {
                    accounts.insert(
                        email.account_id,
                        self.context.account(&store, email.account_id)?,
                    );
                }
                match self
                    .context
                    .mail
                    .load_body(&accounts[&email.account_id], email)
                {
                    Ok(mut value) => {
                        value.id = email.id;
                        value.account_id = email.account_id;
                        value.folder = email.folder.clone();
                        value.remote_only = false;
                        value
                    }
                    Err(_) => {
                        skipped += 1;
                        continue;
                    }
                }
            } else {
                email.clone()
            };
            // Automatic semantic indexing covers bodies only. Attachments require
            // their own explicit extraction workflow and memory budget.
            let mut content = content;
            for attachment in &mut content.attachments {
                attachment.path = None;
                attachment.inline_text = None;
            }
            if content.body.len() > MAX_BODY
                || used.saturating_add(content.body.len()) > MAX_CONTENT
            {
                skipped += 1;
                continue;
            }
            used += content.body.len();
            loaded.push(content);
        }
        let result = self.search.index_with_content(&store, &loaded)?;
        if skipped == 0 {
            Ok(result)
        } else {
            Ok(format!(
                "{result}. {skipped} messages were not indexed because content was unavailable or exceeded the temporary download limit."
            ))
        }
    }
    pub fn installed(&self) -> bool {
        self.search.installed()
    }
    pub fn install_status(&self) -> super::ports::ModelInstallStatus {
        self.search.install_status()
    }
}
