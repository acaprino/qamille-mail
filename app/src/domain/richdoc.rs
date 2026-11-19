//! The visual editor's document. The text box holds exactly the text you see; each character has
//! its own style and each line its own kind, so no marks ever show. It reads and writes HTML.

use std::sync::Arc;

const INDENT: &str = "    ";
const INDENT_PX: usize = 24;

#[derive(Clone, Default, PartialEq, Debug)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strike: bool,
    pub code: bool,
    pub color: Option<Arc<str>>,
    pub bg: Option<Arc<str>>,
    pub link: Option<Arc<str>>,
}

#[derive(Clone, Copy, Default, PartialEq, Debug)]
pub enum LineKind {
    #[default]
    Normal,
    H1,
    H2,
    H3,
    Bullet,
    Numbered,
    Quote,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Attr {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
}

#[derive(Clone, Default, Debug)]
pub struct RichDoc {
    pub text: String,
    pub chars: Vec<Style>,
    /// One per line of `text`.
    pub lines: Vec<LineKind>,
    /// Style for the next typed characters, set by the toolbar with nothing selected.
    pub pending: Option<Style>,
    /// The HTML this document was last read from or written to.
    pub html_cache: String,
    pub link_input: String,
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn unescape(s: &str) -> String {
    s.replace("&nbsp;", " ").replace("&#160;", " ").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&apos;", "'").replace("&amp;", "&")
}

fn safe_url(u: &str) -> bool {
    let l = u.trim().to_lowercase();
    l.starts_with("https://") || l.starts_with("http://") || l.starts_with("mailto:")
}

fn safe_color(c: &str) -> bool {
    let h = c.strip_prefix('#').unwrap_or("");
    (h.len() == 3 || h.len() == 6) && h.chars().all(|x| x.is_ascii_hexdigit())
}

/// How many characters of a line are indent plus list mark, not the person's text.
fn lead_len(line: &[char], kind: LineKind) -> usize {
    let spaces = line.iter().take_while(|c| **c == ' ').count();
    let rest = &line[spaces..];
    let mark = match kind {
        LineKind::Bullet if rest.starts_with(&['•', ' ']) => 2,
        LineKind::Quote if rest.starts_with(&['│', ' ']) => 2,
        LineKind::Numbered => {
            let d = rest.iter().take_while(|c| c.is_ascii_digit()).count();
            if d > 0 && rest.get(d) == Some(&'.') && rest.get(d + 1) == Some(&' ') { d + 2 } else { 0 }
        }
        _ => 0,
    };
    spaces + mark
}

fn has_mark(line: &[char], kind: LineKind) -> bool {
    let spaces = line.iter().take_while(|c| **c == ' ').count();
    !matches!(kind, LineKind::Bullet | LineKind::Numbered | LineKind::Quote) || lead_len(line, kind) > spaces
}

impl RichDoc {
    pub fn empty() -> RichDoc {
        RichDoc { lines: vec![LineKind::Normal], ..Default::default() }
    }

    fn char_vec(&self) -> Vec<char> {
        self.text.chars().collect()
    }

    /// (start, end) in characters of each line, the end before its line break.
    fn line_ranges(&self) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut start = 0;
        let cs = self.char_vec();
        for (i, c) in cs.iter().enumerate() {
            if *c == '\n' {
                out.push((start, i));
                start = i + 1;
            }
        }
        out.push((start, cs.len()));
        out
    }

    fn replace(&mut self, start: usize, end: usize, new: &str) {
        let mut cs = self.char_vec();
        let n = new.chars().count();
        cs.splice(start..end, new.chars());
        self.chars.splice(start..end, std::iter::repeat_n(Style::default(), n));
        self.text = cs.into_iter().collect();
    }

    // ---------- Reading HTML ----------

    pub fn from_html(html: &str) -> RichDoc {
        let mut doc = Reader::default().read(html);
        doc.html_cache = html.to_string();
        doc
    }

    // ---------- Writing HTML ----------

    pub fn to_html(&self) -> String {
        let cs = self.char_vec();
        let mut out = String::new();
        let mut open: Option<&str> = None;
        for (i, (a, b)) in self.line_ranges().into_iter().enumerate() {
            let kind = self.lines.get(i).copied().unwrap_or_default();
            let line = &cs[a..b];
            let list = match kind {
                LineKind::Bullet => Some("ul"),
                LineKind::Numbered => Some("ol"),
                _ => None,
            };
            if open != list {
                if let Some(t) = open {
                    out.push_str(&format!("</{t}>"));
                }
                if let Some(t) = list {
                    out.push_str(&format!("<{t}>"));
                }
                open = list;
            }
            let spaces = line.iter().take_while(|c| **c == ' ').count();
            let level = spaces / INDENT.len();
            let lead = lead_len(line, kind).max(level * INDENT.len());
            let body = self.inline_html(a + lead, b);
            let indent = if level > 0 { format!(" style=\"margin-left:{}px\"", level * INDENT_PX) } else { String::new() };
            match kind {
                LineKind::H1 => out.push_str(&format!("<h1{indent}>{body}</h1>")),
                LineKind::H2 => out.push_str(&format!("<h2{indent}>{body}</h2>")),
                LineKind::H3 => out.push_str(&format!("<h3{indent}>{body}</h3>")),
                LineKind::Bullet | LineKind::Numbered => out.push_str(&format!("<li{indent}>{body}</li>")),
                LineKind::Quote => out.push_str(&format!("<blockquote style=\"margin:0 0 0 8px;padding-left:8px;border-left:2px solid #ccc\">{body}</blockquote>")),
                LineKind::Normal if body.is_empty() => out.push_str("<div><br></div>"),
                LineKind::Normal => out.push_str(&format!("<div{indent}>{body}</div>")),
            }
        }
        if let Some(t) = open {
            out.push_str(&format!("</{t}>"));
        }
        out
    }

    fn inline_html(&self, a: usize, b: usize) -> String {
        let cs = self.char_vec();
        let mut out = String::new();
        let mut i = a;
        while i < b {
            let st = &self.chars[i];
            let mut j = i + 1;
            while j < b && &self.chars[j] == st {
                j += 1;
            }
            let run: String = cs[i..j].iter().collect();
            // Keep runs of spaces, which HTML would otherwise squeeze.
            let mut h = escape(&run).replace("  ", " &nbsp;");
            if st.code {
                h = format!("<code>{h}</code>");
            }
            if st.italic {
                h = format!("<em>{h}</em>");
            }
            if st.strike {
                h = format!("<s>{h}</s>");
            }
            if st.underline {
                h = format!("<u>{h}</u>");
            }
            if st.bold {
                h = format!("<strong>{h}</strong>");
            }
            if let Some(u) = st.link.as_ref().filter(|u| safe_url(u)) {
                h = format!("<a href=\"{}\">{h}</a>", escape(u.trim()));
            }
            if let Some(c) = st.bg.as_ref().filter(|c| safe_color(c)) {
                h = format!("<span style=\"background-color:{c}\">{h}</span>");
            }
            if let Some(c) = st.color.as_ref().filter(|c| safe_color(c)) {
                h = format!("<span style=\"color:{c}\">{h}</span>");
            }
            out.push_str(&h);
            i = j;
        }
        out
    }

    // ---------- Typing ----------

    /// The styles the new text would have after an edit, without changing anything.
    pub(crate) fn synced(&self, new_text: &str) -> (Vec<Style>, Vec<LineKind>, usize, usize, usize) {
        let old = self.char_vec();
        let new: Vec<char> = new_text.chars().collect();
        let mut p = 0;
        while p < old.len() && p < new.len() && old[p] == new[p] {
            p += 1;
        }
        let mut s = 0;
        while s < old.len() - p && s < new.len() - p && old[old.len() - 1 - s] == new[new.len() - 1 - s] {
            s += 1;
        }
        let inserted = new.len() - p - s;
        let base = self.pending.clone().unwrap_or_else(|| {
            if p > 0 && old[p - 1] != '\n' {
                self.chars[p - 1].clone()
            } else if p < old.len() && old[p] != '\n' {
                self.chars[p].clone()
            } else {
                Style::default()
            }
        });
        let mut chars = self.chars[..p].to_vec();
        chars.extend(std::iter::repeat_n(base, inserted));
        chars.extend_from_slice(&self.chars[old.len() - s..]);
        let line = old[..p].iter().filter(|c| **c == '\n').count();
        let removed_nl = old[p..old.len() - s].iter().filter(|c| **c == '\n').count();
        let added_nl = new[p..new.len() - s].iter().filter(|c| **c == '\n').count();
        let mut lines = self.lines.clone();
        let kind = lines.get(line).copied().unwrap_or_default();
        let drop_to = (line + 1 + removed_nl).min(lines.len());
        lines.splice((line + 1).min(lines.len())..drop_to, std::iter::repeat_n(kind, added_nl));
        (chars, lines, p, inserted, old.len() - p - s)
    }

    /// Takes in what the text box now holds. Returns where the cursor must go if list handling moved it.
    pub fn apply_edit(&mut self, new_text: &str) -> Option<usize> {
        let (chars, lines, at, inserted, removed) = self.synced(new_text);
        let inserted_text: String = new_text.chars().skip(at).take(inserted).collect();
        self.chars = chars;
        self.lines = lines;
        self.text = new_text.to_string();
        self.pending = None;
        let mut cursor = None;
        if inserted_text == "\n" && removed == 0 {
            cursor = self.after_enter(at);
        }
        // A list line whose mark was erased becomes an ordinary line.
        let cs = self.char_vec();
        for (i, (a, b)) in self.line_ranges().into_iter().enumerate() {
            if !has_mark(&cs[a..b], self.lines[i]) {
                self.lines[i] = LineKind::Normal;
            }
        }
        cursor
    }

    /// Enter continues a list, ends it on an empty item, and leaves a heading.
    fn after_enter(&mut self, at: usize) -> Option<usize> {
        let cs = self.char_vec();
        let line = cs[..at].iter().filter(|c| **c == '\n').count();
        let kind = self.lines[line];
        let ranges = self.line_ranges();
        let (a, b) = ranges[line];
        let this: Vec<char> = cs[a..b].to_vec();
        let lead = lead_len(&this, kind);
        match kind {
            LineKind::Bullet | LineKind::Numbered | LineKind::Quote => {
                let spaces = this.iter().take_while(|c| **c == ' ').count();
                if lead >= this.len() {
                    // Empty item: finish the list here.
                    self.replace(a, b + 1, "");
                    self.lines.remove(line + 1);
                    self.lines[line] = LineKind::Normal;
                    return Some(a);
                }
                let number: usize = this[spaces..].iter().take_while(|c| c.is_ascii_digit()).collect::<String>().parse().unwrap_or(0);
                let mark = match kind {
                    LineKind::Bullet => format!("{}• ", " ".repeat(spaces)),
                    LineKind::Quote => format!("{}│ ", " ".repeat(spaces)),
                    _ => format!("{}{}. ", " ".repeat(spaces), number + 1),
                };
                self.replace(at + 1, at + 1, &mark);
                Some(at + 1 + mark.chars().count())
            }
            LineKind::H1 | LineKind::H2 | LineKind::H3 => {
                if ranges[line + 1].0 == ranges[line + 1].1 {
                    self.lines[line + 1] = LineKind::Normal;
                }
                None
            }
            LineKind::Normal => {
                let spaces = this.iter().take_while(|c| **c == ' ').count();
                if spaces >= INDENT.len() && spaces < this.len() {
                    // Keep the indent going.
                    let pad = " ".repeat(spaces);
                    self.replace(at + 1, at + 1, &pad);
                    return Some(at + 1 + spaces);
                }
                None
            }
        }
    }

    // ---------- Toolbar ----------

    fn span(&self, sel: (usize, usize)) -> (usize, usize) {
        let n = self.chars.len();
        (sel.0.min(sel.1).min(n), sel.0.max(sel.1).min(n))
    }

    /// The style the toolbar should show as active.
    pub fn style_at(&self, sel: (usize, usize)) -> Style {
        if let Some(p) = &self.pending {
            return p.clone();
        }
        let (a, b) = self.span(sel);
        if a < b {
            return self.chars[a].clone();
        }
        if a > 0 { self.chars[a - 1].clone() } else { self.chars.first().cloned().unwrap_or_default() }
    }

    pub fn kind_at(&self, sel: (usize, usize)) -> LineKind {
        let (a, _) = self.span(sel);
        let line = self.char_vec()[..a].iter().filter(|c| **c == '\n').count();
        self.lines.get(line).copied().unwrap_or_default()
    }

    /// Changes the selection's style, or the style of what is typed next if nothing is selected.
    pub fn restyle(&mut self, sel: (usize, usize), change: impl Fn(&mut Style)) {
        let (a, b) = self.span(sel);
        if a == b {
            let mut st = self.style_at(sel);
            change(&mut st);
            self.pending = Some(st);
            return;
        }
        for st in &mut self.chars[a..b] {
            change(st);
        }
    }

    pub fn toggle(&mut self, sel: (usize, usize), attr: Attr) {
        let get = |s: &Style| match attr {
            Attr::Bold => s.bold,
            Attr::Italic => s.italic,
            Attr::Underline => s.underline,
            Attr::Strike => s.strike,
            Attr::Code => s.code,
        };
        let (a, b) = self.span(sel);
        let on = if a == b { !get(&self.style_at(sel)) } else { !self.chars[a..b].iter().all(get) };
        self.restyle(sel, |s| match attr {
            Attr::Bold => s.bold = on,
            Attr::Italic => s.italic = on,
            Attr::Underline => s.underline = on,
            Attr::Strike => s.strike = on,
            Attr::Code => s.code = on,
        });
    }

    /// Adds a link; a plain address gets https in front.
    pub fn set_link(&mut self, sel: (usize, usize), url: &str) {
        let u = url.trim();
        let full: Option<Arc<str>> = if u.is_empty() {
            None
        } else if safe_url(u) {
            Some(u.into())
        } else {
            Some(format!("https://{u}").into())
        };
        self.restyle(sel, |s| s.link = full.clone());
    }

    /// Lines touched by the selection, as line numbers.
    fn lines_of(&self, sel: (usize, usize)) -> std::ops::RangeInclusive<usize> {
        let (a, b) = self.span(sel);
        let cs = self.char_vec();
        let first = cs[..a].iter().filter(|c| **c == '\n').count();
        let last = cs[..b].iter().filter(|c| **c == '\n').count();
        first..=last
    }

    /// Turns the selected lines into a kind, or back to normal if they already are. Returns the new selection.
    pub fn set_kind(&mut self, sel: (usize, usize), kind: LineKind) -> (usize, usize) {
        let range = self.lines_of(sel);
        let off = range.clone().all(|i| self.lines[i] == kind);
        let mut number = 0;
        for i in range.clone() {
            let (a, b) = self.line_ranges()[i];
            let line: Vec<char> = self.char_vec()[a..b].to_vec();
            let spaces = line.iter().take_while(|c| **c == ' ').count();
            let lead = lead_len(&line, self.lines[i]);
            let new_kind = if off { LineKind::Normal } else { kind };
            number += 1;
            let mark = match new_kind {
                LineKind::Bullet => format!("{}• ", " ".repeat(spaces)),
                LineKind::Numbered => format!("{}{number}. ", " ".repeat(spaces)),
                LineKind::Quote => format!("{}│ ", " ".repeat(spaces)),
                _ => " ".repeat(spaces),
            };
            self.replace(a, a + lead, &mark);
            self.lines[i] = new_kind;
        }
        let ranges = self.line_ranges();
        (ranges[*range.start()].0, ranges[*range.end()].1)
    }

    /// One level more or less of indent on the selected lines.
    pub fn indent(&mut self, sel: (usize, usize), more: bool) -> (usize, usize) {
        let range = self.lines_of(sel);
        for i in range.clone() {
            let (a, b) = self.line_ranges()[i];
            let cs = self.char_vec();
            let spaces = cs[a..b].iter().take_while(|c| **c == ' ').count();
            if more {
                self.replace(a, a, INDENT);
            } else if spaces > 0 {
                self.replace(a, a + spaces.min(INDENT.len()), "");
            }
        }
        let ranges = self.line_ranges();
        (ranges[*range.start()].0, ranges[*range.end()].1)
    }

    pub fn clear_formatting(&mut self, sel: (usize, usize)) -> (usize, usize) {
        let range = self.lines_of(sel);
        for i in range.clone() {
            if self.lines[i] != LineKind::Normal {
                let (a, b) = self.line_ranges()[i];
                let line: Vec<char> = self.char_vec()[a..b].to_vec();
                let lead = lead_len(&line, self.lines[i]);
                self.replace(a, a + lead, "");
                self.lines[i] = LineKind::Normal;
            }
        }
        let ranges = self.line_ranges();
        let (a, b) = (ranges[*range.start()].0, ranges[*range.end()].1);
        for st in &mut self.chars[a..b] {
            *st = Style::default();
        }
        self.pending = None;
        (a, b)
    }

}

// ---------- HTML reader ----------

#[derive(Default)]
struct Reader {
    text: String,
    chars: Vec<Style>,
    lines: Vec<LineKind>,
    line_open: bool,
    style: Vec<(String, Style)>,
    blocks: Vec<(String, LineKind, usize)>,
    list: Vec<(bool, usize)>,
    just_broke: bool,
}

fn attr(tag: &str, name: &str) -> Option<String> {
    let lower = tag.to_lowercase();
    let at = lower.find(&format!("{name}="))?;
    let rest = &tag[at + name.len() + 1..];
    let quote = rest.chars().next()?;
    if quote == '"' || quote == '\'' {
        rest[1..].split(quote).next().map(String::from)
    } else {
        rest.split_whitespace().next().map(String::from)
    }
}

fn style_value(style: &str, name: &str) -> Option<String> {
    style.split(';').find_map(|p| {
        let (k, v) = p.split_once(':')?;
        (k.trim().eq_ignore_ascii_case(name)).then(|| v.trim().to_string())
    })
}

impl Reader {
    fn current(&self) -> Style {
        self.style.last().map(|s| s.1.clone()).unwrap_or_default()
    }

    fn kind(&self) -> LineKind {
        self.blocks.iter().rev().map(|b| b.1).find(|k| *k != LineKind::Normal).unwrap_or_default()
    }

    fn indent(&self) -> usize {
        self.blocks.iter().map(|b| b.2).sum()
    }

    /// Starts a line if none is open, writing its indent and list mark.
    fn begin_line(&mut self) {
        if self.line_open {
            return;
        }
        if !self.lines.is_empty() {
            self.text.push('\n');
            self.chars.push(Style::default());
        }
        let kind = self.kind();
        let mut lead = INDENT.repeat(self.indent());
        match kind {
            LineKind::Bullet => lead.push_str("• "),
            LineKind::Quote => lead.push_str("│ "),
            LineKind::Numbered => {
                let n = self.list.last_mut().map(|l| {
                    l.1 += 1;
                    l.1
                });
                lead.push_str(&format!("{}. ", n.unwrap_or(1)));
            }
            _ => {}
        }
        for c in lead.chars() {
            self.text.push(c);
            self.chars.push(Style::default());
        }
        self.lines.push(kind);
        self.line_open = true;
        self.just_broke = false;
    }

    fn end_line(&mut self) {
        self.line_open = false;
    }

    fn add_text(&mut self, raw: &str) {
        let flat: String = raw.split(['\r', '\n', '\t']).collect::<Vec<_>>().join(" ");
        let t = unescape(&flat);
        if t.trim().is_empty() && !self.line_open {
            return;
        }
        self.begin_line();
        let st = self.current();
        for c in t.chars() {
            self.text.push(c);
            self.chars.push(st.clone());
        }
    }

    fn open_block(&mut self, name: &str, kind: LineKind, tag: &str) {
        self.end_line();
        let margin = attr(tag, "style").and_then(|s| style_value(&s, "margin-left")).and_then(|v| v.trim_end_matches("px").trim().parse::<usize>().ok());
        let level = if kind == LineKind::Quote { 0 } else { margin.map(|m| m / INDENT_PX).unwrap_or(0) };
        self.blocks.push((name.to_string(), kind, level));
    }

    fn close_block(&mut self, name: &str) {
        self.end_line();
        if let Some(p) = self.blocks.iter().rposition(|b| b.0 == name) {
            self.blocks.truncate(p);
        }
    }

    fn open_inline(&mut self, name: &str, tag: &str) {
        let mut st = self.current();
        match name {
            "strong" | "b" => st.bold = true,
            "em" | "i" => st.italic = true,
            "u" => st.underline = true,
            "s" | "strike" | "del" => st.strike = true,
            "code" => st.code = true,
            "a" => {
                if let Some(h) = attr(tag, "href").filter(|h| safe_url(h)) {
                    st.link = Some(unescape(&h).into());
                }
            }
            _ => {
                if let Some(css) = attr(tag, "style") {
                    if let Some(c) = style_value(&css, "color").filter(|c| safe_color(c)) {
                        st.color = Some(c.into());
                    }
                    if let Some(c) = style_value(&css, "background-color").filter(|c| safe_color(c)) {
                        st.bg = Some(c.into());
                    }
                    if style_value(&css, "font-weight").is_some_and(|w| w == "bold" || w == "700") {
                        st.bold = true;
                    }
                    if style_value(&css, "font-style").is_some_and(|w| w == "italic") {
                        st.italic = true;
                    }
                }
            }
        }
        self.style.push((name.to_string(), st));
    }

    fn close_inline(&mut self, name: &str) {
        if let Some(p) = self.style.iter().rposition(|s| s.0 == name) {
            self.style.truncate(p);
        }
    }

    fn read(mut self, html: &str) -> RichDoc {
        let mut rest = html;
        while !rest.is_empty() {
            let Some(lt) = rest.find('<') else {
                self.add_text(rest);
                break;
            };
            if lt > 0 {
                self.add_text(&rest[..lt]);
            }
            rest = &rest[lt..];
            let Some(gt) = rest.find('>') else {
                self.add_text(&rest.replace('<', "&lt;"));
                break;
            };
            let tag = &rest[1..gt];
            rest = &rest[gt + 1..];
            let closing = tag.starts_with('/');
            let name = tag.trim_start_matches('/').split(|c: char| c.is_whitespace() || c == '/').next().unwrap_or("").to_lowercase();
            // Code and page parts never become text.
            if !closing && matches!(name.as_str(), "style" | "script" | "head" | "title") {
                let end = format!("</{name}");
                match rest.to_lowercase().find(&end) {
                    Some(p) => {
                        let tail = &rest[p..];
                        rest = tail.find('>').map(|g| &tail[g + 1..]).unwrap_or("");
                    }
                    None => rest = "",
                }
                continue;
            }
            match (name.as_str(), closing) {
                ("br", _) => {
                    // A line break ends the line; in an empty block it makes a blank line.
                    self.begin_line();
                    self.end_line();
                    self.just_broke = true;
                }
                ("div" | "p" | "section" | "article" | "table" | "tr" | "td", false) => self.open_block(&name, LineKind::Normal, tag),
                ("h1", false) => self.open_block(&name, LineKind::H1, tag),
                ("h2", false) => self.open_block(&name, LineKind::H2, tag),
                ("h3" | "h4" | "h5" | "h6", false) => self.open_block(&name, LineKind::H3, tag),
                ("blockquote", false) => self.open_block(&name, LineKind::Quote, tag),
                ("ul", false) => {
                    self.end_line();
                    self.list.push((false, 0));
                }
                ("ol", false) => {
                    self.end_line();
                    self.list.push((true, 0));
                }
                ("ul" | "ol", true) => {
                    self.end_line();
                    self.list.pop();
                }
                ("li", false) => {
                    let numbered = self.list.last().is_some_and(|l| l.0);
                    self.open_block("li", if numbered { LineKind::Numbered } else { LineKind::Bullet }, tag);
                }
                ("div" | "p" | "section" | "article" | "table" | "tr" | "td" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "blockquote" | "li", true) => self.close_block(&name),
                ("strong" | "b" | "em" | "i" | "u" | "s" | "strike" | "del" | "code" | "a" | "span" | "font", false) => self.open_inline(&name, tag),
                ("strong" | "b" | "em" | "i" | "u" | "s" | "strike" | "del" | "code" | "a" | "span" | "font", true) => self.close_inline(&name),
                _ => {}
            }
        }
        if self.lines.is_empty() {
            self.lines.push(LineKind::Normal);
        }
        RichDoc { text: self.text, chars: self.chars, lines: self.lines, ..Default::default() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc(html: &str) -> RichDoc {
        RichDoc::from_html(html)
    }

    fn same_after_reload(d: &RichDoc) {
        let html = d.to_html();
        let again = RichDoc::from_html(&html);
        assert_eq!(again.text, d.text, "{html}");
        assert_eq!(again.chars, d.chars, "{html}");
        assert_eq!(again.lines, d.lines, "{html}");
        assert_eq!(again.to_html(), html);
    }

    #[test]
    fn no_marks_show_in_the_text() {
        let d = doc("<h1>Ciao</h1><div><strong>Bold</strong> <em>it</em></div><ul><li>uno</li><li>due</li></ul><ol><li>a</li><li>b</li></ol>");
        assert_eq!(d.text, "Ciao\nBold it\n• uno\n• due\n1. a\n2. b");
        assert_eq!(d.lines, vec![LineKind::H1, LineKind::Normal, LineKind::Bullet, LineKind::Bullet, LineKind::Numbered, LineKind::Numbered]);
        assert!(d.chars[5].bold && !d.chars[9].bold && d.chars[10].italic);
    }

    #[test]
    fn everything_survives_saving_and_reopening() {
        let mut d = RichDoc::empty();
        d.apply_edit("Titolo\nGrassetto corsivo città & <co>\nvoce");
        d.set_kind((0, 0), LineKind::H2);
        d.toggle((7, 16), Attr::Bold);
        d.toggle((17, 23), Attr::Italic);
        d.toggle((17, 23), Attr::Bold);
        d.restyle((24, 29), |s| s.color = Some("#157efb".into()));
        d.restyle((24, 29), |s| s.bg = Some("#fff3a3".into()));
        d.toggle((24, 29), Attr::Underline);
        d.set_link((30, 33), "example.it");
        same_after_reload(&d);
        let n = d.text.chars().count();
        d.set_kind((n, n), LineKind::Bullet);
        same_after_reload(&d);
        assert!(d.to_html().contains("<a href=\"https://example.it\">"));
        assert!(d.to_html().contains("&lt;co&gt;"));
    }

    #[test]
    fn spaces_and_blank_lines_survive() {
        let mut d = RichDoc::empty();
        d.apply_edit("a  b\n\nc");
        same_after_reload(&d);
        assert_eq!(doc(&d.to_html()).text, "a  b\n\nc");
    }

    #[test]
    fn typing_inherits_the_style_next_to_it() {
        let mut d = doc("<div><strong>ab</strong></div>");
        d.apply_edit("abc");
        assert!(d.chars[2].bold);
        d.apply_edit("xabc");
        assert!(d.chars[0].bold, "at the start it takes the next character's style");
    }

    #[test]
    fn toolbar_with_no_selection_styles_what_comes_next() {
        let mut d = doc("<div>ab</div>");
        d.toggle((2, 2), Attr::Bold);
        assert!(d.style_at((2, 2)).bold);
        d.apply_edit("abc");
        assert!(d.chars[2].bold && !d.chars[1].bold);
        d.apply_edit("abcd");
        assert!(d.chars[3].bold);
    }

    #[test]
    fn toggling_a_selection_turns_on_then_off() {
        let mut d = doc("<div>abc</div>");
        d.toggle((0, 3), Attr::Bold);
        assert!(d.chars.iter().all(|s| s.bold));
        d.toggle((0, 3), Attr::Bold);
        assert!(d.chars.iter().all(|s| !s.bold));
    }

    #[test]
    fn enter_continues_and_ends_lists() {
        let mut d = doc("<ul><li>uno</li></ul>");
        let c = d.apply_edit("• uno\n");
        assert_eq!(d.text, "• uno\n• ");
        assert_eq!(c, Some(8));
        assert_eq!(d.lines, vec![LineKind::Bullet, LineKind::Bullet]);
        let c = d.apply_edit("• uno\n• \n");
        assert_eq!(d.text, "• uno\n");
        assert_eq!(c, Some(6));
        assert_eq!(d.lines, vec![LineKind::Bullet, LineKind::Normal]);
    }

    #[test]
    fn numbered_lists_count_up() {
        let mut d = doc("<ol><li>uno</li></ol>");
        d.apply_edit("1. uno\n");
        assert_eq!(d.text, "1. uno\n2. ");
    }

    #[test]
    fn erasing_the_mark_ends_the_item() {
        let mut d = doc("<ul><li>uno</li></ul>");
        d.apply_edit("• uno".trim_start_matches('•').trim_start());
        assert_eq!(d.lines, vec![LineKind::Normal]);
    }

    #[test]
    fn enter_leaves_a_heading() {
        let mut d = doc("<h1>Ciao</h1>");
        d.apply_edit("Ciao\n");
        assert_eq!(d.lines, vec![LineKind::H1, LineKind::Normal]);
    }

    #[test]
    fn joining_lines_keeps_the_first_kind() {
        let mut d = doc("<h1>Ciao</h1><div>mondo</div>");
        d.apply_edit("Ciaomondo");
        assert_eq!(d.lines, vec![LineKind::H1]);
        assert_eq!(d.chars.len(), 9);
    }

    #[test]
    fn line_kinds_turn_on_and_off() {
        let mut d = doc("<div>uno</div><div>due</div>");
        let sel = d.set_kind((0, 5), LineKind::Numbered);
        assert_eq!(d.text, "1. uno\n2. due");
        assert_eq!(sel, (0, 13));
        d.set_kind(sel, LineKind::Numbered);
        assert_eq!(d.text, "uno\ndue");
        d.set_kind((0, 0), LineKind::H1);
        d.set_kind((0, 0), LineKind::Quote);
        assert_eq!(d.text, "│ uno\ndue");
        assert_eq!(d.lines[0], LineKind::Quote);
    }

    #[test]
    fn indent_goes_and_comes_back() {
        let mut d = doc("<div>uno</div>");
        d.indent((0, 0), true);
        assert_eq!(d.text, "    uno");
        same_after_reload(&d);
        assert!(d.to_html().contains("margin-left:24px"));
        d.indent((0, 0), false);
        assert_eq!(d.text, "uno");
    }

    #[test]
    fn clearing_formatting_removes_styles_and_marks() {
        let mut d = doc("<ul><li><strong>a</strong></li></ul>");
        d.clear_formatting((0, 3));
        assert_eq!(d.text, "a");
        assert_eq!(d.chars[0], Style::default());
        assert_eq!(d.lines, vec![LineKind::Normal]);
    }

    #[test]
    fn foreign_html_is_read_and_dangerous_parts_dropped() {
        let d = doc("<html><head><title>x</title><style>b{}</style></head><body><p>Uno<br>Due</p><script>alert(1)</script><a href=\"javascript:x\">l</a><b style=\"color:red\">g</b></body></html>");
        assert_eq!(d.text, "Uno\nDue\nlg");
        assert!(d.chars[8].link.is_none());
        assert!(d.chars[9].bold);
        assert!(!d.to_html().contains("script"));
    }

    #[test]
    fn empty_html_gives_an_empty_page() {
        let d = doc("");
        assert_eq!(d.lines.len(), 1);
        assert_eq!(d.text, "");
        assert_eq!(doc("<div><br></div>").text, "");
    }

}
