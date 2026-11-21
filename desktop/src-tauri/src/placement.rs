//! Where the main window reopens (docs/aspetto-nativo.md, rule 29): the pure
//! decisions behind `window_shell.rs`. Only `std`: the tests run on any machine
//! with `rustc --edition 2024 --test src/placement.rs`, without Tauri or Windows.
//!
//! All sizes and positions are physical pixels of the virtual screen, as Tauri
//! reports them (`Monitor::work_area()`, `outer_position()`).

/// The height of the page's own title bar, in logical pixels (`--titlebar-height`
/// in the stylesheet). It is the part of the window that must stay reachable.
pub const TITLE_BAR_LOGICAL: u32 = 32;

/// What the window looked like when it last closed: the normal (not maximized)
/// rectangle, the scale factor of the monitor it was on, and whether it was
/// maximized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SavedPlacement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
    pub maximized: bool,
}

/// The work area of a connected monitor (the screen without the taskbar) and its
/// scale factor.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WorkArea {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub scale: f64,
}

/// What to apply before the first show.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Placement {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub maximized: bool,
}

/// The placement to apply now, or `None` when nothing should be applied (the
/// window then opens where the window config puts it: centred, within the work
/// area). It is kept only if the centre of the saved rectangle is in the work
/// area of a monitor that is connected now. The size follows a change of that
/// monitor's scale, is limited to its work area but never goes below the
/// minimum size, and the window is moved inside the work area, title bar first.
pub fn restore(saved: &SavedPlacement, monitors: &[WorkArea], min_logical: (u32, u32)) -> Option<Placement> {
    if saved.width == 0 || saved.height == 0 || !usable_scale(saved.scale) {
        return None;
    }
    // i64: the saved values come from a file anyone can edit, and they must not overflow.
    let centre_x = i64::from(saved.x) + i64::from(saved.width) / 2;
    let centre_y = i64::from(saved.y) + i64::from(saved.height) / 2;
    let area = monitors.iter().find(|area| {
        let (left, top) = (i64::from(area.x), i64::from(area.y));
        centre_x >= left
            && centre_x < left + i64::from(area.width)
            && centre_y >= top
            && centre_y < top + i64::from(area.height)
    })?;
    let scale = if usable_scale(area.scale) { area.scale } else { saved.scale };
    let ratio = scale / saved.scale;
    let ratio = if (ratio - 1.0).abs() < 0.001 { 1.0 } else { ratio };

    // Never larger than the work area, but never below the minimum: a work area smaller than the
    // minimum (a small screen at a large scale) loses to it, and the title bar is kept on screen below.
    let width = scaled(saved.width, ratio).min(i64::from(area.width)).max(scaled(min_logical.0, scale));
    let height = scaled(saved.height, ratio).min(i64::from(area.height)).max(scaled(min_logical.1, scale));

    let (left, top) = (i64::from(area.x), i64::from(area.y));
    let (right, bottom) = (left + i64::from(area.width), top + i64::from(area.height));
    let x = i64::from(saved.x).clamp(left, left.max(right - width));
    let y = i64::from(saved.y).clamp(top, top.max(bottom - height));
    // Whatever the height, the title bar is what the user needs to reach.
    let y = y.min(bottom - scaled(TITLE_BAR_LOGICAL, scale)).max(top);

    Some(Placement {
        x: i32::try_from(x).ok()?,
        y: i32::try_from(y).ok()?,
        width: u32::try_from(width).ok()?,
        height: u32::try_from(height).ok()?,
        maximized: saved.maximized,
    })
}

/// A scale factor Windows can really report.
fn usable_scale(scale: f64) -> bool {
    scale.is_finite() && scale > 0.0
}

/// A size scaled by `ratio`, in whole pixels (it never overflows, whatever the saved value).
fn scaled(size: u32, ratio: f64) -> i64 {
    (f64::from(size) * ratio).round().clamp(1.0, f64::from(u32::MAX)) as i64
}

/// Where Windows parks a minimized window.
const MINIMIZED_COORDINATE: i32 = -32000;

/// The rectangle to remember for a window that has this outer position, inner
/// size and scale, or `None` when it is not a rectangle to come back to: a
/// minimized window (Windows parks it at -32000,-32000), an empty one, or an
/// unusable scale.
pub fn capture(x: i32, y: i32, width: u32, height: u32, scale: f64) -> Option<SavedPlacement> {
    if width == 0 || height == 0 || !usable_scale(scale) || (x == MINIMIZED_COORDINATE && y == MINIMIZED_COORDINATE) {
        return None;
    }
    Some(SavedPlacement { x, y, width, height, scale, maximized: false })
}

/// What goes into the preferences file: the normal rectangle the window had last
/// (`normal`) with the current `maximized`. A maximized window has no rectangle
/// of its own to read, so without a tracked one the stored `previous` stays;
/// without either there is nothing to save.
pub fn to_store(
    normal: Option<SavedPlacement>,
    maximized: bool,
    previous: Option<SavedPlacement>,
) -> Option<SavedPlacement> {
    normal.or(previous).map(|rectangle| SavedPlacement { maximized, ..rectangle })
}

/// How the main window is shown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShowPath {
    /// `show()`: the window appears as it is (normal, or maximized by a previous show of this run).
    Plain,
    /// Shown maximized in one step, over the normal rectangle it comes back to when restored.
    Maximized,
}

/// The way to show the main window the next time, given what `restore` decided at startup
/// (`pending`, taken here: only the first show of a run is the saved placement's, a later restore from
/// the tray shows the window as it was). A maximized placement is shown maximized; anything else, or
/// no placement (nothing saved, or its monitor is gone), is a plain show. A hidden window is never
/// maximized: tao would set its maximized style without moving it to the monitor's bounds.
pub fn first_show(pending: &mut Option<Placement>) -> ShowPath {
    match pending.take() {
        Some(placement) if placement.maximized => ShowPath::Maximized,
        _ => ShowPath::Plain,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: (u32, u32) = (1000, 640);

    fn primary() -> WorkArea {
        WorkArea { x: 0, y: 0, width: 1920, height: 1040, scale: 1.0 }
    }

    fn saved(x: i32, y: i32, width: u32, height: u32, scale: f64, maximized: bool) -> SavedPlacement {
        SavedPlacement { x, y, width, height, scale, maximized }
    }

    /// The title bar band (`TITLE_BAR_LOGICAL` at the area's scale) and the rest of
    /// the window are inside the work area.
    fn assert_inside(placement: &Placement, area: &WorkArea) {
        let bar = (TITLE_BAR_LOGICAL as f64 * area.scale).ceil() as i64;
        let (x, y) = (placement.x as i64, placement.y as i64);
        let right = x + placement.width as i64;
        let bottom = y + placement.height as i64;
        assert!(x >= area.x as i64, "left edge {x} is off the work area");
        assert!(y >= area.y as i64, "top edge {y} is off the work area");
        assert!(
            y + bar <= area.y as i64 + area.height as i64,
            "the title bar ({bar} px from {y}) is off the work area"
        );
        assert!(right <= area.x as i64 + area.width as i64, "right edge {right} is off the work area");
        assert!(bottom <= area.y as i64 + area.height as i64, "bottom edge {bottom} is off the work area");
    }

    #[test]
    fn placement_on_a_connected_monitor_is_kept() {
        let placed = restore(&saved(120, 80, 1440, 900, 1.0, false), &[primary()], MIN).unwrap();
        assert_eq!(placed, Placement { x: 120, y: 80, width: 1440, height: 900, maximized: false });
    }

    #[test]
    fn placement_on_a_disconnected_monitor_is_dropped() {
        // It was on a second monitor to the right of the primary one, which is gone.
        assert_eq!(restore(&saved(2100, 100, 1440, 900, 1.0, false), &[primary()], MIN), None);
        // With no monitor at all, nothing either.
        assert_eq!(restore(&saved(100, 100, 1440, 900, 1.0, false), &[], MIN), None);
        // The centre is on the taskbar (outside the work area of the monitor): dropped too.
        assert_eq!(restore(&saved(100, 900, 1440, 900, 1.0, false), &[primary()], MIN), None);
        // A monitor to the left of the primary one has negative coordinates.
        let left = WorkArea { x: -1920, y: 0, width: 1920, height: 1040, scale: 1.0 };
        let kept = restore(&saved(-1800, 60, 1440, 900, 1.0, false), &[primary(), left], MIN).unwrap();
        assert_eq!((kept.x, kept.y), (-1800, 60));
        assert_eq!(restore(&saved(-1800, 60, 1440, 900, 1.0, false), &[primary()], MIN), None);
    }

    #[test]
    fn placement_partly_off_screen_keeps_the_title_bar_inside() {
        let area = primary();
        // Hanging off the top left: pulled in, the title bar is back on the screen.
        let placed = restore(&saved(-600, -100, 1440, 900, 1.0, false), &[area], MIN).unwrap();
        assert_inside(&placed, &area);
        assert_eq!((placed.x, placed.y), (0, 0));
        // Hanging off the bottom right: pulled up and left by what stuck out.
        let placed = restore(&saved(1000, 500, 1440, 900, 1.0, false), &[area], MIN).unwrap();
        assert_inside(&placed, &area);
        assert_eq!((placed.x, placed.y), (480, 140));
        // The same on a monitor whose scale is 1.5: the title bar band is 48 px there.
        let scaled = WorkArea { x: 0, y: 0, width: 2560, height: 1400, scale: 1.5 };
        let placed = restore(&saved(1200, 600, 1800, 1200, 1.5, false), &[scaled], MIN).unwrap();
        assert_inside(&placed, &scaled);
    }

    #[test]
    fn placement_follows_a_scale_change() {
        // Saved at 100 %, the monitor is at 150 % now: the same logical size.
        let scaled = WorkArea { x: 0, y: 0, width: 2560, height: 1400, scale: 1.5 };
        let placed = restore(&saved(100, 100, 1200, 800, 1.0, false), &[scaled], MIN).unwrap();
        assert_eq!((placed.width, placed.height), (1800, 1200));
        assert_inside(&placed, &scaled);
        // ...and then limited to the work area.
        let small = WorkArea { x: 0, y: 0, width: 1600, height: 1000, scale: 1.5 };
        let placed = restore(&saved(100, 100, 1200, 800, 1.0, false), &[small], MIN).unwrap();
        assert_eq!((placed.width, placed.height), (1600, 1000));
        assert_inside(&placed, &small);
        // Saved at 150 %, the monitor is at 100 % now: smaller by the same ratio.
        let placed = restore(&saved(100, 100, 2160, 1350, 1.5, false), &[primary()], MIN).unwrap();
        assert_eq!((placed.width, placed.height), (1440, 900));
    }

    #[test]
    fn placement_never_goes_below_the_minimum_size() {
        let placed = restore(&saved(100, 100, 400, 300, 1.0, false), &[primary()], MIN).unwrap();
        assert_eq!((placed.width, placed.height), (1000, 640));
        // The minimum is logical: at 150 % it is 1500 x 960 physical pixels.
        let scaled = WorkArea { x: 0, y: 0, width: 2560, height: 1400, scale: 1.5 };
        let placed = restore(&saved(100, 100, 600, 450, 1.5, false), &[scaled], MIN).unwrap();
        assert_eq!((placed.width, placed.height), (1500, 960));
        // On a work area smaller than the minimum the minimum wins, from the top left corner, so the
        // title bar is still on the screen.
        let tiny = WorkArea { x: 0, y: 0, width: 800, height: 600, scale: 1.0 };
        let placed = restore(&saved(50, 40, 700, 500, 1.0, false), &[tiny], MIN).unwrap();
        assert_eq!((placed.width, placed.height), (1000, 640));
        assert_eq!((placed.x, placed.y), (0, 0));
    }

    #[test]
    fn placement_keeps_maximized() {
        let placed = restore(&saved(120, 80, 1440, 900, 1.0, true), &[primary()], MIN).unwrap();
        assert_eq!(placed, Placement { x: 120, y: 80, width: 1440, height: 900, maximized: true });
        // Maximized on a monitor that is gone: nothing is applied, and it does not open maximized either.
        assert_eq!(restore(&saved(2100, 100, 1440, 900, 1.0, true), &[primary()], MIN), None);
    }

    #[test]
    fn placement_bigger_than_the_work_area_shrinks_to_it() {
        let placed = restore(&saved(-500, -300, 3000, 2000, 1.0, false), &[primary()], MIN).unwrap();
        assert_eq!(placed, Placement { x: 0, y: 0, width: 1920, height: 1040, maximized: false });
    }

    #[test]
    fn placement_goes_to_the_monitor_under_its_centre() {
        let right = WorkArea { x: 1920, y: 0, width: 2560, height: 1400, scale: 1.25 };
        let monitors = [primary(), right];
        // Straddling the two monitors, with the centre on the second one.
        let placed = restore(&saved(1700, 100, 1200, 800, 1.0, false), &monitors, MIN).unwrap();
        // 1200 x 800 at 100 % is 1500 x 1000 at 125 %, inside the second monitor.
        assert_eq!((placed.width, placed.height), (1500, 1000));
        assert_inside(&placed, &right);
        // Straddling them, with the centre on the first one: it stays on the first.
        let placed = restore(&saved(1200, 100, 1200, 800, 1.0, false), &monitors, MIN).unwrap();
        assert_eq!((placed.width, placed.height), (1200, 800));
        assert_inside(&placed, &primary());
    }

    #[test]
    fn placement_with_nonsense_values_is_dropped() {
        let area = [primary()];
        assert_eq!(restore(&saved(100, 100, 0, 900, 1.0, false), &area, MIN), None);
        assert_eq!(restore(&saved(100, 100, 1440, 0, 1.0, false), &area, MIN), None);
        assert_eq!(restore(&saved(100, 100, 1440, 900, 0.0, false), &area, MIN), None);
        assert_eq!(restore(&saved(100, 100, 1440, 900, -1.0, false), &area, MIN), None);
        assert_eq!(restore(&saved(100, 100, 1440, 900, f64::NAN, false), &area, MIN), None);
        assert_eq!(restore(&saved(100, 100, 1440, 900, f64::INFINITY, false), &area, MIN), None);
        // A huge value never overflows.
        let huge = saved(i32::MAX, i32::MAX, u32::MAX, u32::MAX, 1.0, false);
        assert_eq!(restore(&huge, &area, MIN), None);
        let wide = saved(-2_000_000_000, 0, u32::MAX, u32::MAX, 1.0, false);
        assert_eq!(restore(&wide, &area, MIN), None);
        // A monitor with a broken scale or no area is not a monitor to come back to.
        let broken = WorkArea { scale: f64::NAN, ..primary() };
        let placed = restore(&saved(100, 100, 1440, 900, 1.0, false), &[broken], MIN).unwrap();
        assert_eq!((placed.width, placed.height), (1440, 900));
        let empty = WorkArea { width: 0, ..primary() };
        assert_eq!(restore(&saved(100, 100, 1440, 900, 1.0, false), &[empty], MIN), None);
    }

    #[test]
    fn capture_keeps_a_normal_rectangle() {
        assert_eq!(capture(120, 80, 1440, 900, 1.25), Some(saved(120, 80, 1440, 900, 1.25, false)));
        // Left of or above the primary monitor is fine.
        assert_eq!(capture(-1800, -40, 1440, 900, 1.0), Some(saved(-1800, -40, 1440, 900, 1.0, false)));
    }

    #[test]
    fn capture_drops_a_window_that_is_not_on_screen() {
        // Minimized: Windows parks the window at -32000,-32000.
        assert_eq!(capture(-32000, -32000, 160, 28, 1.0), None);
        assert_eq!(capture(-32000, -32000, 1440, 900, 1.0), None);
        assert_eq!(capture(100, 100, 0, 900, 1.0), None);
        assert_eq!(capture(100, 100, 1440, 0, 1.0), None);
        assert_eq!(capture(100, 100, 1440, 900, 0.0), None);
        assert_eq!(capture(100, 100, 1440, 900, f64::NAN), None);
    }

    #[test]
    fn to_store_takes_the_tracked_rectangle_and_the_current_state() {
        let normal = saved(120, 80, 1440, 900, 1.0, false);
        assert_eq!(to_store(Some(normal), false, None), Some(normal));
        assert_eq!(to_store(Some(normal), true, None), Some(SavedPlacement { maximized: true, ..normal }));
        // The tracked rectangle wins over the stored one.
        let old = saved(10, 10, 1000, 640, 1.0, true);
        assert_eq!(to_store(Some(normal), false, Some(old)), Some(normal));
    }

    #[test]
    fn to_store_keeps_the_stored_rectangle_of_a_maximized_window() {
        // It started maximized and was never restored: the rectangle to come back to is the stored one.
        let old = saved(10, 10, 1000, 640, 1.0, false);
        assert_eq!(to_store(None, true, Some(old)), Some(SavedPlacement { maximized: true, ..old }));
        assert_eq!(to_store(None, false, Some(SavedPlacement { maximized: true, ..old })), Some(old));
        // With no rectangle known at all there is nothing to save.
        assert_eq!(to_store(None, true, None), None);
        assert_eq!(to_store(None, false, None), None);
    }

    #[test]
    fn the_first_show_of_a_maximized_placement_is_maximized() {
        let mut pending = restore(&saved(120, 80, 1440, 900, 1.0, true), &[primary()], MIN);
        assert_eq!(first_show(&mut pending), ShowPath::Maximized);
    }

    #[test]
    fn the_first_show_of_a_normal_placement_or_none_is_plain() {
        let mut normal = restore(&saved(120, 80, 1440, 900, 1.0, false), &[primary()], MIN);
        assert_eq!(first_show(&mut normal), ShowPath::Plain);
        assert_eq!(first_show(&mut None), ShowPath::Plain);
        // Maximized on a monitor that is gone: nothing was decided, so the window is shown as it is.
        let mut gone = restore(&saved(2100, 100, 1440, 900, 1.0, true), &[primary()], MIN);
        assert_eq!(first_show(&mut gone), ShowPath::Plain);
    }

    #[test]
    fn only_the_first_show_is_the_saved_placements() {
        // A start in the tray, then a restore, then the tray again: the second restore shows the window
        // as it is (the user may have restored it to its normal size meanwhile).
        let mut pending = restore(&saved(120, 80, 1440, 900, 1.0, true), &[primary()], MIN);
        assert_eq!(first_show(&mut pending), ShowPath::Maximized);
        assert_eq!(pending, None);
        assert_eq!(first_show(&mut pending), ShowPath::Plain);
    }
}
