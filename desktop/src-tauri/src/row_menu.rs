//! The native context menu of the mail rows (docs/aspetto-nativo.md, rule 33). The page decides
//! the entries (`src/lib/row-menu.ts`); this builds them as a Windows menu, shows it where the page
//! says and answers with the value of the entry chosen, or `None`. The coordinates follow
//! `window_shell::show_system_menu`: physical pixels of the client area (`clientX *
//! devicePixelRatio`, which carries the app zoom and the scale). The page holds no menu
//! permission: the menu is built and shown here, a new one for each opening, and released when
//! the command ends.
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{LazyLock, Mutex, MutexGuard, Once};
use std::time::{Duration, Instant};

use serde::Deserialize;
use tauri::menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Manager, PhysicalPosition, WebviewWindow};

/// One entry as the page sends it; `value: None` is a separator. `checked: Some(_)` makes it
/// an entry with a check mark.
#[derive(Debug, Clone, Deserialize)]
pub struct RowMenuEntry {
    pub value: Option<String>,
    pub label: String,
    pub enabled: bool,
    pub checked: Option<bool>,
}

/// How long the command waits, once the menu has closed, for the event of the entry chosen.
const CHOICE_WAIT: Duration = Duration::from_millis(500);

enum Answer {
    /// The index, in the page's list, of the entry chosen.
    Chosen(usize),
    /// Every event sent while the menu was open has been handled.
    Settled,
}

static NEXT_REQUEST: AtomicU64 = AtomicU64::new(1);
static WAITING: LazyLock<Mutex<HashMap<u64, Sender<Answer>>>> = LazyLock::new(Default::default);
static LISTENING: Once = Once::new();

fn waiting() -> MutexGuard<'static, HashMap<u64, Sender<Answer>>> {
    WAITING.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Shows the menu of the rows at `x`, `y` and answers the value of the entry chosen, or `None`
/// when it closes without a choice (Esc, a click elsewhere) or cannot be shown.
#[tauri::command]
pub async fn show_row_menu(window: WebviewWindow, items: Vec<RowMenuEntry>, x: f64, y: f64) -> Option<String> {
    let plan = rules::plan(&items);
    if !plan.iter().any(|line| matches!(line, rules::Line::Entry { .. })) {
        return None;
    }
    let app = window.app_handle().clone();
    // One listener for the whole run: it hands each entry chosen to the command that showed it.
    LISTENING.call_once(|| app.on_menu_event(|_, event| answer(&event)));
    let request = NEXT_REQUEST.fetch_add(1, Ordering::Relaxed);
    let (sender, receiver) = mpsc::channel();
    waiting().insert(request, sender.clone());
    let position = rules::position(x, y);
    // The menu blocks the thread that shows it until it closes: not one of the async runtime's.
    let shown = tauri::async_runtime::spawn_blocking(move || -> tauri::Result<Option<usize>> {
        let menu = build(&app, request, &plan)?;
        match position {
            Some((x, y)) => window.popup_menu_at(&menu, PhysicalPosition::new(x, y))?,
            None => window.popup_menu(&menu)?,
        }
        // On Windows the popup returns once the menu has closed, and the entry chosen is already on
        // its way to the event loop. This marker goes through the same queue behind it, so its
        // arrival means no choice is coming; the wait is bounded either way.
        let _ = app.run_on_main_thread(move || {
            let _ = sender.send(Answer::Settled);
        });
        let deadline = Instant::now() + CHOICE_WAIT;
        let chosen = match receiver.recv_timeout(deadline.saturating_duration_since(Instant::now())) {
            Ok(Answer::Chosen(index)) => Some(index),
            Ok(Answer::Settled) | Err(_) => None,
        };
        drop(menu);
        Ok(chosen)
    })
    .await
    .and_then(|result| result);
    waiting().remove(&request);
    match shown {
        Ok(chosen) => chosen.and_then(|index| items.get(index)).and_then(|entry| entry.value.clone()),
        Err(error) => {
            crate::native_feel::report(&format!("row menu not shown: {error}"));
            None
        }
    }
}

fn build(app: &AppHandle, request: u64, plan: &[rules::Line]) -> tauri::Result<Menu<tauri::Wry>> {
    let menu = Menu::new(app)?;
    for line in plan {
        match line {
            rules::Line::Separator => menu.append(&PredefinedMenuItem::separator(app)?)?,
            rules::Line::Entry { index, text, enabled, checked } => {
                let id = rules::item_id(request, *index);
                match checked {
                    Some(checked) => {
                        menu.append(&CheckMenuItem::with_id(app, id, text, *enabled, *checked, None::<&str>)?)?
                    }
                    None => menu.append(&MenuItem::with_id(app, id, text, *enabled, None::<&str>)?)?,
                }
            }
        }
    }
    Ok(menu)
}

/// On the main thread, for every menu event of the app: an entry of a row menu goes to the
/// command that showed it; any other menu's entry is not ours.
fn answer(event: &MenuEvent) {
    let Some((request, index)) = rules::parse_item_id(event.id().as_ref()) else { return };
    if let Some(sender) = waiting().get(&request) {
        let _ = sender.send(Answer::Chosen(index));
    }
}

/// What the menu shows of what the page sent, without Tauri: tested with `cargo test row_menu`.
mod rules {
    use super::RowMenuEntry;

    /// More entries than this are not a row menu: the rest is left out.
    pub const MAX_ENTRIES: usize = 32;
    const MAX_LABEL: usize = 80;
    const MAX_VALUE: usize = 64;
    const PREFIX: &str = "row-menu:";

    #[derive(Debug, PartialEq)]
    pub enum Line {
        Separator,
        /// `index` is the entry's place in the page's list, the one its value is read from.
        Entry { index: usize, text: String, enabled: bool, checked: Option<bool> },
    }

    /// The lines of the menu: entries with a value and a label, separators only between entries.
    pub fn plan(items: &[RowMenuEntry]) -> Vec<Line> {
        let mut lines = Vec::new();
        for (index, item) in items.iter().enumerate().take(MAX_ENTRIES) {
            match &item.value {
                None => {
                    if matches!(lines.last(), Some(Line::Entry { .. })) {
                        lines.push(Line::Separator);
                    }
                }
                Some(value) if valid_value(value) => {
                    if let Some(text) = menu_text(&item.label) {
                        lines.push(Line::Entry { index, text, enabled: item.enabled, checked: item.checked });
                    }
                }
                Some(_) => {}
            }
        }
        if matches!(lines.last(), Some(Line::Separator)) {
            lines.pop();
        }
        lines
    }

    fn valid_value(value: &str) -> bool {
        !value.is_empty()
            && value.len() <= MAX_VALUE
            && value.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    }

    /// The label as Windows shows it: no control characters (a tab starts the shortcut column),
    /// bounded, and `&` written `&&` so that it shows instead of marking an access key.
    pub fn menu_text(label: &str) -> Option<String> {
        let plain: String = label.chars().filter(|c| !c.is_control()).collect();
        let plain = plain.trim();
        if plain.is_empty() {
            return None;
        }
        Some(plain.chars().take(MAX_LABEL).collect::<String>().replace('&', "&&"))
    }

    /// Where the menu opens, in physical pixels of the client area; `None` (at the pointer) when
    /// the page sent no usable point.
    pub fn position(x: f64, y: f64) -> Option<(i32, i32)> {
        (x.is_finite() && y.is_finite()).then(|| (x.round() as i32, y.round() as i32))
    }

    pub fn item_id(request: u64, index: usize) -> String {
        format!("{PREFIX}{request}:{index}")
    }

    pub fn parse_item_id(id: &str) -> Option<(u64, usize)> {
        let (request, index) = id.strip_prefix(PREFIX)?.split_once(':')?;
        Some((request.parse().ok()?, index.parse().ok()?))
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn entry(value: Option<&str>, label: &str) -> RowMenuEntry {
            RowMenuEntry { value: value.map(str::to_owned), label: label.to_owned(), enabled: true, checked: None }
        }
        fn texts(lines: &[Line]) -> Vec<String> {
            lines
                .iter()
                .map(|line| match line {
                    Line::Separator => "-".to_owned(),
                    Line::Entry { text, .. } => text.clone(),
                })
                .collect()
        }

        #[test]
        fn row_menu_keeps_separators_only_between_entries() {
            let items = [
                entry(None, ""),
                entry(Some("reply"), "Rispondi"),
                entry(None, ""),
                entry(None, ""),
                entry(Some("archive"), "Archivia"),
                entry(None, ""),
            ];
            assert_eq!(texts(&plan(&items)), ["Rispondi", "-", "Archivia"]);
        }

        #[test]
        fn row_menu_keeps_the_page_index_state_and_check_mark() {
            let mut pinned = entry(Some("pin"), "Fissa");
            pinned.checked = Some(true);
            let mut locked = entry(Some("archive"), "Archivia");
            locked.enabled = false;
            let lines = plan(&[entry(None, ""), locked, pinned]);
            assert_eq!(
                lines,
                [
                    Line::Entry { index: 1, text: "Archivia".into(), enabled: false, checked: None },
                    Line::Entry { index: 2, text: "Fissa".into(), enabled: true, checked: Some(true) },
                ]
            );
        }

        #[test]
        fn row_menu_drops_what_it_cannot_show() {
            let long = "x".repeat(MAX_LABEL + 10);
            let items = [
                entry(Some(""), "Empty value"),
                entry(Some("spaced value"), "Spaces"),
                entry(Some("reply"), "  \t "),
                entry(Some("archive"), &long),
            ];
            let lines = plan(&items);
            assert_eq!(texts(&lines), ["x".repeat(MAX_LABEL)]);
            let many: Vec<_> = (0..MAX_ENTRIES + 5).map(|i| entry(Some("pin"), &format!("Entry {i}"))).collect();
            assert_eq!(plan(&many).len(), MAX_ENTRIES);
            assert!(plan(&[]).is_empty());
        }

        #[test]
        fn row_menu_label_shows_an_ampersand_and_no_control_character() {
            assert_eq!(menu_text("Ricerca & sviluppo").as_deref(), Some("Ricerca && sviluppo"));
            assert_eq!(menu_text("Archivia\tCtrl+E").as_deref(), Some("ArchiviaCtrl+E"));
            assert_eq!(menu_text("\n"), None);
        }

        #[test]
        fn row_menu_ids_carry_request_and_index() {
            assert_eq!(parse_item_id(&item_id(42, 7)), Some((42, 7)));
            assert_eq!(parse_item_id("tray:quit"), None);
            assert_eq!(parse_item_id("row-menu:x:1"), None);
            assert_eq!(parse_item_id("row-menu:1"), None);
        }

        #[test]
        fn row_menu_position_needs_a_point() {
            assert_eq!(position(120.4, 33.6), Some((120, 34)));
            assert_eq!(position(f64::NAN, 10.0), None);
            assert_eq!(position(10.0, f64::INFINITY), None);
        }
    }
}
