//! Messages being written, as tabs in the footer. Each tab is collapsed, or open as a small window
//! in the corner ("Pip" in the original), or full screen ("Full"). One is open at a time.

use crate::model::Draft;

/// Original's layout table, renderer modules 644363 and 737885.
pub const PIP_MIN_WIDTH: f32 = 550.0;
pub const PIP_MIN_HEIGHT: f32 = 395.0;
pub const PIP_PADDING: i8 = 12;
pub const FULL_PADDING: i8 = 80;
pub const FULL_COMPACT_PADDING: i8 = 40;
/// Below this width the full screen editor uses the compact margin.
pub const FULL_COMPACT_BELOW: f32 = 900.0; // estimated
pub const AUTOSAVE_SECS: i64 = 30; // estimated

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TabState {
    Collapsed,
    Pip,
    Full,
}

#[derive(Clone, Debug)]
pub struct Tab {
    pub draft: Draft,
    pub state: TabState,
    pub selection: (usize, usize),
    /// Showing the HTML code instead of the visual editor.
    pub source: bool,
    pub prompt: String,
}

impl Tab {
    pub fn title(&self) -> String {
        let s = self.draft.subject.trim();
        if !s.is_empty() {
            s.to_string()
        } else if self.draft.reply_to.is_some() {
            "Reply".to_string()
        } else {
            "New message".to_string()
        }
    }
}

/// Nothing typed yet: closing it leaves no draft behind.
pub fn is_empty_draft(d: &Draft) -> bool {
    d.to.trim().is_empty() && d.cc.trim().is_empty() && d.subject.trim().is_empty() && d.body.trim().is_empty() && d.attachments.is_empty() && d.quote.is_empty()
}

#[derive(Default)]
pub struct Tabs {
    pub items: Vec<Tab>,
}

impl Tabs {
    /// Opens a draft; if it already has a tab, that tab comes forward.
    pub fn open(&mut self, draft: Draft) {
        let at = match self.items.iter().position(|t| t.draft.id == draft.id) {
            Some(i) => i,
            None => {
                self.items.push(Tab { draft, state: TabState::Collapsed, selection: (0, 0), source: false, prompt: String::new() });
                self.items.len() - 1
            }
        };
        self.show(at, TabState::Pip);
    }

    fn show(&mut self, at: usize, state: TabState) {
        for (i, t) in self.items.iter_mut().enumerate() {
            if i == at {
                t.state = state;
            } else {
                t.state = TabState::Collapsed;
            }
        }
    }

    /// Clicking a tab in the footer opens it, or collapses it when it's open.
    pub fn toggle(&mut self, at: usize) {
        match self.items.get(at).map(|t| t.state) {
            Some(TabState::Collapsed) => self.show(at, TabState::Pip),
            Some(_) => self.items[at].state = TabState::Collapsed,
            None => {}
        }
    }

    /// The full screen button: full screen, or back to the small window.
    pub fn toggle_full(&mut self, at: usize) {
        match self.items.get(at).map(|t| t.state) {
            Some(TabState::Full) => self.show(at, TabState::Pip),
            Some(_) => self.show(at, TabState::Full),
            None => {}
        }
    }

    pub fn open_index(&self) -> Option<usize> {
        self.items.iter().position(|t| t.state != TabState::Collapsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(id: u64) -> Draft {
        Draft { id, ..Default::default() }
    }

    #[test]
    fn opening_a_second_message_collapses_the_first() {
        let mut t = Tabs::default();
        t.open(d(1));
        t.open(d(2));
        assert_eq!(t.items[0].state, TabState::Collapsed);
        assert_eq!(t.items[1].state, TabState::Pip);
        assert_eq!(t.open_index(), Some(1));
    }

    #[test]
    fn the_same_draft_never_gets_two_tabs() {
        let mut t = Tabs::default();
        t.open(d(1));
        t.open(d(2));
        t.open(d(1));
        assert_eq!(t.items.len(), 2);
        assert_eq!(t.open_index(), Some(0));
    }

    #[test]
    fn clicking_a_tab_opens_and_collapses_it() {
        let mut t = Tabs::default();
        t.open(d(1));
        t.toggle(0);
        assert_eq!(t.open_index(), None);
        t.toggle(0);
        assert_eq!(t.items[0].state, TabState::Pip);
        t.toggle(9);
    }

    #[test]
    fn full_screen_goes_and_comes_back() {
        let mut t = Tabs::default();
        t.open(d(1));
        t.open(d(2));
        t.toggle_full(0);
        assert_eq!(t.items[0].state, TabState::Full);
        assert_eq!(t.items[1].state, TabState::Collapsed);
        t.toggle_full(0);
        assert_eq!(t.items[0].state, TabState::Pip);
    }

    #[test]
    fn titles_and_empty_drafts() {
        let mut t = Tabs::default();
        t.open(d(1));
        assert_eq!(t.items[0].title(), "New message");
        t.items[0].draft.reply_to = Some(4);
        assert_eq!(t.items[0].title(), "Reply");
        t.items[0].draft.subject = "Ciao".into();
        assert_eq!(t.items[0].title(), "Ciao");
        assert!(is_empty_draft(&d(1)));
        assert!(!is_empty_draft(&Draft { body: "x".into(), ..Default::default() }));
    }
}
