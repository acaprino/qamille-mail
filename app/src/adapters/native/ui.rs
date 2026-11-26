//! The window: sidebar, Smart Inbox, tasks, reading pane with +AI, composer, calendar, meetings,
//! assistant and settings. Slow work runs on background threads and reports back through `Msg`.

use crate::ai;
use crate::attach;
use crate::composer_tabs::{self, Tab, TabState, Tabs};
use crate::mail;
use crate::mcp;
use crate::oauth;
use crate::model::{Access, Account, AccountKind, AiProvider, BodyFormat, Category, Draft, Email, Event, Meeting, Rsvp, Signature, SignatureBinding, Store, now_ts};
use crate::richdoc::{self, Attr, LineKind, RichDoc};
use crate::richtext;
use crate::rag::{self, Hit, Index, IndexReport, Progress, RagStatus};
use crate::recorder::Recorder;
use crate::rules::{self, Preset, ScheduleKind, Section};
use crate::secret;
use chrono::{Datelike, Duration, Local, NaiveDate, TimeZone, Timelike};
use eframe::egui::{self, Align, Align2, Color32, CornerRadius, FontId, Layout, Margin, RichText, Sense, Stroke, TextEdit, Vec2, pos2, vec2};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};

const AUTO_SYNC_SECS: i64 = 300; // estimated
const SCREENSHOT_FRAME: u64 = 120;
const ACCOUNT_COLORS: [[u8; 3]; 5] = [[47, 111, 237], [236, 112, 99], [46, 160, 110], [124, 92, 230], [224, 150, 40]];

#[derive(Clone, PartialEq)]
enum View {
    Inbox,
    Tasks,
    Snoozed,
    Drafts,
    Sent,
    Archive,
    Trash,
    Label(String),
    Calendar,
    Meetings,
    Assistant,
    Settings,
}

enum Msg {
    Note(String),
    LoginChecked { account: Account, is_new: bool, result: Result<(), String> },
    Synced { account: u32, result: Result<Vec<Email>, String>, folders: Option<(Option<String>, Option<String>)> },
    Reply { email: u64, text: String },
    Classified { email: u64, category: Category, labels: Vec<String> },
    AiOutput { email: u64, title: String, text: String },
    Writing { draft: u64, text: String },
    Sent { draft: u64, result: Result<(), String> },
    ModelChecked(bool),
    IndexDone(Result<(Index, IndexReport), String>),
    Search { query: String, hits: Vec<Hit>, note: Option<String> },
    Answer { question: String, answer: String, sources: Vec<u64> },
    Transcribed { meeting: u64, result: Result<String, String> },
    MeetingNotes { meeting: u64, summary: String, key_points: Vec<String>, action_items: Vec<String> },
    AttachmentText { name: String, result: Result<String, String> },
    Summary(String),
}

struct Palette {
    bg: Color32,
    side: Color32,
    text: Color32,
    muted: Color32,
    border: Color32,
    selected: Color32,
    hover: Color32,
    accent: Color32,
    danger: Color32,
}

// Colours from the original's renderer/app-colors.css. The sidebar is dark in both themes there.
const SIDE_BG: Color32 = Color32::from_rgb(0x28, 0x2a, 0x2e); // --sidebarBackground
const SIDE_TEXT: Color32 = Color32::from_rgb(0xae, 0xb8, 0xc5); // --sidebarText
const SIDE_ACTIVE: Color32 = Color32::from_rgb(0x3d, 0x3f, 0x43); // --sidebarExpandedHover over the background
const SIDE_HOVER: Color32 = Color32::from_rgb(0x2f, 0x32, 0x37); // --sidebarGray3

fn palette(dark: bool) -> Palette {
    if dark {
        Palette {
            bg: Color32::from_rgb(0x28, 0x2a, 0x2e),       // --elevationsElevation1
            side: Color32::from_rgb(0x2f, 0x32, 0x37),     // --gray1
            text: Color32::from_rgb(0xde, 0xe3, 0xeb),     // --mainText
            muted: Color32::from_rgb(0x86, 0x8d, 0x97),    // --textSecondary
            border: Color32::from_rgb(0x45, 0x45, 0x45),   // --calendarBorder
            selected: Color32::from_rgb(0x39, 0x3e, 0x47), // --paywallLightButtonBackground
            hover: Color32::from_rgb(0x2f, 0x32, 0x37),    // --gray1
            accent: Color32::from_rgb(0x46, 0xa2, 0xff),   // --accentBlue
            danger: Color32::from_rgb(0xe0, 0x42, 0x42),   // --red4
        }
    } else {
        Palette {
            bg: Color32::WHITE,
            side: Color32::from_rgb(0xf5, 0xf6, 0xf8),     // --gray0
            text: Color32::from_rgb(0x14, 0x14, 0x14),     // --mainText
            muted: Color32::from_rgb(0x66, 0x70, 0x7e),    // --textSecondary
            border: Color32::from_rgb(0xeb, 0xec, 0xed),   // --calendarBorder
            selected: Color32::from_rgb(0xea, 0xf4, 0xff), // --paywallLightButtonBackground
            hover: Color32::from_rgb(0xf5, 0xf6, 0xf8),    // --gray0
            accent: Color32::from_rgb(0x15, 0x7e, 0xfb),   // --accentBlue
            danger: Color32::from_rgb(0xe4, 0x35, 0x35),   // --red4
        }
    }
}

#[derive(Default)]
struct NewAccount {
    name: String,
    email: String,
    password: String,
}

struct EventForm {
    id: Option<u64>,
    title: String,
    date: String,
    time: String,
    minutes: String,
    location: String,
    attendees: String,
    link: String,
}

pub struct App {
    store: Store,
    last_mtime: Option<std::time::SystemTime>,
    dirty: bool,
    view: View,
    account_filter: Option<u32>,
    selected: Option<u64>,
    search: String,
    last_valid_search: String,
    hits: Option<(String, Vec<Hit>)>,
    search_note: Option<String>,
    searching: bool,
    collapsed: HashSet<&'static str>,
    shown_extra: std::collections::HashMap<&'static str, usize>,
    tabs: Tabs,
    locked_section: Option<(u64, Section)>,
    area_top: f32,
    footer_top: f32,
    last_autosave: i64,
    signature_edit: Option<Signature>,
    signature_sel: (usize, usize),
    ai_busy: bool,
    ai_output: Option<(u64, String, String)>,
    reply_hint: String,
    custom_when: String,
    new_label: String,
    attachment_view: Option<(String, Result<String, String>)>,
    attachment_summary: String,
    status: String,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    ctx: egui::Context,
    syncing: HashSet<u32>,
    sending: HashSet<u64>,
    last_tick: i64,
    last_sync: i64,
    // Semantic search
    index: Arc<Index>,
    rag_status: Progress,
    rag_stop: Arc<AtomicBool>,
    /// The compute server (spec D3) and whether it has the search model, read off the UI thread.
    compute: Arc<dyn crate::application::compute::ComputeGateway>,
    model_ready: bool,
    // Accounts form
    new_account: NewAccount,
    checking_login: bool,
    oauth_stop: Arc<AtomicBool>,
    client_secret_input: String,
    login_result: Option<Result<String, String>>,
    ai_key_input: String,
    attachment_imports: Vec<(u64, String)>,
    // Calendar
    week_offset: i64,
    selected_event: Option<u64>,
    event_form: Option<EventForm>,
    // Meetings
    selected_meeting: Option<u64>,
    meeting_tab: u8,
    recorder: Option<Recorder>,
    recording_title: String,
    recording_event: Option<u64>,
    // Assistant
    question: String,
    chat: Vec<(String, String, Vec<u64>)>,
    asking: bool,
    // Screenshot mode
    screenshot: Option<PathBuf>,
    frame: u64,
    screenshot_sent: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, screenshot: Option<PathBuf>) -> App {
        // Screenshots always show sample mail, so real mail never ends up in a picture.
        let store = if screenshot.is_some() { crate::model::sample_store() } else { Store::load_or_default() };
        let (tx, rx) = channel();
        let mut app = App {
            selected: None,
            last_mtime: Store::mtime(),
            dirty: false,
            view: View::Inbox,
            account_filter: None,
            search: String::new(),
            last_valid_search: String::new(),
            hits: None,
            search_note: None,
            searching: false,
            collapsed: HashSet::new(),
            shown_extra: Default::default(),
            tabs: Tabs::default(),
            locked_section: None,
            area_top: 44.0,
            footer_top: f32::INFINITY,
            last_autosave: now_ts(),
            signature_edit: None,
            signature_sel: (0, 0),
            ai_busy: false,
            ai_output: None,
            reply_hint: String::new(),
            custom_when: String::new(),
            new_label: String::new(),
            attachment_view: None,
            attachment_summary: String::new(),
            status: String::new(),
            tx,
            rx,
            ctx: cc.egui_ctx.clone(),
            syncing: HashSet::new(),
            sending: HashSet::new(),
            last_tick: 0,
            last_sync: now_ts(),
            index: Arc::new(if screenshot.is_some() { Index::default() } else { Index::load() }),
            rag_status: Arc::new(Mutex::new(RagStatus::default())),
            rag_stop: Arc::new(AtomicBool::new(false)),
            compute: crate::infrastructure::compute_factory::routed(),
            model_ready: false,
            new_account: NewAccount::default(),
            checking_login: false,
            oauth_stop: Arc::new(AtomicBool::new(false)),
            client_secret_input: String::new(),
            login_result: None,
            ai_key_input: String::new(),
            attachment_imports: Vec::new(),
            week_offset: 0,
            selected_event: None,
            event_form: None,
            selected_meeting: None,
            meeting_tab: 0,
            recorder: None,
            recording_title: String::new(),
            recording_event: None,
            question: String::new(),
            chat: Vec::new(),
            asking: false,
            screenshot,
            frame: 0,
            screenshot_sent: false,
            store,
        };
        if app.screenshot.is_some() {
            app.view = match std::env::var("QAMILLE_VIEW").unwrap_or_default().as_str() {
                "tasks" => View::Tasks,
                "calendar" => View::Calendar,
                "meetings" => View::Meetings,
                "assistant" => View::Assistant,
                "settings" => View::Settings,
                "drafts" => View::Drafts,
                _ => View::Inbox,
            };
            app.selected_event = app.store.events.first().map(|e| e.id);
            app.selected_meeting = app.store.meetings.first().map(|m| m.id);
            if let Ok(q) = std::env::var("QAMILLE_SEARCH") {
                app.search = q;
            }
            if let Ok(mode) = std::env::var("QAMILLE_COMPOSE") {
                let ids = app.mail_ids();
                for id in ids.iter().skip(1).take(2) {
                    if let Some(d) = app.reply_draft(*id, false) {
                        app.open_composer(d);
                    }
                }
                let mut d = app.reply_draft(ids.first().copied().unwrap_or(0), false).unwrap_or_default();
                d.body = "<h1>Hello</h1><div><strong>Bold</strong> and <em>italic</em> with a <span style=\"color:#157efb\">blue word</span>, <u>underlined</u>, <span style=\"background-color:#fff3a3\">highlighted</span> and a <a href=\"https://example.com\">link</a></div><ul><li>First point</li><li>Second point</li></ul><ol><li>One</li><li>Two</li></ol>".into();
                app.open_composer(d);
                let last = app.tabs.items.len() - 1;
                app.tabs.items[last].source = mode == "source";
                match mode.as_str() {
                    "full" => app.tabs.toggle_full(last),
                    "collapsed" => app.tabs.toggle(last),
                    _ => {}
                }
            }
            if std::env::var("QAMILLE_SIGNATURE").is_ok() {
                app.signature_edit = app.store.signatures.first().map(|x| Signature { body: x.html_body(), html: true, ..x.clone() });
            }
        }
        app.selected = app.mail_ids().first().copied();
        if app.view == View::Inbox && app.search.is_empty() {
            app.selected = app.mail_ids().first().copied().or(app.selected);
        }
        load_bold_font(&cc.egui_ctx);
        app.apply_theme();
        if app.screenshot.is_none() {
            let compute = app.compute.clone();
            app.spawn(move || Msg::ModelChecked(rag::model_installed(&*compute)));
            app.sync_all();
        }
        app
    }

    fn pal(&self) -> Palette {
        palette(self.store.settings.dark)
    }

    fn apply_theme(&self) {
        let p = self.pal();
        let mut v = if self.store.settings.dark { egui::Visuals::dark() } else { egui::Visuals::light() };
        v.panel_fill = p.bg;
        v.window_fill = p.bg;
        v.extreme_bg_color = if self.store.settings.dark { p.side } else { Color32::from_rgb(241, 243, 245) };
        v.override_text_color = Some(p.text);
        v.weak_text_color = Some(p.muted.gamma_multiply(0.8));
        v.selection.bg_fill = p.selected;
        v.selection.stroke = Stroke::new(1.0, p.accent);
        v.hyperlink_color = p.accent;
        v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
        for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.open] {
            w.corner_radius = CornerRadius::same(6);
        }
        v.widgets.inactive.weak_bg_fill = p.hover;
        v.widgets.inactive.bg_fill = p.hover;
        // Set the theme explicitly, or following the system theme would replace these colours.
        let theme = if self.store.settings.dark { egui::Theme::Dark } else { egui::Theme::Light };
        self.ctx.set_theme(theme);
        self.ctx.set_visuals_of(theme, v);
        self.ctx.all_styles_mut(|style| {
            style.spacing.item_spacing = vec2(8.0, 6.0);
            style.spacing.button_padding = vec2(10.0, 5.0);
            // A quiet scrollbar: the default one draws in the text colour.
            style.spacing.scroll.foreground_color = false;
        });
    }

    fn spawn(&self, f: impl FnOnce() -> Msg + Send + 'static) {
        let tx = self.tx.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(f());
            ctx.request_repaint();
        });
    }

    // ---------- Mail lists ----------

    fn matches_account(&self, e: &Email) -> bool {
        self.account_filter.is_none_or(|a| e.account_id == a)
    }

    fn in_view(&self, e: &Email, now: i64) -> bool {
        if !self.matches_account(e) {
            return false;
        }
        match &self.view {
            View::Inbox => e.folder == "Inbox" && !e.done && !e.is_snoozed(now) && !e.set_aside,
            View::Tasks => e.is_task() && !e.is_snoozed(now),
            View::Snoozed => e.is_snoozed(now),
            View::Sent => e.folder == "Sent",
            View::Archive => e.folder == "Archive" || (e.done && e.folder == "Inbox"),
            View::Trash => e.folder == "Trash",
            View::Label(l) => e.labels.iter().any(|x| x == l) && e.folder != "Trash",
            _ => false,
        }
    }

    /// Mail shown in the list, in order. A search overrides the current view.
    fn mail_ids(&self) -> Vec<u64> {
        let current = self.search.trim();
        let q = if crate::infrastructure::search_support::parse(&self.store, current, self.account_filter, now_ts()).is_err() { self.last_valid_search.trim() } else { current };
        if !q.is_empty() {
            if let Some((hq, hits)) = &self.hits {
                if hq == q {
                    return hits.iter().map(|h| h.email_id).filter(|id| self.store.email(*id).is_some_and(|e| self.matches_account(e))).collect();
                }
            }
            return crate::infrastructure::search_support::execute(&self.store, q, self.account_filter, now_ts(), "").unwrap_or_default();
        }
        let now = now_ts();
        let mut v: Vec<&Email> = self.store.emails.iter().filter(|e| self.in_view(e, now)).collect();
        v.sort_by(|a, b| b.inbox_date().cmp(&a.inbox_date()));
        if self.view == View::Inbox {
            v.sort_by_key(|e| Section::ORDER.iter().position(|s| *s == rules::section_with_lock(e, self.locked_section)).unwrap_or(9));
        }
        if self.view == View::Tasks {
            let order = |e: &Email| ["Overdue", "Today", "Upcoming", "No date"].iter().position(|g| *g == task_group(e, now, &self.store.settings)).unwrap_or(9);
            v.sort_by_key(|e| (order(e), e.due.or(e.reminder_at).unwrap_or(i64::MAX)));
        }
        v.iter().map(|e| e.id).collect()
    }

    fn count(&self, view: View) -> usize {
        let now = now_ts();
        self.store
            .emails
            .iter()
            .filter(|e| {
                self.matches_account(e)
                    && match &view {
                        View::Inbox => e.folder == "Inbox" && !e.done && !e.is_snoozed(now) && !e.set_aside && !e.read,
                        View::Tasks => e.is_task() && !e.is_snoozed(now),
                        View::Snoozed => e.is_snoozed(now),
                        View::Label(l) => e.labels.iter().any(|x| x == l) && !e.read && e.folder != "Trash",
                        _ => false,
                    }
            })
            .count()
    }

    // ---------- Actions ----------

    fn act(&mut self, id: u64, action: &str, arg: Option<&str>) {
        let from_trash_or_spam = self.store.email(id).is_some_and(|e| matches!(e.folder.as_str(), "Trash" | "Spam"));
        match crate::infrastructure::timezone::apply_email_action(&mut self.store, id, action, arg) {
            Ok(_) => {
                self.dirty = true;
                self.push_remote(id, action, arg, from_trash_or_spam);
            }
            Err(e) => self.status = e,
        }
    }

    fn push_remote(&mut self, id: u64, action: &str, arg: Option<&str>, from_trash_or_spam: bool) {
        let Some(e) = self.store.email(id).cloned() else { return };
        if e.uid.is_none() && e.message_id.is_empty() { return; }
        let Some(acc) = self.store.account(e.account_id).filter(|a| a.kind == AccountKind::Gmail).cloned() else { return };
        let remote = match action {
            "markAsSeen" => mail::Remote::Seen(true),
            "markAsUnseen" => mail::Remote::Seen(false),
            "pin" => mail::Remote::Flag(true),
            "unpin" => mail::Remote::Flag(false),
            "archive" | "markAsDone" if from_trash_or_spam => mail::Remote::Move(acc.archive_folder.clone()),
            "archive" | "markAsDone" => mail::Remote::Archive,
            "moveToTrash" => mail::Remote::Move(acc.trash_folder.clone()),
            "attachLabel" if self.store.settings.push_labels => mail::Remote::Label(arg.unwrap_or_default().to_string()),
            _ => return,
        };
        if matches!(remote, mail::Remote::Move(_) | mail::Remote::Archive) {
            // The message's number belongs to the Inbox; it no longer applies after moving.
            if let Some(m) = self.store.email_mut(id) {
                m.uid = None;
            }
        }
        self.spawn(move || match mail::push_message_action(&acc, &e, remote) {
            Ok(()) => Msg::Note(String::new()),
            Err(e) => Msg::Note(format!("Gmail didn't accept that change: {e}")),
        });
    }

    fn open_mail(&mut self, id: u64) {
        self.locked_section = self.store.email(id).map(|e| (id, rules::section_of(e)));
        self.selected = Some(id);
        self.ai_output = None;
        self.reply_hint.clear();
        if self.store.email(id).is_some_and(|e| !e.read) {
            self.act(id, "markAsSeen", None);
        }
    }

    fn after_remove(&mut self, id: u64) {
        if self.selected == Some(id) {
            let ids = self.mail_ids();
            self.selected = ids.iter().find(|x| **x != id).copied();
        }
    }

    // ---------- Sync ----------

    fn sync_all(&mut self) {
        self.last_sync = now_ts();
        let accounts: Vec<Account> = self.store.accounts.iter().filter(|a| a.kind == AccountKind::Gmail && !a.password.is_empty()).cloned().collect();
        for a in accounts {
            self.sync_account(a);
        }
    }

    fn sync_account(&mut self, a: Account) {
        if !self.syncing.insert(a.id) {
            return;
        }
        let dir = Store::data_dir().join("attachments");
        let first = self.store.next_id + 1_000_000;
        self.spawn(move || {
            let folders = mail::gmail_folders(&a).ok();
            Msg::Synced { account: a.id, result: mail::sync_account(&a, &dir, first), folders }
        });
    }

    fn classify_new(&mut self, ids: Vec<u64>) {
        if !self.store.settings.auto_classify && !self.store.settings.auto_tag {
            return;
        }
        if ai::online(&self.store.settings) {
            let settings = self.store.settings.clone();
            let known = self.store.all_labels();
            let mails: Vec<Email> = ids.iter().filter_map(|i| self.store.email(*i).cloned()).collect();
            let tx = self.tx.clone();
            let ctx = self.ctx.clone();
            std::thread::spawn(move || {
                for e in mails {
                    let (category, labels) = ai::classify(&settings, &e, &known);
                    let _ = tx.send(Msg::Classified { email: e.id, category, labels });
                    ctx.request_repaint();
                }
            });
        } else {
            for id in ids {
                if let Some(e) = self.store.email(id).cloned() {
                    self.apply_classification(id, rules::classify_offline(&e), rules::labels_offline(&e));
                }
            }
        }
    }

    fn apply_classification(&mut self, id: u64, category: Category, labels: Vec<String>) {
        let (auto_c, auto_t) = (self.store.settings.auto_classify, self.store.settings.auto_tag);
        if let Some(e) = self.store.email_mut(id) {
            if auto_c {
                e.category = category;
            }
            e.classified = true;
        }
        self.dirty = true;
        if auto_t {
            for l in labels {
                self.act(id, "attachLabel", Some(&l));
            }
        }
    }

    // ---------- Semantic search ----------

    fn rag_set(&self, message: &str, busy: bool, fraction: f32) {
        if let Ok(mut s) = self.rag_status.lock() {
            *s = RagStatus { message: message.into(), busy, fraction };
        }
    }

    fn rag_busy(&self) -> bool {
        self.rag_status.lock().map(|s| s.busy).unwrap_or(false)
    }

    fn start_indexing(&mut self) {
        if self.rag_busy() || !self.model_ready {
            return;
        }
        self.rag_stop.store(false, Ordering::Relaxed);
        self.rag_set("Reading your mail for semantic search…", true, 0.0);
        let emails = self.store.emails.clone();
        let (st, stop, ctx, compute) = (self.rag_status.clone(), self.rag_stop.clone(), self.ctx.clone(), self.compute.clone());
        self.spawn(move || {
            let r = rag::update_index(&emails, &stop, &|d, t| {
                if let Ok(mut s) = st.lock() {
                    s.fraction = d as f32 / t.max(1) as f32;
                    s.message = format!("Reading your mail for semantic search: {d} of {t}");
                }
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
            }, &*compute);
            Msg::IndexDone(r)
        });
    }

    fn run_search(&mut self) {
        let q = self.search.trim().to_string();
        if q.is_empty() {
            return;
        }
        if let Err(error) = crate::infrastructure::search_support::parse(&self.store, &q, self.account_filter, now_ts()) {
            self.search_note = Some(error);
            return;
        }
        self.last_valid_search = q.clone();
        self.searching = true;
        let mut store = self.store.clone();
        if let Some(account) = self.account_filter {
            store.emails.retain(|e| e.account_id == account);
            store.accounts.retain(|a| a.id == account);
        }
        let index = self.index.clone();
        let compute = self.compute.clone();
        self.spawn(move || {
            let (hits, note) = rag::hybrid_search(&store, &index, &q, store.emails.len(), &*compute);
            Msg::Search { query: q, hits, note }
        });
    }

    fn ask(&mut self) {
        let q = self.question.trim().to_string();
        if q.is_empty() || self.asking {
            return;
        }
        self.question.clear();
        self.asking = true;
        let store = self.store.clone();
        let index = self.index.clone();
        let compute = self.compute.clone();
        // Screenshot mode contains synthetic meetings only.
        let search_meeting_dir = self.screenshot.is_none().then(|| Store::data_dir().join("meetings"));
        self.spawn(move || {
            const SOURCES: usize = 8; // estimated
            let (hits, _) = rag::hybrid_search(&store, &index, &q, SOURCES, &*compute);
            let mut context = rag::context_from_hits(&store, &hits);
            context.push_str(&crate::application::assistant::calendar_context(
                &store.events, &q, Local::now().timestamp(),
            ));
            if let Some(dir) = search_meeting_dir && crate::application::assistant::include_meeting_sources(&q) {
                let question = crate::application::assistant::normalized_words(&q);
                if let Ok(details) = crate::infrastructure::meeting_repository::merged_details(dir, &store.meetings) {
                    for detail in details.iter().filter(|detail| {
                        detail.included_in_search
                            && crate::application::assistant::title_in_question(&question, &detail.meeting.title)
                    }).take(SOURCES) {
                        let meeting = &detail.meeting;
                        let summary: String = meeting.summary.chars().take(1200).collect();
                        context.push_str(&format!("Meeting {} on {}: {summary}\n", meeting.title, crate::infrastructure::timezone::format_timestamp(&store.settings, meeting.date)));
                    }
                }
            }
            let answer = ai::ask_assistant(&store.settings, &q, &context);
            Msg::Answer { question: q, answer, sources: hits.iter().map(|h| h.email_id).collect() }
        });
    }

    // ---------- Sending ----------

    fn send_draft(&mut self, d: Draft) {
        if mail::split_addrs(&d.to).is_empty() {
            self.status = "Add at least one recipient.".into();
            return;
        }
        let Some(acc) = self.store.account(d.account_id).cloned() else {
            self.status = "Choose an account to send from.".into();
            return;
        };
        let total: u64 = d.attachments.iter().filter_map(|p| std::fs::metadata(p).ok()).map(|m| m.len()).sum();
        if total > rules::ATTACHMENT_CAP_BYTES {
            self.status = "Attachments are over 25 MB, the most Gmail accepts.".into();
            return;
        }
        self.upsert_draft(d.clone());
        self.sending.insert(d.id);
        if acc.kind == AccountKind::Sample {
            let _ = self.tx.send(Msg::Sent { draft: d.id, result: Ok(()) });
            return;
        }
        let reply_id = d.reply_to.and_then(|r| self.store.email(r)).map(|e| e.message_id.clone());
        let name = self.store.settings.user_name.clone();
        let rendered = richtext::render_draft(&self.store, &d);
        self.spawn(move || {
            let out = mail::Outgoing {
                to: mail::split_addrs(&d.to),
                cc: mail::split_addrs(&d.cc),
                bcc: mail::split_addrs(&d.bcc),
                subject: d.subject.clone(),
                body: rendered.plain,
                html: rendered.html,
                in_reply_to: reply_id,
                message_id: None,
                attachments: d.attachments.clone(),
            };
            Msg::Sent { draft: d.id, result: mail::send(&acc, &name, &out) }
        });
    }

    fn open_composer(&mut self, mut d: Draft) {
        self.save_open_tabs();
        // Older formatted drafts become HTML, the one thing the visual editor works on.
        if d.format == BodyFormat::Rich {
            d.body = richtext::markup_to_html(&d.body);
            d.format = BodyFormat::Html;
        }
        self.tabs.open(d);
    }

    /// Keeps what's being written in Drafts, so nothing is lost.
    fn save_open_tabs(&mut self) {
        let drafts: Vec<Draft> = self.tabs.items.iter().filter(|t| !composer_tabs::is_empty_draft(&t.draft)).map(|t| t.draft.clone()).collect();
        for d in drafts {
            self.upsert_draft(d);
        }
    }

    /// Saves the draft (unless nothing was written) and removes its tab.
    fn close_tab(&mut self, at: usize) {
        if at < self.tabs.items.len() {
            let t = self.tabs.items.remove(at);
            if composer_tabs::is_empty_draft(&t.draft) {
                self.store.drafts.retain(|x| x.id != t.draft.id);
                self.dirty = true;
            } else {
                self.upsert_draft(t.draft);
            }
        }
    }

    fn upsert_draft(&mut self, mut d: Draft) {
        d.updated = now_ts();
        if let Some(x) = self.store.drafts.iter_mut().find(|x| x.id == d.id) {
            *x = d;
        } else {
            self.store.drafts.push(d);
        }
        self.dirty = true;
    }

    fn new_draft(&mut self) -> Draft {
        let id = self.store.new_id();
        let account_id = self.account_filter.or(self.store.accounts.first().map(|a| a.id)).unwrap_or(1);
        let signature_id = self.store.signature_for_new(account_id);
        Draft { id, account_id, signature_id, format: BodyFormat::Html, updated: now_ts(), ..Default::default() }
    }

    fn reply_draft(&mut self, email_id: u64, all: bool) -> Option<Draft> {
        let e = self.store.email(email_id)?.clone();
        let own = self.store.account(e.account_id).map(|a| a.email.to_lowercase()).unwrap_or_default();
        let mut d = self.new_draft();
        d.account_id = e.account_id;
        d.reply_to = Some(email_id);
        d.to = e.reply_recipients().join(", ");
        if all {
            let others: Vec<String> = e.to.iter().chain(e.cc.iter()).filter(|x| x.to_lowercase() != own && !e.reply_recipients().iter().any(|to| to.eq_ignore_ascii_case(x))).cloned().collect();
            d.cc = others.join(", ");
        }
        d.subject = if e.subject.to_lowercase().starts_with("re:") { e.subject.clone() } else { format!("Re: {}", e.subject) };
        d.signature_id = self.store.signature_for_reply(&e);
        d.quote = format!("On {}, {} wrote:\n{}", crate::infrastructure::timezone::format_timestamp(&self.store.settings, e.date), e.from_name, e.body.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n"));
        Some(d)
    }

    fn generate_reply(&mut self, email_id: u64, instruction: String) {
        if let Some(d) = self.reply_draft(email_id, false) {
            self.open_composer(d);
            self.ai_busy = true;
            let Some(e) = self.store.email(email_id).cloned() else { return };
            let settings = self.store.settings.clone();
            self.spawn(move || Msg::Reply { email: email_id, text: ai::generate_reply(&settings, &e, &instruction) });
        }
    }

    // ---------- Message handling ----------

    fn drain_messages(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            match m {
                Msg::Note(n) => {
                    if !n.is_empty() {
                        self.status = n;
                    }
                }
                Msg::LoginChecked { account, is_new, result } => {
                    self.checking_login = false;
                    match result {
                        Err(e) if account.email.is_empty() => self.login_result = Some(Err(e)),
                        Err(e) => self.login_result = Some(Err(e)),
                        Ok(()) if is_new => self.finish_add_account(account),
                        Ok(()) => self.login_result = Some(Ok(format!("{}: sign-in works, for receiving and for sending.", account.email))),
                    }
                }
                Msg::Synced { account, result, folders } => {
                    self.syncing.remove(&account);
                    if let Some(acc) = self.store.accounts.iter_mut().find(|a| a.id == account) {
                        acc.last_sync = Some(now_ts());
                        if let Some((all, trash)) = folders {
                            if let Some(x) = all {
                                acc.archive_folder = x;
                            }
                            if let Some(x) = trash {
                                acc.trash_folder = x;
                            }
                        }
                    }
                    match result {
                        Ok(fresh) => {
                            let added = mail::merge(&mut self.store, account, fresh);
                            self.status = format!("Gmail synced: {} new", added.len());
                            self.dirty = true;
                            if !added.is_empty() {
                                self.classify_new(added);
                                self.start_indexing();
                            }
                        }
                        Err(e) => self.status = format!("Gmail sync failed: {e}"),
                    }
                }
                Msg::Reply { email, text } => {
                    self.ai_busy = false;
                    if let Some(t) = self.tabs.items.iter_mut().rev().find(|t| t.draft.reply_to == Some(email)) {
                        t.draft.body = if t.draft.format == BodyFormat::Plain { text } else { richtext::plain_to_html(&text) };
                    }
                }
                Msg::Classified { email, category, labels } => {
                    self.ai_busy = false;
                    self.apply_classification(email, category, labels);
                }
                Msg::AiOutput { email, title, text } => {
                    self.ai_busy = false;
                    self.ai_output = Some((email, title, text));
                }
                Msg::Writing { draft, text } => {
                    self.ai_busy = false;
                    if let Some(t) = self.tabs.items.iter_mut().find(|t| t.draft.id == draft) {
                        t.draft.body = if t.draft.format == BodyFormat::Plain { text } else { richtext::plain_to_html(&text) };
                    }
                }
                Msg::Sent { draft, result } => {
                    self.sending.remove(&draft);
                    match result {
                        Ok(()) => self.finish_send(draft),
                        Err(e) => {
                            self.status = format!("Not sent, it stays in Drafts: {e}");
                            if let Some(d) = self.store.drafts.iter_mut().find(|d| d.id == draft) {
                                d.send_at = None;
                            }
                            self.dirty = true;
                        }
                    }
                }
                Msg::ModelChecked(ready) => {
                    self.model_ready = ready;
                    if ready {
                        self.start_indexing();
                    }
                }
                Msg::IndexDone(r) => match r {
                    Ok((idx, rep)) => {
                        let unread = if rep.unreadable_attachments > 0 {
                            format!(" {} attachments had no readable text.", rep.unreadable_attachments)
                        } else {
                            String::new()
                        };
                        let text = format!("{} emails ready for semantic search.{unread}", idx.email_count());
                        self.index = Arc::new(idx);
                        self.rag_set(&text, false, 1.0);
                    }
                    Err(e) => self.rag_set(&e, false, 0.0),
                },
                Msg::Search { query, hits, note } => {
                    self.searching = false;
                    self.search_note = note;
                    if query == self.search.trim() {
                        self.selected = hits.first().map(|h| h.email_id).or(self.selected);
                        self.hits = Some((query, hits));
                    }
                }
                Msg::Answer { question, answer, sources } => {
                    self.asking = false;
                    self.chat.push((question, answer, sources));
                }
                Msg::Transcribed { meeting, result } => match result {
                    Ok(t) => {
                        if let Some(m) = self.store.meetings.iter_mut().find(|m| m.id == meeting) {
                            m.transcript = t;
                            self.dirty = true;
                        }
                        self.status = "Transcript ready.".into();
                    }
                    Err(e) => self.status = e,
                },
                Msg::MeetingNotes { meeting, summary, key_points, action_items } => {
                    self.ai_busy = false;
                    if let Some(m) = self.store.meetings.iter_mut().find(|m| m.id == meeting) {
                        m.summary = summary;
                        m.key_points = key_points;
                        m.action_items = action_items;
                        self.dirty = true;
                    }
                }
                Msg::AttachmentText { name, result } => {
                    self.attachment_summary.clear();
                    self.attachment_view = Some((name, result));
                }
                Msg::Summary(t) => {
                    self.ai_busy = false;
                    self.attachment_summary = t;
                }
            }
        }
    }

    fn finish_send(&mut self, draft: u64) {
        let Some(pos) = self.store.drafts.iter().position(|d| d.id == draft) else { return };
        let d = self.store.drafts.remove(pos);
        let sent_text = richtext::render_draft(&self.store, &d).plain;
        // A reply stays in its conversation, so later replies can find the signature used.
        let thread = d.reply_to.and_then(|r| self.store.email(r)).map(|e| e.thread_id.clone());
        if let Some(a) = self.store.accounts.iter_mut().find(|a| a.id == d.account_id) {
            a.last_signature = d.signature_id;
        }
        let id = self.store.new_id();
        let (name, addr) = self
            .store
            .account(d.account_id)
            .map(|a| (self.store.settings.user_name.clone(), a.email.clone()))
            .unwrap_or_default();
        self.store.emails.push(Email {
            id,
            account_id: d.account_id,
            folder: "Sent".into(),
            thread_id: thread.unwrap_or_else(|| format!("t{id}")),
            from_name: name,
            from_addr: addr,
            to: mail::split_addrs(&d.to),
            cc: mail::split_addrs(&d.cc),
            subject: d.subject.clone(),
            body: sent_text,
            date: now_ts(),
            read: true,
            classified: true,
            ..Default::default()
        });
        if let Some(r) = d.reply_to.and_then(|r| self.store.email_mut(r)) {
            r.replied = true;
        }
        self.dirty = true;
        self.status = format!("Sent: {}", d.subject);
    }

    fn tick(&mut self) {
        let now = now_ts();
        if now == self.last_tick || self.screenshot.is_some() {
            return;
        }
        self.last_tick = now;
        if self.store.wake_snoozed(now) > 0 {
            self.status = "Snoozed message returned to your Inbox".into();
            self.dirty = true;
        }
        let mut reminded = Vec::new();
        for e in &mut self.store.emails {
            if e.reminder_at.is_some_and(|t| t <= now) {
                e.reminder_at = None;
                if !e.replied {
                    e.read = false;
                    e.pinned = true;
                    reminded.push(e.subject.clone());
                }
            }
        }
        if let Some(s) = reminded.first() {
            self.status = format!("Reminder: {s}");
            self.dirty = true;
        }
        let due: Vec<Draft> = self.store.drafts.iter().filter(|d| d.send_at.is_some_and(|t| t <= now) && !self.sending.contains(&d.id)).cloned().collect();
        for d in due {
            self.send_draft(d);
        }
        if now - self.last_autosave >= composer_tabs::AUTOSAVE_SECS {
            self.last_autosave = now;
            self.save_open_tabs();
        }
        if !self.dirty {
            let m = Store::mtime();
            if m != self.last_mtime {
                self.store = Store::load_or_default();
                self.last_mtime = m;
            }
        }
        if now - self.last_sync >= AUTO_SYNC_SECS {
            self.sync_all();
        }
    }

    fn save_if_needed(&mut self) {
        if self.dirty && self.screenshot.is_none() {
            match self.store.save() {
                Ok(()) => {
                    use crate::application::ports::StoreRepository;
                    let ids: Vec<_> = self.attachment_imports.iter().filter(|(id, _)| {
                        self.tabs.items.iter().find(|tab| tab.draft.id == *id).is_none_or(|tab| {
                            self.store.drafts.iter().find(|draft| draft.id == *id)
                                .is_some_and(|draft| draft.attachments == tab.draft.attachments)
                        })
                    }).map(|(_, lease)| lease.clone()).collect();
                    let root = Store::data_dir();
                    if crate::infrastructure::portable_files::release_draft_attachment_imports(&root, &ids).is_ok() {
                        self.attachment_imports.retain(|(_, lease)| !ids.contains(lease));
                    }
                    let _ = crate::infrastructure::repository::JsonStoreRepository::new(Store::path()).reconcile_storage();
                    self.last_mtime = Store::mtime();
                }
                Err(e) => self.status = format!("Couldn't save: {e}"),
            }
        }
        self.dirty = false;
    }

    // ---------- Panels ----------

    fn top_bar(&mut self, ctx: &egui::Context) {
        let p = self.pal();
        let top = egui::TopBottomPanel::top("top")
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(14, 9)).stroke(Stroke::new(1.0, p.border)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("✉ Qamille").size(16.0).strong().color(p.accent));
                    ui.add_space(70.0);
                    let hint = if self.index.passages.is_empty() { "Search mail and attachments" } else { "Search by meaning or exact words, then press Enter" };
                    let w = (ui.available_width() - 260.0).clamp(220.0, 640.0);
                    let resp = ui.add(TextEdit::singleline(&mut self.search).hint_text(hint).desired_width(w).margin(Margin::symmetric(10, 5)));
                    if resp.changed() {
                        match crate::infrastructure::search_support::parse(&self.store, &self.search, self.account_filter, now_ts()) {
                            Ok(_) => {
                                self.last_valid_search = self.search.clone();
                                self.hits = None;
                                self.search_note = None;
                            }
                            Err(error) => { self.search_note = Some(format!("{error} · Showing the last valid search")); }
                        }
                    }
                    if resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        self.run_search();
                    }
                    if !self.search.is_empty() && ui.button("✖").on_hover_text("Clear search").clicked() {
                        self.search.clear();
                        self.hits = None;
                        self.search_note = None;
                    }
                    if self.searching {
                        ui.spinner();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        let compose = egui::Button::new(RichText::new("✏ Compose").color(Color32::WHITE).strong()).fill(p.accent);
                        if ui.add(compose).clicked() {
                            let d = self.new_draft();
                            self.open_composer(d);
                        }
                        let has_gmail = self.store.accounts.iter().any(|a| a.kind == AccountKind::Gmail);
                        if has_gmail {
                            if !self.syncing.is_empty() {
                                ui.spinner();
                            } else if ui.button("⟳").on_hover_text("Check Gmail now").clicked() {
                                self.sync_all();
                            }
                        }
                    });
                });
            });
        self.area_top = top.response.rect.bottom();
    }

    fn status_bar(&mut self, ctx: &egui::Context) {
        let p = self.pal();
        let rag = self.rag_status.lock().map(|s| s.clone()).unwrap_or_default();
        let text = if !self.status.is_empty() { self.status.clone() } else { rag.message.clone() };
        let semantic = if self.model_ready {
            if self.index.passages.is_empty() { "Semantic search on · nothing read yet".to_string() } else { format!("Semantic search on · {} passages", self.index.passages.len()) }
        } else {
            "Semantic search not installed".to_string()
        };
        let footer = egui::TopBottomPanel::bottom("status")
            .frame(egui::Frame::new().fill(p.side).inner_margin(Margin::symmetric(14, 4)).stroke(Stroke::new(1.0, p.border)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    self.footer_tabs(ui);
                    ui.add(egui::Label::new(RichText::new(text).size(12.0).color(p.muted)).truncate());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        ui.label(RichText::new(semantic).size(12.0).color(p.muted));
                        if rag.busy {
                            ui.add(egui::ProgressBar::new(rag.fraction).desired_width(120.0));
                        }
                    });
                });
            });
        self.footer_top = footer.response.rect.top();
    }

    fn side_row(&self, ui: &mut egui::Ui, icon: &str, dot: Option<[u8; 3]>, title: &str, active: bool, badge: usize) -> egui::Response {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 30.0), Sense::click());
        if ui.is_rect_visible(rect) {
            let fill = if active {
                SIDE_ACTIVE
            } else if resp.hovered() {
                SIDE_HOVER
            } else {
                Color32::TRANSPARENT
            };
            ui.painter().rect_filled(rect, CornerRadius::same(6), fill);
            let color = if active { Color32::WHITE } else { SIDE_TEXT };
            if let Some(c) = dot {
                ui.painter().circle_filled(rect.left_center() + vec2(16.0, 0.0), 4.0, Color32::from_rgb(c[0], c[1], c[2]));
            } else {
                ui.painter().text(rect.left_center() + vec2(10.0, 0.0), Align2::LEFT_CENTER, icon, FontId::proportional(14.0), color);
            }
            let clip = egui::Rect::from_min_max(rect.min, pos2(rect.right() - 28.0, rect.bottom()));
            ui.painter_at(clip).text(rect.left_center() + vec2(34.0, 0.0), Align2::LEFT_CENTER, title, FontId::proportional(13.5), color);
            if badge > 0 {
                ui.painter().text(rect.right_center() - vec2(10.0, 0.0), Align2::RIGHT_CENTER, badge.to_string(), FontId::proportional(12.0), SIDE_TEXT);
            }
        }
        resp
    }

    fn nav_row(&mut self, ui: &mut egui::Ui, icon: &str, title: &str, view: View, badge: usize) {
        let active = self.view == view && self.search.trim().is_empty();
        if self.side_row(ui, icon, None, title, active, badge).clicked() {
            self.search.clear();
            self.hits = None;
            self.view = view;
            self.selected = self.mail_ids().first().copied();
            self.ai_output = None;
        }
    }

    fn sidebar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("side")
            .exact_width(212.0)
            .resizable(false)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(SIDE_BG).inner_margin(Margin::same(10)))
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().id_salt("side_scroll").scroll_bar_visibility(egui::scroll_area::ScrollBarVisibility::AlwaysHidden).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 2.0;
                    let unread = self.count(View::Inbox);
                    let tasks = self.count(View::Tasks);
                    let snoozed = self.count(View::Snoozed);
                    self.nav_row(ui, "📥", "Smart Inbox", View::Inbox, unread);
                    self.nav_row(ui, "☑", "Tasks", View::Tasks, tasks);
                    self.nav_row(ui, "⏰", "Snoozed", View::Snoozed, snoozed);
                    let drafts = self.store.drafts.len();
                    self.nav_row(ui, "📝", "Drafts", View::Drafts, drafts);
                    self.nav_row(ui, "📤", "Sent", View::Sent, 0);
                    self.nav_row(ui, "📦", "Archive", View::Archive, 0);
                    self.nav_row(ui, "🗑", "Trash", View::Trash, 0);
                    ui.add_space(8.0);
                    self.nav_row(ui, "📅", "Calendar", View::Calendar, 0);
                    self.nav_row(ui, "🎤", "Meetings", View::Meetings, 0);
                    self.nav_row(ui, "✨", "Assistant", View::Assistant, 0);
                    ui.add_space(10.0);
                    ui.label(RichText::new("ACCOUNTS").size(11.0).color(SIDE_TEXT));
                    if self.side_row(ui, "✉", None, "All Accounts", self.account_filter.is_none(), 0).clicked() {
                        self.account_filter = None;
                        self.hits = None;
                        self.selected = self.mail_ids().first().copied();
                    }
                    let accounts: Vec<(u32, String, [u8; 3])> = self.store.accounts.iter().map(|a| (a.id, a.name.clone(), a.color)).collect();
                    for (id, name, c) in accounts {
                        if self.side_row(ui, "", Some(c), &name, self.account_filter == Some(id), 0).clicked() {
                            self.account_filter = Some(id);
                            self.hits = None;
                            self.selected = self.mail_ids().first().copied();
                        }
                    }
                    let labels = self.store.all_labels();
                    if !labels.is_empty() {
                        ui.add_space(10.0);
                        ui.label(RichText::new("LABELS").size(11.0).color(SIDE_TEXT));
                        for l in labels {
                            let badge = self.count(View::Label(l.clone()));
                            self.nav_row(ui, "🏷", &l, View::Label(l.clone()), badge);
                        }
                    }
                    ui.add_space(10.0);
                    self.nav_row(ui, "⚙", "Settings", View::Settings, 0);
                });
            });
    }

    fn mail_row(&self, ui: &mut egui::Ui, e: &Email, passage: Option<&str>, selected: bool) -> (egui::Response, bool) {
        let p = self.pal();
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 66.0), Sense::click());
        let mut done_clicked = false;
        if !ui.is_rect_visible(rect) {
            return (resp, false);
        }
        let hovered = resp.hovered();
        let fill = if selected {
            p.selected
        } else if hovered {
            p.hover
        } else {
            Color32::TRANSPARENT
        };
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect.shrink2(vec2(4.0, 1.0)), CornerRadius::same(6), fill);
        // Done circle: a mail is a task you can tick off.
        let center = rect.left_center() + vec2(20.0, 0.0);
        let hit = egui::Rect::from_center_size(center, vec2(24.0, 24.0));
        let check = ui.interact(hit, resp.id.with("done"), Sense::click());
        let ring = if check.hovered() || !e.read { p.accent } else { p.border };
        painter.circle_stroke(center, 8.0, Stroke::new(1.5, ring));
        if check.hovered() || e.done {
            painter.text(center, Align2::CENTER_CENTER, "✔", FontId::proportional(11.0), p.accent);
        } else if !e.read {
            painter.circle_filled(center, 3.5, p.accent);
        }
        if check.on_hover_text("Done").clicked() {
            done_clicked = true;
        }
        let left = rect.left() + 38.0;
        let right = rect.right() - 12.0;
        let name_font = FontId::proportional(13.5);
        let sender_color = if e.read { p.muted } else { p.text };
        let text_area = egui::Rect::from_min_max(pos2(left, rect.top()), pos2(right, rect.bottom()));
        let text_painter = ui.painter_at(text_area);
        text_painter.text(pos2(left, rect.top() + 8.0), Align2::LEFT_TOP, truncate(&e.from_name, 34), name_font, sender_color);
        painter.text(pos2(right, rect.top() + 9.0), Align2::RIGHT_TOP, if e.date_missing { "Date unavailable".into() } else { crate::infrastructure::timezone::format_timestamp(&self.store.settings, e.date) }, FontId::proportional(11.5), p.muted);
        let mut flags = String::new();
        if e.pinned {
            flags.push_str("📌 ");
        }
        if !e.attachments.is_empty() {
            flags.push_str("📎 ");
        }
        if e.reminder_at.is_some() {
            flags.push_str("🔔 ");
        }
        if !flags.is_empty() {
            painter.text(pos2(right - 44.0, rect.top() + 9.0), Align2::RIGHT_TOP, flags, FontId::proportional(11.0), p.muted);
        }
        let subject_color = if e.read { p.muted } else { p.text };
        text_painter.text(pos2(left, rect.top() + 27.0), Align2::LEFT_TOP, truncate(&e.subject, 90), FontId::proportional(13.0), subject_color);
        let snippet = match passage {
            Some(t) if !t.is_empty() => truncate(&t.split_whitespace().collect::<Vec<_>>().join(" "), 110),
            _ => e.snippet(110),
        };
        text_painter.text(pos2(left, rect.top() + 46.0), Align2::LEFT_TOP, snippet, FontId::proportional(11.5), p.muted);
        // Which inbox it came from: a small bar in the account's colour.
        if let Some(a) = self.store.account(e.account_id) {
            let bar = egui::Rect::from_min_size(pos2(rect.left() + 5.0, rect.top() + 12.0), vec2(3.0, 42.0));
            painter.rect_filled(bar, CornerRadius::same(2), Color32::from_rgb(a.color[0], a.color[1], a.color[2]));
        }
        (resp, done_clicked)
    }

    fn list_panel(&mut self, ctx: &egui::Context) {
        let p = self.pal();
        let ids = self.mail_ids();
        let searching = !self.search.trim().is_empty();
        let title = if searching {
            let by_meaning = self.hits.as_ref().is_some_and(|(q, _)| q == self.search.trim()) && !self.index.passages.is_empty();
            let n = ids.len();
            if by_meaning {
                format!("{n} {} by meaning and words", if n == 1 { "result" } else { "results" })
            } else {
                format!("{n} exact {}, press Enter for meaning", if n == 1 { "match" } else { "matches" })
            }
        } else {
            match &self.view {
                View::Inbox => "Smart Inbox".to_string(),
                View::Tasks => "Tasks".to_string(),
                View::Snoozed => "Snoozed".to_string(),
                View::Sent => "Sent".to_string(),
                View::Archive => "Archive".to_string(),
                View::Trash => "Trash".to_string(),
                View::Label(l) => l.clone(),
                _ => String::new(),
            }
        };
        let mut open = None;
        let mut done = None;
        let mut toggled = None;
        let mut more: Option<&'static str> = None;
        egui::SidePanel::left("list")
            .default_width(372.0)
            .width_range(300.0..=520.0)
            .show_separator_line(false)
            .frame(egui::Frame::new().fill(p.bg).inner_margin(Margin::symmetric(0, 8)).stroke(Stroke::new(1.0, p.border)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.add_space(14.0);
                    ui.label(RichText::new(title).size(15.0).strong());
                });
                if let Some(n) = &self.search_note {
                    ui.horizontal(|ui| {
                        ui.add_space(14.0);
                        ui.label(RichText::new(n).size(11.5).color(p.muted));
                    });
                }
                egui::ScrollArea::vertical().id_salt("list_scroll").auto_shrink([false, false]).show(ui, |ui| {
                    ui.spacing_mut().item_spacing.y = 0.0;
                    if ids.is_empty() {
                        ui.add_space(50.0);
                        ui.vertical_centered(|ui| {
                            ui.label(RichText::new(if searching { "Nothing found" } else { "All done here" }).color(p.muted));
                        });
                    }
                    let mut current: Option<&'static str> = None;
                    let now = now_ts();
                    let mut in_group = 0usize;
                    let card_limit = |g: &'static str| -> Option<usize> {
                        Section::ORDER.iter().find(|s| s.label() == g).and_then(|s| s.limit())
                    };
                    let group_size = |g: &'static str| -> usize {
                        ids.iter().filter_map(|i| self.store.email(*i)).filter(|e| rules::section_with_lock(e, self.locked_section).label() == g).count()
                    };
                    for id in &ids {
                        let Some(e) = self.store.email(*id) else { continue };
                        let group: Option<&'static str> = if searching {
                            None
                        } else if self.view == View::Inbox {
                            Some(rules::section_with_lock(e, self.locked_section).label())
                        } else if self.view == View::Tasks {
                            Some(task_group(e, now, &self.store.settings))
                        } else {
                            None
                        };
                        if group != current {
                            current = group;
                            in_group = 0;
                            if let Some(g) = group {
                                let closed = self.collapsed.contains(g);
                                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 28.0), Sense::click());
                                let c = r.left_center() + vec2(18.0, 2.0);
                                let tri = if closed {
                                    vec![c + vec2(-3.0, -4.0), c + vec2(3.0, 0.0), c + vec2(-3.0, 4.0)]
                                } else {
                                    vec![c + vec2(-4.0, -3.0), c + vec2(4.0, -3.0), c + vec2(0.0, 3.0)]
                                };
                                ui.painter().add(egui::Shape::convex_polygon(tri, p.muted, Stroke::NONE));
                                ui.painter().text(r.left_center() + vec2(28.0, 2.0), Align2::LEFT_CENTER, g.to_uppercase(), FontId::proportional(11.0), p.muted);
                                if resp.clicked() {
                                    toggled = Some(g);
                                }
                            }
                        }
                        if group.is_some_and(|g| self.collapsed.contains(g)) {
                            continue;
                        }
                        in_group += 1;
                        // A card shows its first few; "show more" opens rules::MAX_VISIBLE_ITEMS at a time.
                        if let Some((g, limit)) = group.filter(|_| self.view == View::Inbox).and_then(|g| card_limit(g).map(|l| (g, l))) {
                            let allowed = limit + self.shown_extra.get(g).copied().unwrap_or(0);
                            if in_group > allowed {
                                if in_group == allowed + 1 {
                                    let left = group_size(g) - allowed;
                                    let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 26.0), Sense::click());
                                    let color = if resp.hovered() { p.accent } else { p.muted };
                                    ui.painter().text(r.left_center() + vec2(38.0, 0.0), Align2::LEFT_CENTER, format!("Show {} more", left.min(rules::MAX_VISIBLE_ITEMS)), FontId::proportional(12.0), color);
                                    if resp.clicked() {
                                        more = Some(g);
                                    }
                                }
                                continue;
                            }
                        }
                        let passage = self
                            .hits
                            .as_ref()
                            .filter(|_| searching)
                            .and_then(|(_, hs)| hs.iter().find(|h| h.email_id == *id))
                            .map(|h| h.passage.as_str());
                        let (resp, clicked_done) = self.mail_row(ui, e, passage, self.selected == Some(*id));
                        if resp.clicked() {
                            open = Some(*id);
                        }
                        if clicked_done {
                            done = Some(*id);
                        }
                    }
                });
            });
        if let Some(g) = more {
            *self.shown_extra.entry(g).or_default() += rules::MAX_VISIBLE_ITEMS;
        }
        if let Some(g) = toggled {
            if !self.collapsed.remove(g) {
                self.collapsed.insert(g);
            }
        }
        if let Some(id) = open {
            self.open_mail(id);
        }
        if let Some(id) = done {
            self.after_remove(id);
            self.act(id, "markAsDone", None);
        }
    }

    fn schedule_menu(&mut self, ui: &mut egui::Ui, title: &str, kind: ScheduleKind, action: &str, id: u64) {
        ui.menu_button(title, |ui| {
            let now = Local::now().with_timezone(&crate::infrastructure::timezone::zone(&self.store.settings));
            for preset in Preset::list(kind) {
                self.schedule_button(ui, preset, action, id, now);
            }
            ui.menu_button("More", |ui| {
                for preset in Preset::more(kind) {
                    self.schedule_button(ui, preset, action, id, now);
                }
            });
            ui.separator();
            ui.add(TextEdit::singleline(&mut self.custom_when).hint_text("yyyy-mm-dd").desired_width(150.0));
            if ui.button("Set this date").clicked() {
                let v = self.custom_when.clone();
                self.act(id, action, Some(&v));
                ui.close();
            }
        });
    }

    fn schedule_button(&mut self, ui: &mut egui::Ui, preset: Preset, action: &str, id: u64, now: chrono::DateTime<chrono_tz::Tz>) {
        let Some(t) = preset.time(now) else { return };
        let when = if rules::is_someday(t, now.timestamp()) { "Someday".into() }
            else { crate::infrastructure::timezone::format_timestamp(&self.store.settings, t) };
        let text = if when == preset.label() { when } else { format!("{}   {}", preset.label(), when) };
        if ui.button(text).clicked() {
            self.act(id, action, Some(preset.label()));
            if action == "snooze" {
                self.after_remove(id);
            }
            ui.close();
        }
    }

    fn reading_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let Some(id) = self.selected else {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Select a message").color(p.muted));
            });
            return;
        };
        let Some(e) = self.store.email(id).cloned() else {
            self.selected = None;
            return;
        };
        egui::Frame::new().inner_margin(Margin::symmetric(22, 14)).show(ui, |ui| {
            // Task actions
            ui.horizontal_wrapped(|ui| {
                let done = egui::Button::new(RichText::new("✔ Done").color(Color32::WHITE)).fill(p.accent);
                if ui.add(done).on_hover_text("Finish this task and clear it from the inbox").clicked() {
                    self.after_remove(id);
                    self.act(id, "markAsDone", None);
                }
                if ui.button(if e.pinned { "📌 Unpin" } else { "📌 Pin" }).clicked() {
                    self.act(id, if e.pinned { "unpin" } else { "pin" }, None);
                }
                self.schedule_menu(ui, "⏰ Snooze", ScheduleKind::Snooze, "snooze", id);
                self.schedule_menu(ui, "🔔 Remind", ScheduleKind::Reminder, "changeReminder", id);
                self.schedule_menu(ui, "📅 Due", ScheduleKind::Reminder, "setDue", id);
                ui.menu_button("🏷 Label", |ui| {
                    for l in self.store.all_labels() {
                        let on = e.labels.contains(&l);
                        if ui.selectable_label(on, l.as_str()).clicked() {
                            self.act(id, if on { "detachLabel" } else { "attachLabel" }, Some(&l));
                            ui.close();
                        }
                    }
                    ui.separator();
                    ui.add(TextEdit::singleline(&mut self.new_label).hint_text("New label").desired_width(130.0));
                    if ui.button("Add").clicked() {
                        let l = self.new_label.trim().to_string();
                        if !l.is_empty() {
                            self.act(id, "attachLabel", Some(&l));
                        }
                        self.new_label.clear();
                        ui.close();
                    }
                });
                if ui.button("📦 Archive").clicked() {
                    self.after_remove(id);
                    self.act(id, "archive", None);
                }
                if ui.button("🗑").on_hover_text("Move to Trash").clicked() {
                    self.after_remove(id);
                    self.act(id, "moveToTrash", None);
                }
            });
            ui.add_space(8.0);
            ui.separator();
            ui.add_space(4.0);
            egui::ScrollArea::vertical().id_salt("read_scroll").auto_shrink([false, false]).show(ui, |ui| {
                ui.add(egui::Label::new(RichText::new(&e.subject).size(20.0).strong()).wrap());
                ui.add_space(4.0);
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new(&e.from_name).strong());
                    ui.label(RichText::new(format!("<{}>", e.from_addr)).size(12.0).color(p.muted));
                    ui.label(RichText::new(format!("· {}", crate::infrastructure::timezone::format_timestamp(&self.store.settings, e.date))).size(12.0).color(p.muted));
                });
                ui.label(RichText::new(format!("To: {}", e.to.join(", "))).size(12.0).color(p.muted));
                ui.horizontal_wrapped(|ui| {
                    chip(ui, e.category.label(), p.accent);
                    for l in &e.labels {
                        chip(ui, l, p.muted);
                    }
                    if let Some(d) = e.due {
                        chip(ui, &format!("Due {}", crate::infrastructure::timezone::format_timestamp(&self.store.settings, d)), if d < now_ts() { p.danger } else { p.muted });
                    }
                    if let Some(d) = e.reminder_at {
                        chip(ui, &format!("Reminder {}", crate::infrastructure::timezone::format_timestamp(&self.store.settings, d)), p.muted);
                    }
                    if let Some(d) = e.snoozed_until {
                        chip(ui, &format!("Snoozed: {}", rules::fmt_when(d, now_ts())), p.muted);
                    }
                });
                ui.add_space(8.0);
                // +AI
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("✨ +AI").strong().color(p.accent));
                    ui.add(TextEdit::singleline(&mut self.reply_hint).hint_text("What should the reply say? (optional)").desired_width(220.0));
                    if ui.button("Generate reply").clicked() {
                        let h = self.reply_hint.clone();
                        self.generate_reply(id, h);
                    }
                    if ui.button("Classify and label").clicked() {
                        self.ai_busy = true;
                        let (s, m, known) = (self.store.settings.clone(), e.clone(), self.store.all_labels());
                        self.spawn(move || {
                            let (category, labels) = ai::classify(&s, &m, &known);
                            Msg::Classified { email: m.id, category, labels }
                        });
                    }
                    ui.menu_button("Summary", |ui| {
                        for kind in ai::SUMMARY_COMMANDS {
                            if ui.button(kind).clicked() {
                                self.ai_busy = true;
                                let (s, m) = (self.store.settings.clone(), e.clone());
                                self.spawn(move || Msg::AiOutput {
                                    email: m.id,
                                    title: kind.to_string(),
                                    text: ai::summarize(&s, kind, &format!("Subject: {}\n{}", m.subject, m.body)),
                                });
                                ui.close();
                            }
                        }
                    });
                    if self.ai_busy {
                        ui.spinner();
                    }
                });
                if let Some((eid, title, text)) = self.ai_output.clone() {
                    if eid == id {
                        egui::Frame::new().fill(p.selected).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(12)).show(ui, |ui| {
                            ui.label(RichText::new(title).strong());
                            ui.add(egui::Label::new(text.as_str()).selectable(true).wrap());
                            ui.horizontal(|ui| {
                                if ui.small_button("Copy").clicked() {
                                    ui.ctx().copy_text(text.clone());
                                }
                                if ui.small_button("Close").clicked() {
                                    self.ai_output = None;
                                }
                            });
                        });
                    }
                }
                ui.add_space(10.0);
                ui.add(egui::Label::new(RichText::new(&e.body).size(14.0)).selectable(true).wrap());
                ui.add_space(12.0);
                if !e.attachments.is_empty() {
                    ui.label(RichText::new("ATTACHMENTS").size(11.0).color(p.muted));
                    for a in &e.attachments {
                        ui.horizontal(|ui| {
                            ui.label(format!("📎 {}  ({})", a.name, attach::human_size(a.size)));
                            if ui.small_button("Read").clicked() {
                                let att = a.clone();
                                self.spawn(move || Msg::AttachmentText { name: att.name.clone(), result: attach::read_text(&att) });
                            }
                            if let Some(path) = &a.path {
                                if ui.small_button("Open").clicked() {
                                    if let Err(x) = open::that(path) {
                                        self.status = x.to_string();
                                    }
                                }
                            }
                        });
                    }
                    ui.add_space(8.0);
                }
                ui.horizontal(|ui| {
                    if ui.button("↩ Reply").clicked() {
                        if let Some(d) = self.reply_draft(id, false) {
                            self.open_composer(d);
                        }
                    }
                    if ui.button("↩↩ Reply all").clicked() {
                        if let Some(d) = self.reply_draft(id, true) {
                            self.open_composer(d);
                        }
                    }
                    if ui.button("➡ Forward").clicked() {
                        let mut d = self.new_draft();
                        d.account_id = e.account_id;
                        d.subject = format!("Fwd: {}", e.subject);
                        d.quote = format!("---------- Forwarded message ----------\nFrom: {} <{}>\nDate: {}\nSubject: {}\n\n{}", e.from_name, e.from_addr, crate::infrastructure::timezone::format_timestamp(&self.store.settings, e.date), e.subject, e.body);
                        d.attachments = e.attachments.iter().filter_map(|a| a.path.clone()).collect();
                        self.open_composer(d);
                    }
                });
            });
        });
    }

    fn drafts_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        egui::Frame::new().inner_margin(Margin::same(18)).show(ui, |ui| {
            ui.label(RichText::new("Drafts").size(18.0).strong());
            ui.add_space(6.0);
            let drafts = self.store.drafts.clone();
            if drafts.is_empty() {
                ui.label(RichText::new("No drafts").color(p.muted));
            }
            for d in drafts {
                ui.horizontal(|ui| {
                    let label = format!("{}   →  {}", if d.subject.is_empty() { "(no subject)" } else { &d.subject }, if d.to.is_empty() { "no recipient" } else { &d.to });
                    if ui.link(label).clicked() {
                        self.open_composer(d.clone());
                    }
                    if let Some(t) = d.send_at {
                        ui.label(RichText::new(format!("Sends {}", crate::infrastructure::timezone::format_timestamp(&self.store.settings, t))).size(12.0).color(p.accent));
                    }
                    if ui.small_button("Delete").clicked() {
                        self.store.drafts.retain(|x| x.id != d.id);
                        self.tabs.items.retain(|t| t.draft.id != d.id);
                        self.dirty = true;
                    }
                });
            }
        });
    }

    /// The open message: a small window above the footer, or the whole area below the top bar.
    fn composer_window(&mut self, ctx: &egui::Context) {
        let Some(at) = self.tabs.open_index() else { return };
        let mut tab: Tab = self.tabs.items.remove(at);
        let mut keep = true;
        let mut send = false;
        let mut later = None;
        let mut save = false;
        let mut collapse = false;
        let mut toggle_full = false;
        let mut close = false;
        let pal = self.pal();
        let accounts: Vec<(u32, String)> = self.store.accounts.iter().map(|a| (a.id, format!("{} <{}>", a.name, a.email))).collect();
        let screen = ctx.content_rect();
        let bottom = self.footer_top.min(screen.bottom());
        let full = tab.state == TabState::Full;
        let edge = 12.0; // estimated
        let (rect, padding) = if full {
            let m = if screen.width() < composer_tabs::FULL_COMPACT_BELOW { composer_tabs::FULL_COMPACT_PADDING } else { composer_tabs::FULL_PADDING };
            (egui::Rect::from_min_max(pos2(screen.left(), self.area_top), pos2(screen.right(), bottom)), Margin::symmetric(m, 14))
        } else {
            let w = 640.0_f32.max(composer_tabs::PIP_MIN_WIDTH).min(screen.width() - edge * 2.0);
            let h = 620.0_f32.max(composer_tabs::PIP_MIN_HEIGHT).min(bottom - self.area_top - edge * 2.0);
            (egui::Rect::from_min_max(pos2(screen.right() - edge - w, bottom - edge - h), pos2(screen.right() - edge, bottom - edge)), Margin::same(composer_tabs::PIP_PADDING))
        };
        let mut frame = egui::Frame::window(&ctx.style()).fill(pal.bg).stroke(Stroke::new(1.0, pal.border)).inner_margin(padding);
        if full {
            frame = frame.corner_radius(CornerRadius::ZERO).shadow(egui::epaint::Shadow::NONE);
        }
        let title = tab.title();
        let inner = rect.size() - padding.sum() - Vec2::splat(2.0);
        egui::Area::new(egui::Id::new("composer_window"))
            .order(egui::Order::Middle)
            .fixed_pos(rect.min)
            .constrain(false)
            .show(ctx, |ui| frame.show(ui, |ui| {
                ui.set_min_size(inner);
                ui.set_max_size(inner);
                let d = &mut tab.draft;
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(RichText::new(truncate(&title, 70)).size(15.0).strong()).truncate());
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if window_button(ui, WindowIcon::Close, &pal).on_hover_text("Close Composer").clicked() {
                            close = true;
                        }
                        let icon = if full { WindowIcon::Restore } else { WindowIcon::Full };
                        if window_button(ui, icon, &pal).on_hover_text(if full { "Small window" } else { "Full screen" }).clicked() {
                            toggle_full = true;
                        }
                        if window_button(ui, WindowIcon::Collapse, &pal).on_hover_text("Collapse to the footer").clicked() {
                            collapse = true;
                        }
                    });
                });
                ui.separator();
                let actions_height = 36.0;
                let scroll_height = (ui.available_height() - actions_height).max(80.0);
                egui::ScrollArea::vertical().id_salt("compose_scroll").max_height(scroll_height).auto_shrink([false, false]).show(ui, |ui| {
                    let field_width = (ui.available_width() - 70.0).max(120.0);
                    egui::Grid::new("compose_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label("From");
                        let account_before = d.account_id;
                        egui::ComboBox::from_id_salt("from")
                            .width(field_width)
                            .selected_text(accounts.iter().find(|a| a.0 == d.account_id).map(|a| a.1.clone()).unwrap_or_default())
                            .show_ui(ui, |ui| {
                                for (id, name) in &accounts {
                                    ui.selectable_value(&mut d.account_id, *id, name.as_str());
                                }
                            });
                        if d.account_id != account_before {
                            // Each mailbox brings its own signature.
                            d.signature_id = self.store.signature_for_new(d.account_id);
                        }
                        ui.end_row();
                        ui.label("To");
                        ui.add(TextEdit::singleline(&mut d.to).desired_width(field_width));
                        ui.end_row();
                        ui.label("Cc");
                        ui.add(TextEdit::singleline(&mut d.cc).desired_width(field_width));
                        ui.end_row();
                        ui.label("Subject");
                        ui.add(TextEdit::singleline(&mut d.subject).desired_width(field_width));
                        ui.end_row();
                    });
                    ui.add_space(4.0);
                    ui.horizontal_wrapped(|ui| {
                        ui.label(RichText::new("✨ +AI").strong());
                        ui.menu_button("Improve", |ui| {
                            for cmd in ai::WRITING_COMMANDS {
                                if ui.button(cmd).clicked() {
                                    self.ai_busy = true;
                                    let plain = if d.format == BodyFormat::Plain { d.body.clone() } else { RichDoc::from_html(&d.body).text };
                                    let (s, text, draft) = (self.store.settings.clone(), plain, d.id);
                                    self.spawn(move || Msg::Writing { draft, text: ai::writing_command(&s, cmd, &text) });
                                    ui.close();
                                }
                            }
                        });
                        if let Some(r) = d.reply_to {
                            ui.add(TextEdit::singleline(&mut tab.prompt).hint_text("What should it say?").desired_width(200.0));
                            if ui.button("Write reply").clicked() {
                                self.ai_busy = true;
                                if let Some(e) = self.store.email(r).cloned() {
                                    let (s, h) = (self.store.settings.clone(), tab.prompt.clone());
                                    self.spawn(move || Msg::Reply { email: r, text: ai::generate_reply(&s, &e, &h) });
                                }
                            }
                        }
                        if self.ai_busy {
                            ui.spinner();
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Format").size(12.0).color(pal.muted));
                        for (label, f) in [("Plain text", BodyFormat::Plain), ("HTML", BodyFormat::Html)] {
                            if ui.selectable_label(d.format == f, label).clicked() && d.format != f {
                                d.body = richtext::convert_body(&d.body, d.format, f);
                                d.format = f;
                                tab.source = false;
                            }
                        }
                        if d.format == BodyFormat::Html {
                            ui.toggle_value(&mut tab.source, "Source").on_hover_text("See or edit the HTML code");
                        }
                    });
                    // In full screen the text box takes the room left over.
                    let box_height = (scroll_height - 330.0).max(150.0);
                    let editor_id = format!("compose_body_{}", d.id);
                    if d.format == BodyFormat::Plain {
                        ui.add_sized([ui.available_width(), box_height], TextEdit::multiline(&mut d.body));
                    } else if tab.source {
                        ui.add_sized([ui.available_width(), box_height], TextEdit::multiline(&mut d.body).code_editor().hint_text("<p>Write HTML here</p>"));
                    } else {
                        rich_editor(ui, &editor_id, &mut d.body, &mut tab.selection, box_height, &pal);
                    }
                    // Signature: its own part of the message, so it can be swapped at any time.
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("Signature").size(12.0).color(pal.muted));
                        let current = d.signature_id.and_then(|id| self.store.signature(id)).map(|s| s.name.clone()).unwrap_or_else(|| "No Signature".into());
                        egui::ComboBox::from_id_salt("compose_signature").selected_text(current).show_ui(ui, |ui| {
                            ui.selectable_value(&mut d.signature_id, None, "No Signature");
                            for s in &self.store.signatures {
                                ui.selectable_value(&mut d.signature_id, Some(s.id), s.name.as_str());
                            }
                        });
                    });
                    if let Some(sig) = d.signature_id.and_then(|id| self.store.signature(id)) {
                        let job = richdoc::preview_job(&sig.html_body(), ui.available_width(), pal.muted, pal.muted, pal.accent);
                        ui.label(job);
                    }
                    if !d.quote.is_empty() {
                        let lines = d.quote.lines().count();
                        ui.label(RichText::new(format!("Quoted text included ({lines} lines)")).size(12.0).color(pal.muted));
                    }
                    if !d.attachments.is_empty() {
                        let mut remove = None;
                        for (i, a) in d.attachments.iter().enumerate() {
                            ui.horizontal(|ui| {
                                let name = std::path::Path::new(a).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                                ui.label(format!("📎 {name}"));
                                if ui.small_button("✖").clicked() {
                                    remove = Some(i);
                                }
                            });
                        }
                        if let Some(i) = remove {
                            d.attachments.remove(i);
                        }
                    }
                });
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("Send").color(Color32::WHITE).strong()).fill(pal.accent)).clicked() {
                        send = true;
                    }
                    ui.menu_button("Send Later", |ui| {
                        let now = Local::now().with_timezone(&crate::infrastructure::timezone::zone(&self.store.settings));
                        let kind = ScheduleKind::SendLater;
                        for preset in Preset::list(kind).into_iter().chain(Preset::more(kind)) {
                            let Some(t) = preset.time(now) else { continue };
                            if ui.button(format!("{}   {}", preset.label(), crate::infrastructure::timezone::format_timestamp(&self.store.settings, t))).clicked() {
                                later = Some(t);
                                ui.close();
                            }
                        }
                    });
                    if ui.button("📎 Attach").clicked() {
                        if let Some(files) = rfd::FileDialog::new().pick_files() {
                            match crate::infrastructure::portable_files::stage_draft_attachments(&Store::data_dir(), &files) {
                                Ok(imported) => {
                                    d.attachments.extend(imported.paths);
                                    if let Some(id) = imported.lease_id { self.attachment_imports.push((d.id, id)); }
                                }
                                Err(error) => self.status = error.message,
                            }
                        }
                    }
                    if ui.button("Save and close").clicked() {
                        save = true;
                    }
                    if ui.button("Discard").clicked() {
                        self.store.drafts.retain(|x| x.id != d.id);
                        self.dirty = true;
                        keep = false;
                    }
                });
            }));
        if send {
            if mail::split_addrs(&tab.draft.to).is_empty() {
                self.status = "Add at least one recipient.".into();
                self.tabs.items.insert(at, tab);
            } else {
                self.send_draft(tab.draft);
            }
            return;
        }
        if let Some(t) = later {
            tab.draft.send_at = Some(t);
            self.status = format!("Will send {} while Qamille is open", crate::infrastructure::timezone::format_timestamp(&self.store.settings, t));
            self.upsert_draft(tab.draft);
            return;
        }
        if save {
            self.upsert_draft(tab.draft);
            self.status = "Draft saved".into();
            return;
        }
        if !keep {
            return;
        }
        self.tabs.items.insert(at, tab);
        if close {
            self.close_tab(at);
        } else if collapse {
            self.tabs.toggle(at);
            self.save_open_tabs();
        } else if toggle_full {
            self.tabs.toggle_full(at);
        }
    }

    /// The tabs in the footer: click to open or collapse, the cross to save and close.
    fn footer_tabs(&mut self, ui: &mut egui::Ui) {
        let pal = self.pal();
        let count = self.tabs.items.len();
        if count == 0 {
            return;
        }
        let width = ((ui.available_width() * 0.7 - 6.0 * count as f32) / count as f32).clamp(84.0, 190.0);
        let mut toggled = None;
        let mut closed = None;
        for (i, t) in self.tabs.items.iter().enumerate() {
            let (rect, resp) = ui.allocate_exact_size(vec2(width, 24.0), Sense::click());
            let open = t.state != TabState::Collapsed;
            let fill = if open {
                pal.selected
            } else if resp.hovered() {
                pal.hover
            } else {
                pal.bg
            };
            ui.painter().rect(rect, CornerRadius::same(5), fill, Stroke::new(1.0, if open { pal.accent } else { pal.border }), egui::StrokeKind::Inside);
            let c = rect.left_center() + vec2(11.0, 0.0);
            let tri = if open {
                vec![c + vec2(-3.5, -2.5), c + vec2(3.5, -2.5), c + vec2(0.0, 3.0)]
            } else {
                vec![c + vec2(-3.5, 2.5), c + vec2(3.5, 2.5), c + vec2(0.0, -3.0)]
            };
            ui.painter().add(egui::Shape::convex_polygon(tri, if open { pal.accent } else { pal.muted }, Stroke::NONE));
            let text_area = egui::Rect::from_min_max(rect.min + vec2(20.0, 0.0), rect.max - vec2(22.0, 0.0));
            ui.painter_at(text_area).text(text_area.left_center(), Align2::LEFT_CENTER, t.title(), FontId::proportional(12.0), pal.text);
            let close_rect = egui::Rect::from_center_size(rect.right_center() - vec2(12.0, 0.0), vec2(18.0, 18.0));
            let x = ui.interact(close_rect, egui::Id::new(("footer_tab_close", t.draft.id)), Sense::click());
            draw_window_icon(ui, close_rect, WindowIcon::Close, if x.hovered() { pal.danger } else { pal.muted });
            if x.on_hover_text("Save and close").clicked() {
                closed = Some(i);
            } else if resp.on_hover_text(t.title()).clicked() {
                toggled = Some(i);
            }
        }
        if let Some(i) = closed {
            self.close_tab(i);
        } else if let Some(i) = toggled {
            self.tabs.toggle(i);
            self.save_open_tabs();
        }
        ui.add_space(6.0);
    }

    fn attachment_window(&mut self, ctx: &egui::Context) {
        let Some((name, result)) = self.attachment_view.clone() else { return };
        let mut open = true;
        egui::Window::new(name).id(egui::Id::new("attachment")).open(&mut open).default_size([560.0, 460.0]).show(ctx, |ui| match &result {
            Ok(text) => {
                ui.horizontal(|ui| {
                    if ui.button("✨ Summarize").clicked() {
                        self.ai_busy = true;
                        let (s, t) = (self.store.settings.clone(), text.clone());
                        self.spawn(move || Msg::Summary(ai::summarize(&s, "Short summary", &t)));
                    }
                    if self.ai_busy {
                        ui.spinner();
                    }
                });
                if !self.attachment_summary.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&self.attachment_summary).strong()).wrap());
                    ui.separator();
                }
                egui::ScrollArea::vertical().id_salt("att_scroll").show(ui, |ui| {
                    ui.add(egui::Label::new(text.as_str()).selectable(true).wrap());
                });
            }
            Err(e) => {
                ui.label(e);
            }
        });
        if !open {
            self.attachment_view = None;
        }
    }

    // ---------- Calendar ----------

    fn calendar_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let zone = crate::infrastructure::timezone::zone(&self.store.settings);
        let now = chrono::Utc::now().with_timezone(&zone);
        let today = now.date_naive();
        let locale = crate::infrastructure::timezone::resolved_locale(&self.store.settings);
        let weekday_offset = if locale.eq_ignore_ascii_case("en-US") { today.weekday().num_days_from_sunday() } else { today.weekday().num_days_from_monday() };
        let week_start = today - Duration::days(weekday_offset as i64) + Duration::weeks(self.week_offset);
        egui::Frame::new().inner_margin(Margin::same(16)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{}", week_start.format("%B %Y"))).size(18.0).strong());
                ui.add_space(10.0);
                if ui.button("◀").clicked() {
                    self.week_offset -= 1;
                }
                if ui.button("Today").clicked() {
                    self.week_offset = 0;
                }
                if ui.button("▶").clicked() {
                    self.week_offset += 1;
                }
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    if ui.add(egui::Button::new(RichText::new("+ New event").color(Color32::WHITE)).fill(p.accent)).clicked() {
                        self.event_form = Some(EventForm {
                            id: None,
                            title: String::new(),
                            date: today.format("%Y-%m-%d").to_string(),
                            time: format!("{:02}:00", (now.hour() + 1).min(23)),
                            minutes: "30".into(),
                            location: String::new(),
                            attendees: String::new(),
                            link: String::new(),
                        });
                    }
                });
            });
            ui.add_space(6.0);
            let details = 280.0;
            let (rect, _) = ui.allocate_exact_size(vec2((ui.available_width() - details).max(420.0), ui.available_height()), Sense::hover());
            let (first_hour, last_hour) = (7i32, 21i32);
            let header = 28.0;
            let gutter = 42.0;
            let col = (rect.width() - gutter) / 7.0;
            let hour_h = (rect.height() - header - 10.0) / (last_hour - first_hour) as f32;
            let painter = ui.painter_at(rect);
            for h in first_hour..=last_hour {
                let y = rect.top() + header + (h - first_hour) as f32 * hour_h;
                painter.line_segment([pos2(rect.left() + gutter, y), pos2(rect.right(), y)], Stroke::new(1.0, p.border));
                painter.text(pos2(rect.left() + gutter - 6.0, y), Align2::RIGHT_CENTER, format!("{h:02}"), FontId::proportional(11.0), p.muted);
            }
            for i in 0..7 {
                let day = week_start + Duration::days(i);
                let x = rect.left() + gutter + i as f32 * col;
                painter.line_segment([pos2(x, rect.top() + header), pos2(x, rect.bottom())], Stroke::new(1.0, p.border));
                let is_today = day == today;
                if is_today {
                    painter.rect_filled(egui::Rect::from_min_size(pos2(x, rect.top() + header), vec2(col, rect.height() - header)), 0.0, p.selected.gamma_multiply(0.5));
                }
                painter.text(
                    pos2(x + col / 2.0, rect.top() + 12.0),
                    Align2::CENTER_CENTER,
                    day.format("%a %d").to_string(),
                    FontId::proportional(12.5),
                    if is_today { p.accent } else { p.muted },
                );
            }
            let events = self.store.events.clone();
            for ev in &events {
                let Some(start) = zone.timestamp_opt(ev.start, 0).single() else { continue };
                let offset = (start.date_naive() - week_start).num_days();
                if !(0..7).contains(&offset) {
                    continue;
                }
                let sh = start.hour() as f32 + start.minute() as f32 / 60.0;
                let wall_seconds = zone.timestamp_opt(ev.end, 0).single().map(|end| (end.naive_local() - start.naive_local()).num_seconds()).unwrap_or(ev.end - ev.start);
                let dur = (wall_seconds as f32 / 3600.0).max(0.4);
                let top = rect.top() + header + (sh - first_hour as f32).max(0.0) * hour_h;
                let height = (dur * hour_h).max(20.0);
                let x = rect.left() + gutter + offset as f32 * col;
                let r = egui::Rect::from_min_size(pos2(x + 2.0, top + 1.0), vec2(col - 4.0, height - 2.0)).intersect(rect);
                let c = Color32::from_rgb(ev.color[0], ev.color[1], ev.color[2]);
                let resp = ui.interact(r, egui::Id::new(("event", ev.id)), Sense::click());
                painter.rect_filled(r, CornerRadius::same(4), c.gamma_multiply(if self.selected_event == Some(ev.id) { 1.0 } else { 0.85 }));
                let inner = ui.painter_at(r);
                inner.text(r.left_top() + vec2(5.0, 3.0), Align2::LEFT_TOP, &ev.title, FontId::proportional(12.0), Color32::WHITE);
                inner.text(r.left_top() + vec2(5.0, 18.0), Align2::LEFT_TOP, start.format("%H:%M").to_string(), FontId::proportional(10.5), Color32::WHITE);
                if resp.clicked() {
                    self.selected_event = Some(ev.id);
                }
            }
            let side = egui::Rect::from_min_max(pos2(rect.right() + 14.0, rect.top()), pos2(ui.max_rect().right(), rect.bottom()));
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(side).layout(Layout::top_down(Align::LEFT)));
            self.event_details(&mut child);
        });
    }

    fn event_details(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        let Some(id) = self.selected_event else {
            ui.label(RichText::new("Select an event to see its details, join its meeting or record it.").color(p.muted));
            return;
        };
        let Some(ev) = self.store.events.iter().find(|e| e.id == id).cloned() else { return };
        let remote = mcp::calendar::is_remote_projection(id).unwrap_or(true);
        if remote { ui.label("Remote calendar event. Use Qamille desktop to edit or respond."); }
        ui.add(egui::Label::new(RichText::new(&ev.title).size(16.0).strong()).wrap());
        ui.label(RichText::new(crate::infrastructure::timezone::format_timestamp(&self.store.settings, ev.start)).color(p.muted));
        if !ev.location.is_empty() {
            ui.label(format!("📍 {}", ev.location));
        }
        for a in &ev.attendees {
            ui.label(RichText::new(format!("👤 {a}")).size(12.0));
        }
        if let Some(link) = ev.meeting_link.clone().filter(|l| !l.is_empty()) {
            if ui.add(egui::Button::new(RichText::new("🎥 Join meeting").color(Color32::WHITE)).fill(p.accent)).clicked() {
                if let Err(e) = open::that(&link) {
                    self.status = e.to_string();
                }
            }
            ui.add(egui::Label::new(RichText::new(link).size(11.0).color(p.muted)).wrap());
        }
        ui.horizontal(|ui| {
            for (label, value) in [("Accept", Rsvp::Accepted), ("Maybe", Rsvp::Maybe), ("Decline", Rsvp::Declined)] {
                if ui.add_enabled(!remote, egui::Button::new(label).selected(ev.rsvp == value)).clicked() {
                    if let Some(x) = self.store.events.iter_mut().find(|e| e.id == id) {
                        x.rsvp = value;
                    }
                    self.dirty = true;
                }
            }
        });
        if ui.button("🎤 Record this meeting").clicked() {
            self.start_recording(ev.title.clone(), Some(id));
            self.view = View::Meetings;
        }
        if !ev.attendees.is_empty() && ui.button("✉ Email attendees").clicked() {
            let mut d = self.new_draft();
            d.to = ev.attendees.join(", ");
            d.subject = ev.title.clone();
            d.body = format!(
                "{}\nWhen: {}\n{}\n{}",
                ev.title,
                crate::infrastructure::timezone::format_timestamp(&self.store.settings, ev.start),
                if ev.location.is_empty() { String::new() } else { format!("Where: {}", ev.location) },
                ev.meeting_link.clone().map(|l| format!("Join: {l}")).unwrap_or_default()
            );
            d.body = richtext::plain_to_html(&d.body);
            self.open_composer(d);
        }
        ui.horizontal(|ui| {
            if ui.add_enabled(!remote, egui::Button::new("Edit")).clicked() {
                let zone = crate::infrastructure::timezone::zone(&self.store.settings);
                let start = zone.timestamp_opt(ev.start, 0).single().unwrap_or_else(|| chrono::Utc::now().with_timezone(&zone));
                self.event_form = Some(EventForm {
                    id: Some(id),
                    title: ev.title.clone(),
                    date: start.format("%Y-%m-%d").to_string(),
                    time: start.format("%H:%M").to_string(),
                    minutes: ((ev.end - ev.start) / 60).to_string(),
                    location: ev.location.clone(),
                    attendees: ev.attendees.join(", "),
                    link: ev.meeting_link.clone().unwrap_or_default(),
                });
            }
            if ui.add_enabled(!remote, egui::Button::new("Delete")).clicked() {
                self.store.events.retain(|e| e.id != id);
                self.selected_event = None;
                self.dirty = true;
            }
        });
    }

    fn event_window(&mut self, ctx: &egui::Context) {
        let Some(mut f) = self.event_form.take() else { return };
        if f.id.is_some_and(|id| mcp::calendar::is_remote_projection(id).unwrap_or(true)) {
            self.status = "Use Qamille desktop to modify remote calendar events.".into();
            return;
        }
        let mut keep = true;
        let mut save = false;
        egui::Window::new(if f.id.is_some() { "Edit event" } else { "New event" }).id(egui::Id::new("event_form")).collapsible(false).show(ctx, |ui| {
            egui::Grid::new("event_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                ui.label("Title");
                ui.add(TextEdit::singleline(&mut f.title).desired_width(320.0));
                ui.end_row();
                ui.label("Date");
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut f.date).desired_width(100.0));
                    ui.label("at");
                    ui.add(TextEdit::singleline(&mut f.time).desired_width(60.0));
                    ui.label("for");
                    ui.add(TextEdit::singleline(&mut f.minutes).desired_width(40.0));
                    ui.label("min");
                });
                ui.end_row();
                ui.label("");
                if ui.button("Find a free time").clicked() {
                    let zone = crate::infrastructure::timezone::zone(&self.store.settings);
                    let day = NaiveDate::parse_from_str(&f.date, "%Y-%m-%d").map_err(|_| "Use YYYY-MM-DD".to_owned())
                        .and_then(|day| crate::domain::regional::day_start(day, zone.name()));
                    match day {
                        Ok(day) => {
                            let len = f.minutes.parse::<i64>().unwrap_or(30).clamp(5, 1440) * 60;
                            match mcp::free_slots(&self.store, day, len).first() {
                                Some((s, _)) => f.time = zone.timestamp_opt(*s, 0).single().map(|t| t.format("%H:%M").to_string()).unwrap_or_default(),
                                None => self.status = "No free time on that day".into(),
                            }
                        }
                        Err(error) => self.status = error,
                    }
                }
                ui.end_row();
                ui.label("Where");
                ui.add(TextEdit::singleline(&mut f.location).desired_width(320.0));
                ui.end_row();
                ui.label("Attendees");
                ui.add(TextEdit::singleline(&mut f.attendees).desired_width(320.0).hint_text("Emails, separated by commas"));
                ui.end_row();
                ui.label("Online");
                ui.add(TextEdit::singleline(&mut f.link).desired_width(320.0).hint_text("Meeting link"));
                ui.end_row();
                ui.label("");
                ui.horizontal(|ui| {
                    if ui.button("Start a Google Meet").on_hover_text("Opens a new Meet in your browser. Copy its link into the box above.").clicked() {
                        if let Err(e) = open::that("https://meet.google.com/new") {
                            self.status = e.to_string();
                        }
                    }
                    if ui.button("Create a link for me").on_hover_text("A free Jitsi room, no account needed").clicked() {
                        f.link = mcp::meeting_link(&f.title, self.store.next_id + 1);
                    }
                });
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button("Save").clicked() {
                    save = true;
                }
                if ui.button("Cancel").clicked() {
                    keep = false;
                }
            });
        });
        if save {
            let zone = crate::infrastructure::timezone::zone(&self.store.settings);
            let start = chrono::NaiveDateTime::parse_from_str(&format!("{}T{}", f.date, f.time), "%Y-%m-%dT%H:%M")
                .map_err(|_| "Check the date (YYYY-MM-DD) and time (HH:mm).".to_owned())
                .and_then(|local| crate::domain::regional::local_timestamp(local, zone.name()));
            match (start, f.title.trim().is_empty()) {
                (Ok(start), false) => {
                    let mins = f.minutes.trim().parse::<i64>().unwrap_or(30).clamp(5, 1440);
                    let id = f.id.unwrap_or_else(|| self.store.new_id());
                    let existing = self.store.events.iter().find(|e| e.id == id).cloned().unwrap_or_default();
                    let ev = Event {
                        id,
                        title: f.title.trim().to_string(),
                        start,
                        end: start + mins * 60,
                        location: f.location.clone(),
                        attendees: mail::split_addrs(&f.attendees),
                        meeting_link: Some(f.link.trim().to_string()).filter(|l| !l.is_empty()),
                        conference: String::new(),
                        color: if f.id.is_some() { existing.color } else { [47, 111, 237] },
                        rsvp: existing.rsvp,
                        notes: existing.notes,
                    };
                    self.store.events.retain(|e| e.id != id);
                    self.store.events.push(ev);
                    self.selected_event = Some(id);
                    self.dirty = true;
                    return;
                }
                (Err(error), _) => self.status = error,
                (_, true) => self.status = "Give the event a title.".into(),
            }
        }
        if keep {
            self.event_form = Some(f);
        }
    }

    // ---------- Meetings ----------

    fn start_recording(&mut self, title: String, event: Option<u64>) {
        if Store::data_dir().join("meetings").exists() {
            self.status = "Apri Qamille desktop per registrare e modificare i meeting nel nuovo archivio.".into();
            return;
        }
        if self.recorder.is_some() {
            return;
        }
        let dir = Store::data_dir().join("recordings");
        let _ = std::fs::create_dir_all(&dir);
        match Recorder::start(dir.join(format!("meeting-{}.wav", now_ts()))) {
            Ok(r) => {
                self.recorder = Some(r);
                self.recording_title = title;
                self.recording_event = event;
            }
            Err(e) => self.status = format!("Couldn't start recording: {e}"),
        }
    }

    fn finish_recording(&mut self) {
        let Some(r) = self.recorder.take() else { return };
        match r.finish() {
            Ok((path, secs)) => {
                let id = self.store.new_id();
                self.store.meetings.push(Meeting {
                    id,
                    title: if self.recording_title.is_empty() { "Ad-Hoc Meeting".into() } else { self.recording_title.clone() },
                    date: now_ts() - secs as i64,
                    duration_secs: secs,
                    audio_path: Some(path.to_string_lossy().to_string()),
                    event_id: self.recording_event,
                    ..Default::default()
                });
                self.selected_meeting = Some(id);
                self.meeting_tab = 1;
                self.dirty = true;
                self.transcribe(id, path);
            }
            Err(e) => self.status = format!("Recording failed: {e}"),
        }
    }

    fn transcribe(&mut self, meeting: u64, path: PathBuf) {
        let settings = self.store.settings.clone();
        self.status = "Transcribing…".into();
        self.spawn(move || Msg::Transcribed { meeting, result: ai::transcribe(&settings, &path) });
    }

    fn meetings_panel(&mut self, ui: &mut egui::Ui) {
        if Store::data_dir().join("meetings").exists() {
            ui.heading("Meetings");
            ui.label("Archivio aggiornato: registrazione e modifiche sono disponibili in Qamille desktop.");
            egui::ScrollArea::vertical().show(ui, |ui| {
                for meeting in &self.store.meetings {
                    ui.separator();
                    ui.heading(&meeting.title);
                    ui.label(&meeting.summary);
                    ui.label(&meeting.notes);
                }
            });
            return;
        }
        let p = self.pal();
        egui::Frame::new().inner_margin(Margin::same(16)).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Meetings").size(18.0).strong());
                ui.add_space(12.0);
                if self.recorder.is_none() {
                    ui.add(TextEdit::singleline(&mut self.recording_title).hint_text("Meeting name").desired_width(180.0));
                    if ui.add(egui::Button::new(RichText::new("🎤 Start recording").color(Color32::WHITE)).fill(p.danger)).clicked() {
                        let t = self.recording_title.clone();
                        self.start_recording(t, None);
                    }
                }
            });
            if let Some(r) = &self.recorder {
                let (paused, secs, level) = (r.is_paused(), r.seconds(), r.level());
                let mut toggle = false;
                let mut finish = false;
                ui.ctx().request_repaint_after(std::time::Duration::from_millis(150));
                egui::Frame::new().fill(p.selected).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(10)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(if paused { "⏸ Paused" } else { "⏺ Recording" }).strong().color(p.danger));
                        ui.label(format!("{:02}:{:02}", secs / 60, secs % 60));
                        ui.add(egui::ProgressBar::new(level).desired_width(120.0));
                        if ui.button(if paused { "Resume" } else { "Pause" }).clicked() {
                            toggle = true;
                        }
                        if ui.button("Finish").clicked() {
                            finish = true;
                        }
                    });
                });
                if toggle {
                    if let Some(r) = &self.recorder {
                        r.set_paused(!paused);
                    }
                }
                if finish {
                    self.finish_recording();
                }
            }
            ui.add_space(6.0);
            let meetings = self.store.meetings.clone();
            ui.columns(2, |cols| {
                egui::ScrollArea::vertical().id_salt("meet_list").show(&mut cols[0], |ui| {
                    if meetings.is_empty() {
                        ui.label(RichText::new("No meetings yet").color(p.muted));
                    }
                    for m in &meetings {
                        let text = format!("{}\n{} · {} min", m.title, crate::infrastructure::timezone::format_timestamp(&self.store.settings, m.date), m.duration_secs / 60);
                        if ui.selectable_label(self.selected_meeting == Some(m.id), text).clicked() {
                            self.selected_meeting = Some(m.id);
                        }
                    }
                });
                let ui = &mut cols[1];
                let Some(id) = self.selected_meeting else {
                    ui.label(RichText::new("Select a meeting").color(p.muted));
                    return;
                };
                let Some(pos) = self.store.meetings.iter().position(|m| m.id == id) else { return };
                let m = self.store.meetings[pos].clone();
                ui.label(RichText::new(&m.title).size(16.0).strong());
                ui.horizontal(|ui| {
                    for (i, t) in ["Summary", "Transcript", "Notes"].iter().enumerate() {
                        if ui.selectable_label(self.meeting_tab == i as u8, *t).clicked() {
                            self.meeting_tab = i as u8;
                        }
                    }
                    if let Some(a) = &m.audio_path {
                        if ui.small_button("▶ Listen").clicked() {
                            if let Err(e) = open::that(a) {
                                self.status = e.to_string();
                            }
                        }
                        if ui.small_button("Transcribe").clicked() {
                            self.transcribe(id, PathBuf::from(a));
                        }
                    }
                    if ui.small_button("✨ Summarize").clicked() {
                        self.ai_busy = true;
                        let s = self.store.settings.clone();
                        let (t, n) = (m.transcript.clone(), m.notes.clone());
                        self.spawn(move || {
                            let (summary, key_points, action_items) = ai::summarize_meeting(&s, &t, &n);
                            Msg::MeetingNotes { meeting: id, summary, key_points, action_items }
                        });
                    }
                });
                ui.separator();
                egui::ScrollArea::vertical().id_salt("meet_body").show(ui, |ui| match self.meeting_tab {
                    0 => {
                        ui.add(egui::Label::new(m.summary.as_str()).wrap());
                        ui.add_space(6.0);
                        ui.label(RichText::new("Key Points").strong());
                        for k in &m.key_points {
                            ui.add(egui::Label::new(format!("• {k}")).wrap());
                        }
                        ui.add_space(6.0);
                        ui.label(RichText::new("Action Items").strong());
                        for k in &m.action_items {
                            ui.add(egui::Label::new(format!("☐ {k}")).wrap());
                        }
                    }
                    1 => {
                        ui.add(egui::Label::new(if m.transcript.is_empty() { "No transcript yet." } else { m.transcript.as_str() }).selectable(true).wrap());
                    }
                    _ => {
                        if ui.add_sized([ui.available_width(), 260.0], TextEdit::multiline(&mut self.store.meetings[pos].notes)).changed() {
                            self.dirty = true;
                        }
                    }
                });
            });
        });
    }

    // ---------- Assistant ----------

    fn assistant_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        egui::Frame::new().inner_margin(Margin::same(18)).show(ui, |ui| {
            ui.label(RichText::new("Assistant").size(18.0).strong());
            ui.label(
                RichText::new("Ask about your mail and attachments. It finds the right passages on this computer, then the AI you chose in Settings writes the answer.")
                    .size(12.5)
                    .color(p.muted),
            );
            ui.add_space(6.0);
            let input_h = 44.0;
            let mut go = None;
            egui::ScrollArea::vertical().id_salt("chat").auto_shrink([false, false]).max_height(ui.available_height() - input_h).stick_to_bottom(true).show(ui, |ui| {
                for (q, a, sources) in &self.chat {
                    ui.label(RichText::new(q).strong().color(p.accent));
                    ui.add(egui::Label::new(a.as_str()).selectable(true).wrap());
                    ui.horizontal_wrapped(|ui| {
                        for id in sources {
                            if let Some(e) = self.store.email(*id) {
                                if ui.small_button(truncate(&e.subject, 34)).clicked() {
                                    go = Some(*id);
                                }
                            }
                        }
                    });
                    ui.add_space(10.0);
                }
                if self.asking {
                    ui.spinner();
                }
            });
            if let Some(id) = go {
                self.view = View::Inbox;
                self.open_mail(id);
            }
            ui.horizontal(|ui| {
                let r = ui.add(TextEdit::singleline(&mut self.question).hint_text("e.g. How much is the October invoice?").desired_width(ui.available_width() - 70.0).margin(Margin::symmetric(10, 6)));
                let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                if ui.button("Ask").clicked() || enter {
                    self.ask();
                }
            });
        });
    }

    // ---------- Settings ----------

    fn settings_panel(&mut self, ui: &mut egui::Ui) {
        let p = self.pal();
        egui::ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| {
            egui::Frame::new().inner_margin(Margin::same(20)).show(ui, |ui| {
                ui.label(RichText::new("Settings").size(18.0).strong());
                ui.add_space(6.0);
                ui.set_max_width(680.0);
                card(ui, &p, "Google accounts", |ui| self.settings_accounts(ui, &p));
                card(ui, &p, "Signatures", |ui| self.settings_signatures(ui, &p));
                card(ui, &p, "Semantic search on this computer", |ui| self.settings_rag(ui, &p));
                card(ui, &p, "AI", |ui| self.settings_ai(ui, &p));
                card(ui, &p, "AI assistants (MCP server)", |ui| self.settings_mcp(ui, &p));
                card(ui, &p, "You", |ui| {
                    egui::Grid::new("you_grid").num_columns(2).show(ui, |ui| {
                        ui.label("Name");
                        self.dirty |= ui.text_edit_singleline(&mut self.store.settings.user_name).changed();
                        ui.end_row();
                    });
                    if ui.checkbox(&mut self.store.settings.dark, "Dark theme").changed() {
                        self.dirty = true;
                        self.apply_theme();
                    }
                });
            });
        });
    }

    fn settings_accounts(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let accounts = self.store.accounts.clone();
        let mut remove = None;
        for a in &accounts {
            ui.horizontal(|ui| {
                let kind = if a.kind == AccountKind::Sample { "sample mail".to_string() } else { a.last_sync.map(|t| format!("synced {}", crate::infrastructure::timezone::format_timestamp(&self.store.settings, t))).unwrap_or_else(|| "not synced yet".into()) };
                ui.label(format!("{} <{}>", a.name, a.email));
                ui.label(RichText::new(kind).size(12.0).color(p.muted));
                if a.kind == AccountKind::Gmail {
                    let how = if a.oauth_client_id.is_empty() { "app password" } else { "signed in with Google" };
                    ui.label(RichText::new(how).size(12.0).color(p.muted));
                }
                if a.kind == AccountKind::Gmail && a.password.is_empty() {
                    ui.label(RichText::new("needs to sign in again").size(12.0).color(p.danger));
                }
                if a.kind == AccountKind::Gmail && !a.password.is_empty() && !self.checking_login && ui.small_button("Test sign-in").clicked() {
                    self.checking_login = true;
                    self.login_result = None;
                    let acc = a.clone();
                    self.spawn(move || {
                        let result = mail::check_login(&acc);
                        Msg::LoginChecked { account: acc, is_new: false, result }
                    });
                }
                if ui.small_button("Remove").clicked() {
                    remove = Some(a.id);
                }
            });
        }
        if let Some(id) = remove {
            if let Err(error) = crate::infrastructure::mailbox_access::retire_legacy(&mut self.store, id) {
                self.status = format!("Could not remove the mailbox: {error}");
                return;
            }
            self.selected = None;
            self.dirty = true;
            self.start_indexing();
        }
        ui.separator();
        ui.label(RichText::new("Add a Gmail or Google Workspace account").strong());
        let can_sign_in = !self.store.settings.google_client_id.trim().is_empty() && !self.store.settings.google_client_secret.is_empty();
        if self.checking_login {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Waiting for Google. Finish in your browser…").color(p.muted));
                if ui.small_button("Stop").clicked() {
                    self.oauth_stop.store(true, Ordering::Relaxed);
                }
            });
        } else if can_sign_in {
            let google = egui::Button::new(RichText::new("Sign in with Google").color(Color32::WHITE).strong()).fill(p.accent);
            if ui.add(google).clicked() {
                self.start_oauth();
            }
            ui.label(RichText::new("Your browser opens Google's page. Qamille never sees your password. Qamille keeps only a sign-in token, in its portable data folder.").size(12.0).color(p.muted));
        } else {
            ui.label(RichText::new("Google requires every app to have its own project, and Qamille doesn't have a shared one yet. Create yours once, about 10 minutes:").size(12.5));
            for step in [
                "1. Open Google Cloud Console and create a project.",
                "2. Turn on the Gmail API for it, plus the People API if you want Google contacts.",
                "3. Set up the OAuth consent screen and add your Google address as a test user.",
                "4. Create an OAuth client of type Desktop app.",
                "5. Paste its client ID and secret below.",
            ] {
                ui.label(RichText::new(step).size(12.5).color(p.muted));
            }
            ui.hyperlink_to("Open Google Cloud Console", oauth::CONSOLE_URL);
            ui.label(RichText::new("While the project is in Testing mode, Google asks you to sign in again every 7 days.").size(12.0).color(p.muted));
        }
        egui::CollapsingHeader::new("Google project").default_open(!can_sign_in).show(ui, |ui| {
            egui::Grid::new("google_project").num_columns(2).show(ui, |ui| {
                ui.label("Client ID");
                self.dirty |= ui.add(TextEdit::singleline(&mut self.store.settings.google_client_id).desired_width(320.0)).changed();
                ui.end_row();
                ui.label("Client secret");
                let hint = if self.store.settings.google_client_secret.is_empty() { "Paste the secret" } else { "Saved. Paste to replace" };
                ui.horizontal(|ui| {
                    ui.add(TextEdit::singleline(&mut self.client_secret_input).password(true).hint_text(hint).desired_width(230.0));
                    if ui.button("Save").clicked() {
                        match secret::set(oauth::CLIENT_SECRET_KEY, self.client_secret_input.trim()) {
                            Ok(()) => {
                                self.store.settings.google_client_secret = self.client_secret_input.trim().to_string();
                                self.client_secret_input.clear();
                            }
                            Err(e) => self.login_result = Some(Err(format!("Couldn't store the secret safely: {e}"))),
                        }
                    }
                });
                ui.end_row();
            });
        });
        egui::CollapsingHeader::new("Use an app password instead").show(ui, |ui| {
            ui.label(RichText::new("Needs 2-Step Verification. Create the password on Google's page and paste it here; it is tested first, then stored in Qamille's portable data folder.").size(12.0).color(p.muted));
            ui.hyperlink_to("Create an app password", mail::APP_PASSWORD_HELP);
            egui::Grid::new("acct_grid").num_columns(2).show(ui, |ui| {
                ui.label("Name");
                ui.add(TextEdit::singleline(&mut self.new_account.name).hint_text("Work").desired_width(280.0));
                ui.end_row();
                ui.label("Google address");
                ui.add(TextEdit::singleline(&mut self.new_account.email).hint_text("you@gmail.com").desired_width(280.0));
                ui.end_row();
                ui.label("App password");
                ui.add(TextEdit::singleline(&mut self.new_account.password).password(true).desired_width(280.0));
                ui.end_row();
            });
            if !self.checking_login && ui.button("Sign in and add").clicked() {
                self.add_account();
            }
        });
        match &self.login_result {
            Some(Ok(t)) => {
                ui.label(RichText::new(format!("✔ {t}")).color(p.accent));
            }
            Some(Err(t)) => {
                ui.add(egui::Label::new(RichText::new(t).color(p.danger)).wrap());
            }
            None => {}
        }
    }

    /// Signs in for real first; the account and its password are kept only if Google accepts them.
    fn add_account(&mut self) {
        let f = &self.new_account;
        self.login_result = None;
        if !mail::looks_like_email(&f.email) {
            self.login_result = Some(Err("Enter a full Google address.".into()));
            return;
        }
        if f.password.trim().is_empty() {
            self.login_result = Some(Err("Paste the app password from Google.".into()));
            return;
        }
        let existing = self.store.accounts.iter().find(|a| a.email.eq_ignore_ascii_case(f.email.trim()));
        let id = existing.map_or(0, |a| a.id);
        let color = existing.map(|a| a.color).unwrap_or(ACCOUNT_COLORS[id as usize % ACCOUNT_COLORS.len()]);
        let acc = mail::gmail_account(id, &f.name, &f.email, &f.password, color);
        self.checking_login = true;
        self.spawn(move || {
            let result = mail::check_login(&acc);
            Msg::LoginChecked { account: acc, is_new: true, result }
        });
    }

    /// Runs Google's sign-in in the browser, then checks it works for receiving and for sending.
    fn start_oauth(&mut self) {
        self.login_result = None;
        self.checking_login = true;
        self.oauth_stop.store(false, Ordering::Relaxed);
        let (client_id, client_secret) = (self.store.settings.google_client_id.trim().to_string(), self.store.settings.google_client_secret.clone());
        let stop = self.oauth_stop.clone();
        let login_hint = self.new_account.email.trim().to_string();
        let known: Vec<(u32, String, [u8; 3])> = self.store.accounts.iter().map(|a| (a.id, a.email.clone(), a.color)).collect();
        self.spawn(move || {
            let (email, tokens) = match oauth::sign_in(&client_id, &client_secret, &stop, &login_hint) {
                Ok(x) => x,
                Err(e) => return Msg::LoginChecked { account: Account::default(), is_new: true, result: Err(e) },
            };
            let existing = known.iter().find(|k| k.1.eq_ignore_ascii_case(&email));
            let id = existing.map_or(0, |k| k.0);
            let color = existing.map(|k| k.2).unwrap_or(ACCOUNT_COLORS[id as usize % ACCOUNT_COLORS.len()]);
            let account = mail::oauth_account(id, &email, &client_id, &tokens.refresh, color);
            let result = mail::check_login(&account);
            Msg::LoginChecked { account, is_new: true, result }
        });
    }

    fn finish_add_account(&mut self, mut acc: Account) {
        if let Err(e) = secret::set(&secret::account_key(&acc.email), &acc.password) {
            self.login_result = Some(Err(format!("Sign-in works, but the password couldn't be stored safely, so the account wasn't added: {e}")));
            return;
        }
        // The first real account replaces the sample mail.
        let sample: Vec<u32> = self.store.accounts.iter().filter(|a| a.kind == AccountKind::Sample).map(|a| a.id).collect();
        for id in sample { self.store.detach_mailbox(id, now_ts()); }
        acc.id = match self.store.accounts.iter().find(|a| a.resolved_provider_id() == "gmail" && a.email.eq_ignore_ascii_case(&acc.email)) {
            Some(existing) => existing.id,
            None => match self.store.allocate_mailbox_id() {
                Some(id) => { acc.color = ACCOUNT_COLORS[id as usize % ACCOUNT_COLORS.len()]; id }
                None => {
                    self.login_result = Some(Err("Mailbox identifiers exhausted".into()));
                    return;
                }
            },
        };
        // Adding a known address again just renews its password.
        self.store.accounts.retain(|a| a.id != acc.id);
        self.store.accounts.push(acc.clone());
        self.new_account = NewAccount::default();
        self.selected = None;
        self.dirty = true;
        self.login_result = Some(Ok(format!("{}: sign-in works. Downloading your mail…", acc.email)));
        self.sync_account(acc);
    }

    fn settings_signatures(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let list: Vec<(u64, String)> = self.store.signatures.iter().map(|s| (s.id, s.name.clone())).collect();
        ui.label(RichText::new("All signatures").strong());
        if list.is_empty() {
            ui.label(RichText::new("No signatures yet.").size(12.5).color(p.muted));
        }
        let mut delete = None;
        for (id, name) in &list {
            ui.horizontal(|ui| {
                ui.label(name);
                if ui.small_button("Edit").clicked() {
                    self.signature_edit = self.store.signature(*id).map(|x| Signature { body: x.html_body(), html: true, ..x.clone() });
                    self.signature_sel = (0, 0);
                }
                if ui.small_button("Delete").clicked() {
                    delete = Some(*id);
                }
            });
        }
        if let Some(id) = delete {
            self.store.remove_signature(id);
            if self.signature_edit.as_ref().is_some_and(|s| s.id == id) {
                self.signature_edit = None;
            }
            self.dirty = true;
        }
        let mut close = false;
        let mut save = false;
        if let Some(e) = self.signature_edit.as_mut() {
            egui::Frame::new().stroke(Stroke::new(1.0, p.border)).corner_radius(CornerRadius::same(6)).inner_margin(Margin::same(10)).show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Name");
                    ui.add(TextEdit::singleline(&mut e.name).desired_width(240.0));
                });
                rich_editor(ui, "signature_body", &mut e.body, &mut self.signature_sel, 90.0, p);
                egui::CollapsingHeader::new("HTML").show(ui, |ui| {
                    ui.add(egui::Label::new(RichText::new(&e.body).monospace().size(11.0)).wrap());
                });
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        } else if ui.button("Create Signature").clicked() {
            let id = self.store.new_id();
            let name = format!("Signature {}", list.len() + 1);
            self.signature_edit = Some(Signature { id, name, html: true, ..Default::default() });
            self.signature_sel = (0, 0);
            self.dirty = true;
        }
        if save {
            if let Some(mut e) = self.signature_edit.take() {
                e.modified = now_ts();
                self.store.signatures.retain(|s| s.id != e.id);
                self.store.signatures.push(e);
                self.store.signatures.sort_by_key(|s| s.id);
                self.dirty = true;
            }
        }
        if close {
            self.signature_edit = None;
        }
        ui.add_space(6.0);
        ui.label(RichText::new("Default signatures").strong());
        ui.label(RichText::new("Each mailbox starts its messages with its own signature. You can still change it in any message.").size(12.0).color(p.muted));
        let accounts: Vec<(u32, String, String, SignatureBinding)> = self.store.accounts.iter().map(|a| (a.id, a.name.clone(), a.email.clone(), a.signature)).collect();
        for (id, name, email, binding) in accounts {
            ui.horizontal(|ui| {
                ui.label(format!("{name} <{email}>"));
                let current = match binding {
                    SignatureBinding::MostRecent => "Most Recent".to_string(),
                    SignatureBinding::None => "No Signature".to_string(),
                    SignatureBinding::Id(s) => self.store.signature(s).map(|x| x.name.clone()).unwrap_or_else(|| "Most Recent".into()),
                };
                let mut chosen = binding;
                egui::ComboBox::from_id_salt(("account_signature", id)).selected_text(current).show_ui(ui, |ui| {
                    ui.selectable_value(&mut chosen, SignatureBinding::MostRecent, "Most Recent");
                    ui.selectable_value(&mut chosen, SignatureBinding::None, "No Signature");
                    for (sid, sname) in &list {
                        ui.selectable_value(&mut chosen, SignatureBinding::Id(*sid), sname.as_str());
                    }
                });
                if chosen != binding {
                    if let Some(a) = self.store.accounts.iter_mut().find(|a| a.id == id) {
                        a.signature = chosen;
                    }
                    self.dirty = true;
                }
            });
        }
    }

    fn settings_rag(&mut self, ui: &mut egui::Ui, p: &Palette) {
        ui.label(
            RichText::new("Finds mail and attachments by what they mean, also across languages. It uses EmbeddingGemma 2 in its smallest version (4-bit) on the CPU. Nothing leaves this computer.")
                .size(12.5)
                .color(p.muted),
        );
        let st = self.rag_status.lock().map(|s| s.clone()).unwrap_or_default();
        if st.busy {
            ui.add(egui::ProgressBar::new(st.fraction).text(st.message.clone()));
            if ui.button("Stop").clicked() {
                self.rag_stop.store(true, Ordering::Relaxed);
            }
            return;
        }
        if !st.message.is_empty() {
            ui.label(st.message);
        }
        if self.model_ready {
            ui.label(format!("Model installed. {} passages from {} emails.", self.index.passages.len(), self.index.email_count()));
            if ui.button("Update now").clicked() {
                self.start_indexing();
            }
        } else {
            ui.label("Semantic search runs on a Qamille server: connect one in Settings. Word search works without it.");
        }
    }

    fn settings_ai(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let s = &mut self.store.settings;
        egui::ComboBox::from_label("Who writes the answers").selected_text(s.ai_provider.label()).show_ui(ui, |ui| {
            for v in [AiProvider::Offline, AiProvider::Anthropic, AiProvider::OpenAiCompatible, AiProvider::Ollama] {
                if ui.selectable_value(&mut s.ai_provider, v, v.label()).clicked() {
                    self.dirty = true;
                }
            }
        });
        if s.ai_provider != AiProvider::Offline {
            egui::Grid::new("ai_grid").num_columns(2).show(ui, |ui| {
                ui.label("Model");
                self.dirty |= ui.add(TextEdit::singleline(&mut s.ai_model).hint_text(ai::default_model(s.ai_provider)).desired_width(280.0)).changed();
                ui.end_row();
                ui.label("Web address");
                self.dirty |= ui.add(TextEdit::singleline(&mut s.ai_base_url).hint_text("Leave empty for the default").desired_width(280.0)).changed();
                ui.end_row();
                if s.ai_provider != AiProvider::Ollama {
                    ui.label("Key");
                    ui.horizontal(|ui| {
                        ui.add(TextEdit::singleline(&mut self.ai_key_input).password(true).hint_text(if s.ai_key.is_empty() { "Paste your key" } else { "Saved. Paste to replace" }).desired_width(200.0));
                        if ui.button("Save key").clicked() {
                            match secret::set(secret::AI_KEY, self.ai_key_input.trim()) {
                                Ok(()) => {
                                    s.ai_key = self.ai_key_input.trim().to_string();
                                    self.ai_key_input.clear();
                                }
                                Err(e) => self.status = format!("Couldn't store the key safely: {e}"),
                            }
                        }
                    });
                    ui.end_row();
                }
            });
            ui.label(RichText::new("Mail you ask about is sent to this service, unless you choose Ollama.").size(12.0).color(p.muted));
        } else {
            ui.label(RichText::new("Replies, summaries and labels use simple built-in rules until you choose an AI.").size(12.0).color(p.muted));
        }
        self.dirty |= ui.checkbox(&mut s.auto_classify, "Sort new mail automatically").changed();
        self.dirty |= ui.checkbox(&mut s.auto_tag, "Label new mail automatically").changed();
        self.dirty |= ui.checkbox(&mut s.push_labels, "Also add these labels in Gmail").changed();
    }

    fn settings_mcp(&mut self, ui: &mut egui::Ui, p: &Palette) {
        let s = &mut self.store.settings;
        ui.label(RichText::new("Lets an assistant such as Claude search your mail by meaning, read attachments and write drafts.").size(12.5).color(p.muted));
        ui.horizontal(|ui| {
            ui.label("Access");
            for v in [Access::ReadOnly, Access::Triage, Access::Send] {
                if ui.selectable_value(&mut s.mcp_access, v, v.label()).clicked() {
                    self.dirty = true;
                }
            }
        });
        let exe = std::env::current_exe().map(|e| e.to_string_lossy().replace('\\', "\\\\")).unwrap_or_default();
        let config = format!("{{\n  \"mcpServers\": {{\n    \"qamille\": {{ \"command\": \"{exe}\", \"args\": [\"--mcp\"] }}\n  }}\n}}");
        ui.add(egui::Label::new(RichText::new(&config).monospace().size(11.0)).wrap());
        if ui.button("Copy setup").clicked() {
            ui.ctx().copy_text(config);
        }
    }
}

/// A real bold typeface for the editor: the bundled font only has one weight.
fn load_bold_font(ctx: &egui::Context) {
    let Ok(bytes) = std::fs::read(r"C:\Windows\Fonts\segoeuib.ttf") else { return };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert("segoe-bold".to_owned(), Arc::new(egui::FontData::from_owned(bytes)));
    let mut family = vec!["segoe-bold".to_owned()];
    family.extend(fonts.families.get(&egui::FontFamily::Proportional).cloned().unwrap_or_default());
    fonts.families.insert(egui::FontFamily::Name(richdoc::BOLD_FAMILY.into()), family);
    ctx.set_fonts(fonts);
    richdoc::BOLD_READY.store(true, Ordering::Relaxed);
}

/// The visual editor: what you type already looks like the finished message. `text` holds its HTML.
fn rich_editor(ui: &mut egui::Ui, id: &str, text: &mut String, sel: &mut (usize, usize), height: f32, p: &Palette) {
    let edit_id = egui::Id::new(id);
    let doc_slot = ui.ctx().data_mut(|d| d.get_temp_mut_or_insert_with(edit_id.with("doc"), || Arc::new(Mutex::new(RichDoc::empty()))).clone());
    let mut guard = doc_slot.lock().unwrap_or_else(|e| e.into_inner());
    let doc = &mut *guard;
    // Reload when the HTML changed elsewhere: AI, the Source view, another draft.
    if doc.html_cache != *text {
        *doc = RichDoc::from_html(text);
    }
    let mut changed = false;
    let mut new_sel = None;
    let active = doc.style_at(*sel);
    let kind = doc.kind_at(*sel);
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().button_padding = vec2(7.0, 3.0);
        ui.spacing_mut().item_spacing.x = 3.0;
        for (label, hover, attr, on) in [
            ("B", "Bold (Ctrl+B)", Attr::Bold, active.bold),
            ("I", "Italic (Ctrl+I)", Attr::Italic, active.italic),
            ("U", "Underline (Ctrl+U)", Attr::Underline, active.underline),
            ("S", "Strikethrough", Attr::Strike, active.strike),
            ("Code", "Code", Attr::Code, active.code),
        ] {
            let mut rt = RichText::new(label).size(12.5);
            rt = match attr {
                Attr::Bold => rt.strong(),
                Attr::Italic => rt.italics(),
                Attr::Underline => rt.underline(),
                Attr::Strike => rt.strikethrough(),
                Attr::Code => rt,
            };
            if ui.add(egui::Button::new(rt).selected(on)).on_hover_text(hover).clicked() {
                doc.toggle(*sel, attr);
                changed = true;
                new_sel = Some(*sel);
            }
        }
        ui.separator();
        ui.menu_button("Heading", |ui| {
            for (label, k) in [("Heading 1", LineKind::H1), ("Heading 2", LineKind::H2), ("Heading 3", LineKind::H3), ("Normal text", LineKind::Normal)] {
                if ui.selectable_label(kind == k, label).clicked() {
                    new_sel = Some(doc.set_kind(*sel, k));
                    changed = true;
                    ui.close();
                }
            }
        });
        for (label, hover, k) in [("•", "Bullet list", LineKind::Bullet), ("1.", "Numbered list", LineKind::Numbered), ("Quote", "Quote", LineKind::Quote)] {
            if ui.add(egui::Button::new(label).selected(kind == k)).on_hover_text(hover).clicked() {
                new_sel = Some(doc.set_kind(*sel, k));
                changed = true;
            }
        }
        if ui.button("Indent").on_hover_text("Increase indent").clicked() {
            new_sel = Some(doc.indent(*sel, true));
            changed = true;
        }
        if ui.button("Outdent").on_hover_text("Decrease indent").clicked() {
            new_sel = Some(doc.indent(*sel, false));
            changed = true;
        }
        ui.separator();
        ui.menu_button("Link", |ui| {
            if sel.0 == sel.1 {
                ui.label(RichText::new("Select some text first").size(12.0).color(p.muted));
            }
            ui.add(TextEdit::singleline(&mut doc.link_input).hint_text("Paste your link here").desired_width(220.0));
            ui.horizontal(|ui| {
                if ui.button("Add link").clicked() {
                    let url = std::mem::take(&mut doc.link_input);
                    doc.set_link(*sel, &url);
                    changed = true;
                    new_sel = Some(*sel);
                    ui.close();
                }
                if ui.button("Remove").clicked() {
                    doc.set_link(*sel, "");
                    changed = true;
                    new_sel = Some(*sel);
                    ui.close();
                }
            });
        });
        ui.menu_button("Color", |ui| {
            ui.label(RichText::new("Text color").size(12.0).color(p.muted));
            for (name, hex) in richtext::TEXT_COLORS {
                if ui.button(RichText::new(name).color(color_of(hex))).clicked() {
                    doc.restyle(*sel, |s| s.color = Some(hex.into()));
                    changed = true;
                    new_sel = Some(*sel);
                    ui.close();
                }
            }
            if ui.button("Default color").clicked() {
                doc.restyle(*sel, |s| s.color = None);
                changed = true;
                new_sel = Some(*sel);
                ui.close();
            }
            ui.label(RichText::new("Highlight color").size(12.0).color(p.muted));
            for (name, hex) in richtext::HIGHLIGHT_COLORS {
                if ui.button(RichText::new(name).background_color(color_of(hex)).color(Color32::BLACK)).clicked() {
                    doc.restyle(*sel, |s| s.bg = Some(hex.into()));
                    changed = true;
                    new_sel = Some(*sel);
                    ui.close();
                }
            }
            if ui.button("No highlight").clicked() {
                doc.restyle(*sel, |s| s.bg = None);
                changed = true;
                new_sel = Some(*sel);
                ui.close();
            }
        });
        if ui.button("Clear").on_hover_text("Clear Formatting").clicked() {
            new_sel = Some(doc.clear_formatting(*sel));
            changed = true;
        }
    });
    let (normal, muted, link) = (p.text, p.muted, p.accent);
    let snapshot = doc.clone();
    let mut layouter = move |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap: f32| {
        let job = snapshot.layout(buf.as_str(), wrap, normal, muted, link);
        ui.fonts_mut(|f| f.layout_job(job))
    };
    let mut buffer = doc.text.clone();
    let out = TextEdit::multiline(&mut buffer).id(edit_id).desired_width(f32::INFINITY).min_size(vec2(0.0, height)).layouter(&mut layouter).show(ui);
    let edited = buffer != doc.text;
    if edited {
        new_sel = doc.apply_edit(&buffer).map(|c| (c, c)).or(new_sel);
        changed = true;
    }
    if let Some(r) = out.cursor_range {
        let now = (r.primary.index, r.secondary.index);
        if now != *sel && !edited && !changed {
            doc.pending = None;
        }
        *sel = now;
    }
    if out.response.has_focus() {
        for (key, attr) in [(egui::Key::B, Attr::Bold), (egui::Key::I, Attr::Italic), (egui::Key::U, Attr::Underline)] {
            if ui.input_mut(|i| i.consume_key(egui::Modifiers::COMMAND, key)) {
                doc.toggle(*sel, attr);
                changed = true;
            }
        }
    }
    if let Some((a, b)) = new_sel {
        let mut state = egui::text_edit::TextEditState::load(ui.ctx(), edit_id).unwrap_or_default();
        state.cursor.set_char_range(Some(egui::text::CCursorRange::two(egui::text::CCursor::new(a), egui::text::CCursor::new(b))));
        state.store(ui.ctx(), edit_id);
        ui.ctx().memory_mut(|m| m.request_focus(edit_id));
        *sel = (a, b);
    }
    if changed {
        // An empty message leaves nothing behind, so closing it creates no draft.
        *text = if doc.text.trim().is_empty() { String::new() } else { doc.to_html() };
        doc.html_cache = text.clone();
    }
}

fn color_of(hex: &str) -> Color32 {
    richdoc::parse_color(hex).unwrap_or(Color32::BLACK)
}

#[derive(Clone, Copy)]
enum WindowIcon {
    Collapse,
    Full,
    Restore,
    Close,
}

/// Drawn as shapes: the bundled fonts have no window-control symbols.
fn draw_window_icon(ui: &egui::Ui, rect: egui::Rect, icon: WindowIcon, color: Color32) {
    let c = rect.center();
    let s = Stroke::new(1.4, color);
    let p = ui.painter();
    match icon {
        WindowIcon::Collapse => {
            p.line_segment([c + vec2(-5.0, 4.0), c + vec2(5.0, 4.0)], s);
        }
        WindowIcon::Full => {
            p.rect_stroke(egui::Rect::from_center_size(c, vec2(10.0, 10.0)), CornerRadius::same(1), s, egui::StrokeKind::Middle);
        }
        WindowIcon::Restore => {
            p.rect_stroke(egui::Rect::from_center_size(c + vec2(-1.5, 1.5), vec2(8.0, 8.0)), CornerRadius::same(1), s, egui::StrokeKind::Middle);
            p.line_segment([c + vec2(-2.5, -4.5), c + vec2(4.5, -4.5)], s);
            p.line_segment([c + vec2(4.5, -4.5), c + vec2(4.5, 2.5)], s);
        }
        WindowIcon::Close => {
            p.line_segment([c + vec2(-4.0, -4.0), c + vec2(4.0, 4.0)], s);
            p.line_segment([c + vec2(4.0, -4.0), c + vec2(-4.0, 4.0)], s);
        }
    }
}

fn window_button(ui: &mut egui::Ui, icon: WindowIcon, p: &Palette) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(26.0, 24.0), Sense::click());
    if resp.hovered() {
        ui.painter().rect_filled(rect, CornerRadius::same(4), p.hover);
    }
    draw_window_icon(ui, rect, icon, if resp.hovered() { p.text } else { p.muted });
    resp
}

fn card(ui: &mut egui::Ui, p: &Palette, title: &str, body: impl FnOnce(&mut egui::Ui)) {
    egui::Frame::new().stroke(Stroke::new(1.0, p.border)).corner_radius(CornerRadius::same(8)).inner_margin(Margin::same(14)).show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(RichText::new(title).size(14.0).strong());
        ui.add_space(4.0);
        body(ui);
    });
    ui.add_space(10.0);
}

fn chip(ui: &mut egui::Ui, text: &str, color: Color32) {
    egui::Frame::new().fill(color.gamma_multiply(0.14)).corner_radius(CornerRadius::same(9)).inner_margin(Margin::symmetric(8, 2)).show(ui, |ui| {
        ui.label(RichText::new(text).size(11.5).color(color));
    });
}

fn truncate(s: &str, n: usize) -> String {
    if s.chars().count() > n { format!("{}…", s.chars().take(n).collect::<String>()) } else { s.to_string() }
}

fn task_group(e: &Email, now: i64, settings: &crate::model::Settings) -> &'static str {
    let zone = crate::infrastructure::timezone::zone(settings);
    let today_end = zone.timestamp_opt(now, 0).single().and_then(|time| time.date_naive().succ_opt())
        .and_then(|tomorrow| crate::domain::regional::day_start(tomorrow, zone.name()).ok())
        .map(|timestamp| timestamp - 1).unwrap_or(now);
    match e.due.or(e.reminder_at) {
        Some(t) if t < now => "Overdue",
        Some(t) if t <= today_end => "Today",
        Some(_) => "Upcoming",
        None => "No date",
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.frame += 1;
        self.drain_messages();
        self.tick();
        self.top_bar(ctx);
        self.status_bar(ctx);
        self.sidebar(ctx);
        match self.view.clone() {
            View::Calendar | View::Meetings | View::Assistant | View::Settings | View::Drafts if self.search.trim().is_empty() => {
                let p = self.pal();
                egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg)).show(ctx, |ui| match self.view {
                    View::Calendar => self.calendar_panel(ui),
                    View::Meetings => self.meetings_panel(ui),
                    View::Assistant => self.assistant_panel(ui),
                    View::Settings => self.settings_panel(ui),
                    _ => self.drafts_panel(ui),
                });
            }
            _ => {
                self.list_panel(ctx);
                let p = self.pal();
                egui::CentralPanel::default().frame(egui::Frame::new().fill(p.bg)).show(ctx, |ui| self.reading_panel(ui));
            }
        }
        self.composer_window(ctx);
        self.attachment_window(ctx);
        self.event_window(ctx);
        self.save_if_needed();
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

        if let Some(path) = self.screenshot.clone() {
            ctx.request_repaint();
            if self.frame >= SCREENSHOT_FRAME && !self.screenshot_sent {
                self.screenshot_sent = true;
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
            }
            let shot = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(img) = shot {
                if let Some(dir) = path.parent() {
                    let _ = std::fs::create_dir_all(dir);
                }
                let [w, h] = img.size;
                let bytes: Vec<u8> = img.pixels.iter().flat_map(|c| [c.r(), c.g(), c.b(), 255]).collect();
                if let Err(e) = image::save_buffer(&path, &bytes, w as u32, h as u32, image::ColorType::Rgba8) {
                    eprintln!("Couldn't save the picture: {e}");
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.rag_stop.store(true, Ordering::Relaxed);
        if self.screenshot.is_none() {
            if self.recorder.is_some() {
                self.finish_recording();
            }
            self.save_open_tabs();
            self.save_if_needed();
        }
    }
}
