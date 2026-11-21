//! What the taskbar button of the main window shows (docs/aspetto-nativo.md, rule 30): the overlay
//! with the unread mail of the Inbox, the tray's tooltip, and the progress bar of a full indexing.
//! Pure: no Tauri and no I/O, so `rustc --edition 2024 --test taskbar_rules.rs` tries it alone.
//! `taskbar.rs` feeds it the events and applies what it answers.
#![cfg_attr(not(test), allow(dead_code))]

/// The text of the overlay: the count up to 9, then «9+»; nothing for no unread mail.
pub fn badge_label(count: u32) -> Option<&'static str> {
    const DIGITS: [&str; 9] = ["1", "2", "3", "4", "5", "6", "7", "8", "9"];
    match count {
        0 => None,
        1..=9 => Some(DIGITS[count as usize - 1]),
        _ => Some("9+"),
    }
}

/// The badge's colour: the red of Windows' own badges, which reads on a light and a dark taskbar.
const BADGE: [f32; 3] = [196.0, 43.0, 28.0];
/// The colour of the count.
const TEXT: [f32; 3] = [255.0, 255.0, 255.0];

/// A 3×5 glyph of the count, one row per entry, the leftmost of three bits first.
fn glyph(c: char) -> [u8; 5] {
    match c {
        '1' => [0b010, 0b110, 0b010, 0b010, 0b111],
        '2' => [0b111, 0b001, 0b111, 0b100, 0b111],
        '3' => [0b111, 0b001, 0b111, 0b001, 0b111],
        '4' => [0b101, 0b101, 0b111, 0b001, 0b001],
        '5' => [0b111, 0b100, 0b111, 0b001, 0b111],
        '6' => [0b111, 0b100, 0b111, 0b101, 0b111],
        '7' => [0b111, 0b001, 0b001, 0b010, 0b010],
        '8' => [0b111, 0b101, 0b111, 0b101, 0b111],
        '9' => [0b111, 0b101, 0b111, 0b001, 0b111],
        '+' => [0b000, 0b010, 0b111, 0b010, 0b000],
        _ => [0; 5],
    }
}

/// The overlay for `count` unread messages, `size`×`size` pixels of straight RGBA (the size of a small
/// icon at the window's scale): a round badge with the count in it. `None` takes the overlay away.
pub fn badge_rgba(count: u32, size: u32) -> Option<Vec<u8>> {
    let label = badge_label(count)?;
    let side = size as f32;
    let glyphs: Vec<[u8; 5]> = label.chars().map(glyph).collect();
    // The text in units of the glyph grid: three columns a glyph, one between glyphs, five rows; as
    // large as fits the badge, and its top-left corner on a whole pixel so its strokes stay sharp.
    let columns = (glyphs.len() * 4 - 1) as f32;
    let unit = (side * 0.6 / 5.0).min(side * 0.8 / columns).max(0.5);
    let left = ((side - columns * unit) / 2.0).round();
    let top = ((side - 5.0 * unit) / 2.0).round();
    let lit = |x: f32, y: f32| -> bool {
        let (column, row) = ((x - left) / unit, (y - top) / unit);
        if column < 0.0 || row < 0.0 || row >= 5.0 {
            return false;
        }
        let (column, row) = (column as usize, row as usize);
        let (index, bit) = (column / 4, column % 4);
        bit < 3 && glyphs.get(index).is_some_and(|rows| rows[row] & (0b100 >> bit) != 0)
    };
    let (center, radius) = (side / 2.0, side / 2.0);
    // Four by four samples a pixel: the edge of the circle and the strokes are smooth at any scale.
    const SAMPLES: usize = 4;
    let mut rgba = Vec::with_capacity((size * size * 4) as usize);
    for py in 0..size {
        for px in 0..size {
            let (mut disc, mut text) = (0usize, 0usize);
            for sy in 0..SAMPLES {
                for sx in 0..SAMPLES {
                    let x = px as f32 + (sx as f32 + 0.5) / SAMPLES as f32;
                    let y = py as f32 + (sy as f32 + 0.5) / SAMPLES as f32;
                    if (x - center).powi(2) + (y - center).powi(2) <= radius * radius {
                        disc += 1;
                        if lit(x, y) {
                            text += 1;
                        }
                    }
                }
            }
            if disc == 0 {
                rgba.extend_from_slice(&[0, 0, 0, 0]);
                continue;
            }
            // Straight alpha: the colour of the covered part, the coverage as alpha.
            let share = text as f32 / disc as f32;
            for channel in 0..3 {
                rgba.push((BADGE[channel] * (1.0 - share) + TEXT[channel] * share).round() as u8);
            }
            rgba.push((disc * 255 / (SAMPLES * SAMPLES)) as u8);
        }
    }
    Some(rgba)
}

/// The tray icon's tooltip: «Qamille», and how many messages wait to be read.
pub fn tray_tooltip(count: u32, italian: bool) -> String {
    match (count, italian) {
        (0, _) => "Qamille".to_owned(),
        (count, true) => format!("Qamille, {count} da leggere"),
        (count, false) => format!("Qamille, {count} unread"),
    }
}

/// An indexing that starts under this percent is a full one (a new account, «Reindex»); one that
/// starts higher is the mailbox keeping up with new mail.
const FULL_BELOW: u8 = 90;

/// The look of the progress bar: green while it moves, yellow while it waits for the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bar {
    Normal,
    Paused,
}

/// The full indexing of each account, as far as the taskbar shows it.
#[derive(Clone, Debug, Default)]
pub struct IndexBars {
    accounts: Vec<(u32, Account)>,
}

#[derive(Clone, Copy, Debug, Default)]
struct Account {
    /// A full indexing is shown: it began under `FULL_BELOW` and has not reached 100 yet.
    active: bool,
    /// It reached 100 in this process: a later drop is new mail being indexed, not shown.
    completed: bool,
    percent: u8,
    waiting: bool,
}

impl IndexBars {
    /// An `IndexProgress` (`percent`) or an `IndexWaiting` (`waiting`) of `account`. Answers the bar
    /// to show now for all accounts together: the lowest account's, or `None` for no bar.
    pub fn apply(&mut self, account: u32, percent: Option<u8>, waiting: Option<bool>) -> Option<(Bar, u8)> {
        let state = match self.accounts.iter().position(|(id, _)| *id == account) {
            Some(index) => &mut self.accounts[index].1,
            None => {
                self.accounts.push((account, Account::default()));
                &mut self.accounts.last_mut().expect("just pushed").1
            }
        };
        if let Some(percent) = percent.map(|percent| percent.min(100)) {
            if percent == 100 {
                state.active = false;
                state.completed = true;
            } else if state.active || (!state.completed && percent < FULL_BELOW) {
                state.active = true;
                state.percent = percent;
            }
        }
        if let Some(waiting) = waiting {
            state.waiting = waiting;
        }
        self.bar()
    }

    /// The bar of the lowest account still indexing in full, paused when that account waits.
    fn bar(&self) -> Option<(Bar, u8)> {
        let lowest = self.accounts.iter().filter(|(_, a)| a.active).map(|(_, a)| a.percent).min()?;
        let waits = self.accounts.iter().any(|(_, a)| a.active && a.percent == lowest && a.waiting);
        Some((if waits { Bar::Paused } else { Bar::Normal }, lowest))
    }

    /// The accounts whose indexing is under way now (`WorkspaceService::indexing_accounts`): every
    /// other one stops showing, whatever percent it reached (it failed, was switched off or removed,
    /// or the server unlinked it). One that had reached 100 stays finished. Answers the bar to show.
    pub fn retain(&mut self, indexing: &[u32]) -> Option<(Bar, u8)> {
        for (id, state) in &mut self.accounts {
            if !indexing.contains(id) {
                state.active = false;
                state.waiting = false;
            }
        }
        self.bar()
    }

    /// «Reindex» (`search_reindex`): what comes next is a full indexing again, also for the accounts
    /// that had reached 100.
    pub fn reset(&mut self) {
        for (_, state) in &mut self.accounts {
            state.completed = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The pixel at (x, y) of a badge.
    fn pixel(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let at = ((y * size + x) * 4) as usize;
        [rgba[at], rgba[at + 1], rgba[at + 2], rgba[at + 3]]
    }

    #[test]
    fn badge_is_none_for_zero_and_nine_plus_above_nine() {
        assert_eq!(badge_label(0), None);
        assert_eq!(badge_label(1), Some("1"));
        assert_eq!(badge_label(9), Some("9"));
        assert_eq!(badge_label(10), Some("9+"));
        assert_eq!(badge_label(4_000), Some("9+"));
        assert_eq!(badge_rgba(0, 16), None, "no unread mail: the overlay goes");
        for size in [16, 20, 24, 32] {
            let badge = badge_rgba(3, size).expect("a badge");
            assert_eq!(badge.len(), (size * size * 4) as usize, "size {size}");
            // Round: the corners are transparent, the middle is not.
            assert_eq!(pixel(&badge, size, 0, 0)[3], 0, "size {size}: corner");
            assert_eq!(pixel(&badge, size, size - 1, size - 1)[3], 0, "size {size}: corner");
            assert_eq!(pixel(&badge, size, size / 2, size / 2)[3], 255, "size {size}: middle");
            // The count is drawn in it: light pixels on the dark badge.
            assert!(badge.chunks(4).any(|p| p[3] == 255 && p[0] > 200 && p[1] > 200 && p[2] > 200), "size {size}: the digit");
        }
        assert_ne!(badge_rgba(1, 16), badge_rgba(2, 16), "each count has its own picture");
        assert_ne!(badge_rgba(9, 16), badge_rgba(10, 16));
        assert_eq!(badge_rgba(10, 16), badge_rgba(250, 16), "above nine it is «9+»");
        assert_eq!(tray_tooltip(0, true), "Qamille");
        assert_eq!(tray_tooltip(3, true), "Qamille, 3 da leggere");
        assert_eq!(tray_tooltip(3, false), "Qamille, 3 unread");
        assert_eq!(tray_tooltip(1, true), "Qamille, 1 da leggere");
    }

    #[test]
    fn index_bar_shows_only_a_full_indexing() {
        let mut bars = IndexBars::default();
        assert_eq!(bars.apply(1, Some(90), None), None, "90 % at the first announcement is the mailbox keeping up");
        assert_eq!(bars.apply(1, Some(95), None), None);
        assert_eq!(bars.apply(1, Some(10), None), Some((Bar::Normal, 10)), "under 90 % a full indexing begins");
        assert_eq!(bars.apply(1, Some(60), None), Some((Bar::Normal, 60)));
        assert_eq!(bars.apply(1, Some(92), None), Some((Bar::Normal, 92)), "once shown, it stays to the end");
        assert_eq!(bars.apply(1, Some(100), None), None, "100 % takes it away");
        assert_eq!(bars.apply(1, Some(97), None), None, "after 100 % a drop for new mail is not shown");
        assert_eq!(bars.apply(1, Some(40), None), None, "not even a deep one");
        assert_eq!(bars.apply(1, Some(100), None), None);
        bars.reset();
        assert_eq!(bars.apply(1, Some(0), None), Some((Bar::Normal, 0)), "after «Reindex» it is shown again");
        assert_eq!(bars.apply(1, Some(100), None), None);
        // An account first seen at 100 % is complete: what follows is the mailbox keeping up.
        let mut bars = IndexBars::default();
        assert_eq!(bars.apply(2, Some(100), None), None);
        assert_eq!(bars.apply(2, Some(20), None), None);
    }

    #[test]
    fn index_bar_pauses_while_waiting() {
        let mut bars = IndexBars::default();
        assert_eq!(bars.apply(1, None, Some(true)), None, "waiting before any indexing shows nothing");
        assert_eq!(bars.apply(1, Some(30), None), Some((Bar::Paused, 30)), "it still waits");
        assert_eq!(bars.apply(1, None, Some(false)), Some((Bar::Normal, 30)));
        assert_eq!(bars.apply(1, None, Some(true)), Some((Bar::Paused, 30)), "the server is busy: yellow");
        assert_eq!(bars.apply(1, Some(35), None), Some((Bar::Paused, 35)));
        assert_eq!(bars.apply(1, None, Some(false)), Some((Bar::Normal, 35)));
        assert_eq!(bars.apply(1, Some(100), None), None);
        assert_eq!(bars.apply(1, None, Some(true)), None, "a finished indexing that waits again shows nothing");
    }

    #[test]
    fn index_bar_stops_with_an_account_that_stopped_indexing() {
        let mut bars = IndexBars::default();
        assert_eq!(bars.apply(1, Some(40), None), Some((Bar::Normal, 40)));
        assert_eq!(bars.apply(2, Some(20), None), Some((Bar::Normal, 20)));
        // Account 2 failed, was switched off or removed below 100 %, or the server unlinked it (which
        // even says it no longer waits): only the accounts still indexing keep the bar.
        assert_eq!(bars.apply(2, None, Some(false)), Some((Bar::Normal, 20)));
        assert_eq!(bars.retain(&[1]), Some((Bar::Normal, 40)));
        assert_eq!(bars.retain(&[]), None, "nothing indexes: no bar");
        // An account that resumes its full indexing shows it again...
        assert_eq!(bars.apply(1, Some(41), None), Some((Bar::Normal, 41)));
        assert_eq!(bars.apply(1, Some(100), None), None);
        // ...but one that had finished stays finished: a drop for new mail is still not shown.
        assert_eq!(bars.retain(&[]), None);
        assert_eq!(bars.apply(1, Some(30), None), None);
        // A waiting account that stops no longer counts as waiting when it comes back.
        let mut bars = IndexBars::default();
        assert_eq!(bars.apply(3, Some(10), Some(true)), Some((Bar::Paused, 10)));
        assert_eq!(bars.retain(&[]), None);
        assert_eq!(bars.apply(3, Some(11), None), Some((Bar::Normal, 11)));
    }

    #[test]
    fn index_bar_takes_the_lowest_account() {
        let mut bars = IndexBars::default();
        assert_eq!(bars.apply(1, Some(50), None), Some((Bar::Normal, 50)));
        assert_eq!(bars.apply(2, Some(20), None), Some((Bar::Normal, 20)), "the lowest of the two");
        assert_eq!(bars.apply(1, Some(70), None), Some((Bar::Normal, 20)));
        assert_eq!(bars.apply(3, Some(95), None), Some((Bar::Normal, 20)), "an account keeping up is not part of it");
        // The bar says whether the account it shows waits.
        assert_eq!(bars.apply(1, None, Some(true)), Some((Bar::Normal, 20)));
        assert_eq!(bars.apply(2, None, Some(true)), Some((Bar::Paused, 20)));
        assert_eq!(bars.apply(2, Some(100), None), Some((Bar::Paused, 70)), "one finished: the other one's bar");
        assert_eq!(bars.apply(1, Some(100), None), None, "all finished");
    }
}
