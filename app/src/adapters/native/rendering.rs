//! egui rendering kept outside the domain.
use crate::domain::richdoc::*;
use eframe::egui::text::{LayoutJob, TextFormat};
use eframe::egui::{Color32, FontFamily, FontId, Stroke};
use std::sync::atomic::{AtomicBool, Ordering};
pub const BOLD_FAMILY: &str = "qamille-bold";
pub static BOLD_READY: AtomicBool = AtomicBool::new(false);
pub fn parse_color(c: &str) -> Option<Color32> {
    let h = c.strip_prefix('#')?;
    let v = |s: &str| u8::from_str_radix(s, 16).ok();
    match h.len() {
        6 => Some(Color32::from_rgb(v(h.get(0..2)?)?, v(h.get(2..4)?)?, v(h.get(4..6)?)?)),
        3 => Some(Color32::from_rgb(v(h.get(0..1)?)? * 17, v(h.get(1..2)?)? * 17, v(h.get(2..3)?)? * 17)),
        _ => None,
    }
}


impl RichDoc {
    // ---------- Drawing ----------

    pub fn layout(&self, text: &str, wrap: f32, normal: Color32, muted: Color32, link: Color32) -> LayoutJob {
        let (chars, lines, ..) = self.synced(text);
        draw(text, &chars, &lines, wrap, normal, muted, link)
    }
}

/// Draws finished HTML the way it will look, for signatures shown under a message.
pub fn preview_job(html: &str, wrap: f32, normal: Color32, muted: Color32, link: Color32) -> LayoutJob {
    let d = RichDoc::from_html(html);
    draw(&d.text, &d.chars, &d.lines, wrap, normal, muted, link)
}

fn draw(text: &str, chars: &[Style], lines: &[LineKind], wrap: f32, normal: Color32, muted: Color32, link: Color32) -> LayoutJob {
    let mut job = LayoutJob::default();
    job.wrap.max_width = wrap;
    let cs: Vec<char> = text.chars().collect();
    let bold_ready = BOLD_READY.load(Ordering::Relaxed);
    let mut line = 0;
    let mut i = 0;
    while i < cs.len() {
        let kind = lines.get(line).copied().unwrap_or_default();
        let st = chars.get(i).cloned().unwrap_or_default();
        let mut j = i + 1;
        while j < cs.len() && cs[j - 1] != '\n' && chars.get(j) == Some(&st) {
            j += 1;
        }
        let run: String = cs[i..j].iter().collect();
        let size = match kind {
            LineKind::H1 => 24.0,
            LineKind::H2 => 20.0,
            LineKind::H3 => 17.0,
            _ => 14.0,
        };
        let heavy = st.bold || matches!(kind, LineKind::H1 | LineKind::H2 | LineKind::H3);
        let family = if st.code {
            FontFamily::Monospace
        } else if heavy && bold_ready {
            FontFamily::Name(BOLD_FAMILY.into())
        } else {
            FontFamily::Proportional
        };
        let color = if st.link.is_some() {
            link
        } else {
            st.color.as_deref().and_then(parse_color).unwrap_or(if kind == LineKind::Quote { muted } else { normal })
        };
        let mut f = TextFormat { font_id: FontId::new(size, family), color, italics: st.italic, ..Default::default() };
        if heavy && !bold_ready {
            f.extra_letter_spacing = 0.4;
        }
        if st.underline || st.link.is_some() {
            f.underline = Stroke::new(1.0, color);
        }
        if st.strike {
            f.strikethrough = Stroke::new(1.0, color);
        }
        if let Some(bg) = st.bg.as_deref().and_then(parse_color) {
            f.background = bg;
        }
        job.append(&run, 0.0, f);
        if cs[j - 1] == '\n' {
            line += 1;
        }
        i = j;
    }
    if job.sections.is_empty() {
        job.append("", 0.0, TextFormat { font_id: FontId::proportional(14.0), color: normal, ..Default::default() });
    }
    job
}


#[cfg(test)]
mod tests {
use super::*;
fn doc(html: &str) -> RichDoc { RichDoc::from_html(html) }
    #[test]
    fn drawing_covers_all_text_even_mid_edit() {
        let d = doc("<h1>Ciao</h1><div><strong>a</strong>è</div>");
        let job = d.layout("Ciao\naèxy\n\n", 300.0, Color32::BLACK, Color32::GRAY, Color32::BLUE);
        assert_eq!(job.text, "Ciao\naèxy\n\n");
        let job = d.layout("", 300.0, Color32::BLACK, Color32::GRAY, Color32::BLUE);
        assert_eq!(job.text, "");
    }
}
