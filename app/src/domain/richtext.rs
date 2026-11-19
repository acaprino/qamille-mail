//! Reads the small markup older drafts and signatures were saved in, and builds the final message
//! in the original's order: message, signature, quote (DraftFullBodyGenerator, SparkComposer 0x180065180).

use crate::domain::text;
use crate::model::{BodyFormat, Draft, Store};

#[derive(Debug, PartialEq, Clone)]
pub enum Node {
    Text(String),
    Bold(Vec<Node>),
    Italic(Vec<Node>),
    Underline(Vec<Node>),
    Strike(Vec<Node>),
    Code(String),
    Link(Vec<Node>, String),
    Color(String, Vec<Node>),
    Highlight(String, Vec<Node>),
}

pub const TEXT_COLORS: [(&str, &str); 6] =
    [("Red", "#d0342c"), ("Orange", "#d9730d"), ("Green", "#2e9e5b"), ("Blue", "#157efb"), ("Purple", "#7c5ce6"), ("Gray", "#66707e")];
pub const HIGHLIGHT_COLORS: [(&str, &str); 4] = [("Yellow", "#fff3a3"), ("Green", "#c9f2d0"), ("Blue", "#cfe6ff"), ("Pink", "#ffd6e7")];

pub fn escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn safe_color(c: &str) -> bool {
    let h = c.strip_prefix('#').unwrap_or("");
    (h.len() == 3 || h.len() == 6) && h.chars().all(|x| x.is_ascii_hexdigit())
}

fn safe_url(u: &str) -> bool {
    let l = u.trim().to_lowercase();
    l.starts_with("https://") || l.starts_with("http://") || l.starts_with("mailto:")
}

/// Reads one line's inline marks. Anything unmatched stays as literal text.
pub fn parse_inline(s: &str) -> Vec<Node> {
    let mut out: Vec<Node> = Vec::new();
    let mut text = String::new();
    let mut i = 0;
    let flush = |text: &mut String, out: &mut Vec<Node>| {
        if !text.is_empty() {
            out.push(Node::Text(std::mem::take(text)));
        }
    };
    while i < s.len() {
        let rest = &s[i..];
        if let Some(r) = rest.strip_prefix('\\') {
            if let Some(c) = r.chars().next().filter(|c| "*_~`[]{}\\".contains(*c)) {
                text.push(c);
                i += 1 + c.len_utf8();
                continue;
            }
        }
        let mut matched = false;
        for (mark, kind) in [("**", 0), ("__", 1), ("~~", 2), ("`", 3), ("*", 4)] {
            if let Some(r) = rest.strip_prefix(mark) {
                // A mark counts only when it hugs the text, so "2 * 3" stays as typed.
                let hugging = !r.starts_with(char::is_whitespace);
                if let Some(end) = r.find(mark).filter(|e| hugging && *e > 0 && !r[..*e].ends_with(char::is_whitespace)) {
                    flush(&mut text, &mut out);
                    let inner = &r[..end];
                    out.push(match kind {
                        0 => Node::Bold(parse_inline(inner)),
                        1 => Node::Underline(parse_inline(inner)),
                        2 => Node::Strike(parse_inline(inner)),
                        3 => Node::Code(inner.to_string()),
                        _ => Node::Italic(parse_inline(inner)),
                    });
                    i += mark.len() * 2 + end;
                    matched = true;
                    break;
                }
            }
        }
        if matched {
            continue;
        }
        if rest.starts_with('[') {
            if let Some(mid) = rest.find("](") {
                if let Some(close) = rest[mid + 2..].find(')') {
                    flush(&mut text, &mut out);
                    out.push(Node::Link(parse_inline(&rest[1..mid]), rest[mid + 2..mid + 2 + close].to_string()));
                    i += mid + 2 + close + 1;
                    continue;
                }
            }
        }
        if rest.starts_with("{c:#") || rest.starts_with("{h:#") {
            if let (Some(bar), Some(close)) = (rest.find('|'), rest.find('}')) {
                if bar < close && safe_color(&rest[3..bar]) {
                    flush(&mut text, &mut out);
                    let (color, inner) = (rest[3..bar].to_string(), parse_inline(&rest[bar + 1..close]));
                    out.push(if rest.starts_with("{c") { Node::Color(color, inner) } else { Node::Highlight(color, inner) });
                    i += close + 1;
                    continue;
                }
            }
        }
        let c = rest.chars().next().unwrap_or(' ');
        text.push(c);
        i += c.len_utf8();
    }
    flush(&mut text, &mut out);
    out
}

fn nodes_html(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|n| match n {
            Node::Text(t) => escape(t),
            Node::Bold(c) => format!("<strong>{}</strong>", nodes_html(c)),
            Node::Italic(c) => format!("<em>{}</em>", nodes_html(c)),
            Node::Underline(c) => format!("<u>{}</u>", nodes_html(c)),
            Node::Strike(c) => format!("<s>{}</s>", nodes_html(c)),
            Node::Code(t) => format!("<code>{}</code>", escape(t)),
            Node::Link(c, u) if safe_url(u) => format!("<a href=\"{}\">{}</a>", escape(u.trim()), nodes_html(c)),
            Node::Link(c, u) => format!("{} ({})", nodes_html(c), escape(u)),
            Node::Color(col, c) => format!("<span style=\"color:{col}\">{}</span>", nodes_html(c)),
            Node::Highlight(col, c) => format!("<span style=\"background-color:{col}\">{}</span>", nodes_html(c)),
        })
        .collect()
}

fn nodes_plain(nodes: &[Node]) -> String {
    nodes
        .iter()
        .map(|n| match n {
            Node::Text(t) | Node::Code(t) => t.clone(),
            Node::Link(c, u) => {
                let label = nodes_plain(c);
                if label.trim() == u.trim() { label } else { format!("{label} ({u})") }
            }
            Node::Bold(c) | Node::Italic(c) | Node::Underline(c) | Node::Strike(c) | Node::Color(_, c) | Node::Highlight(_, c) => nodes_plain(c),
        })
        .collect()
}

#[derive(Debug, PartialEq)]
enum Line<'a> {
    Heading(usize, &'a str),
    Bullet(&'a str),
    Numbered(&'a str, &'a str),
    Quote(&'a str),
    Text(&'a str),
    Blank,
}

/// Returns the indent level (each four spaces) and the kind of line.
fn classify(line: &str) -> (usize, Line<'_>) {
    let trimmed = line.trim_start_matches(' ');
    let level = (line.len() - trimmed.len()) / 4;
    if trimmed.trim().is_empty() {
        return (0, Line::Blank);
    }
    for (p, n) in [("### ", 3), ("## ", 2), ("# ", 1)] {
        if let Some(r) = trimmed.strip_prefix(p) {
            return (level, Line::Heading(n, r));
        }
    }
    if let Some(r) = trimmed.strip_prefix("- ") {
        return (level, Line::Bullet(r));
    }
    if let Some(r) = trimmed.strip_prefix("> ") {
        return (level, Line::Quote(r));
    }
    if let Some((num, r)) = trimmed.split_once(". ") {
        if !num.is_empty() && num.chars().all(|c| c.is_ascii_digit()) {
            return (level, Line::Numbered(num, r));
        }
    }
    (level, Line::Text(trimmed))
}

pub fn markup_to_html(markup: &str) -> String {
    let mut out = String::new();
    let mut open: Option<&str> = None;
    for raw in markup.lines() {
        let (level, line) = classify(raw);
        let list = match line {
            Line::Bullet(_) => Some("ul"),
            Line::Numbered(..) => Some("ol"),
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
        let indent = if level > 0 { format!(" style=\"margin-left:{}px\"", level * 24) } else { String::new() };
        match line {
            Line::Blank => out.push_str("<div><br></div>"),
            Line::Heading(n, t) => out.push_str(&format!("<h{n}{indent}>{}</h{n}>", nodes_html(&parse_inline(t)))),
            Line::Bullet(t) | Line::Numbered(_, t) => out.push_str(&format!("<li{indent}>{}</li>", nodes_html(&parse_inline(t)))),
            Line::Quote(t) => out.push_str(&format!("<blockquote style=\"margin:0 0 0 8px;padding-left:8px;border-left:2px solid #ccc\">{}</blockquote>", nodes_html(&parse_inline(t)))),
            Line::Text(t) => out.push_str(&format!("<div{indent}>{}</div>", nodes_html(&parse_inline(t)))),
        }
    }
    if let Some(t) = open {
        out.push_str(&format!("</{t}>"));
    }
    out
}

pub fn markup_to_plain(markup: &str) -> String {
    markup
        .lines()
        .map(|raw| {
            let (level, line) = classify(raw);
            let pad = "    ".repeat(level);
            match line {
                Line::Blank => String::new(),
                Line::Heading(_, t) | Line::Text(t) => format!("{pad}{}", nodes_plain(&parse_inline(t))),
                Line::Bullet(t) => format!("{pad}• {}", nodes_plain(&parse_inline(t))),
                Line::Numbered(n, t) => format!("{pad}{n}. {}", nodes_plain(&parse_inline(t))),
                Line::Quote(t) => format!("{pad}> {}", nodes_plain(&parse_inline(t))),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn plain_to_html(text: &str) -> String {
    text.lines().map(|l| if l.is_empty() { "<div><br></div>".to_string() } else { format!("<div>{}</div>", escape(l)) }).collect()
}

/// Moves a body between editor modes.
pub fn convert_body(body: &str, from: BodyFormat, to: BodyFormat) -> String {
    match (from, to) {
        (a, b) if a == b => body.to_string(),
        (BodyFormat::Plain, BodyFormat::Rich) => body.to_string(),
        (BodyFormat::Html, BodyFormat::Plain) => crate::richdoc::RichDoc::from_html(body).text,
        (BodyFormat::Plain, BodyFormat::Html) => plain_to_html(body),
        (BodyFormat::Rich, BodyFormat::Plain) => markup_to_plain(body),
        (BodyFormat::Rich, BodyFormat::Html) => markup_to_html(body),
        (BodyFormat::Html, _) => text::html_to_text(body),
        _ => body.to_string(),
    }
}

pub struct Rendered {
    pub plain: String,
    pub html: Option<String>,
}

/// The message as it will leave: message, then signature, then quote.
pub fn render_draft(store: &Store, d: &Draft) -> Rendered {
    let signature = d.signature_id.and_then(|id| store.signature(id));
    let sig_plain = signature.map(|s| s.plain_body()).unwrap_or_default();
    let sig_html = signature.map(|s| s.html_body()).unwrap_or_default();
    let join = |parts: Vec<String>| parts.into_iter().filter(|p| !p.trim().is_empty()).collect::<Vec<_>>().join("\n\n");
    let quote_plain = d.quote.clone();
    match d.format {
        BodyFormat::Plain => Rendered { plain: join(vec![d.body.clone(), sig_plain, quote_plain]), html: None },
        BodyFormat::Rich | BodyFormat::Html => {
            let (body_html, body_plain) = if d.format == BodyFormat::Rich {
                (markup_to_html(&d.body), markup_to_plain(&d.body))
            } else {
                (d.body.clone(), crate::richdoc::RichDoc::from_html(&d.body).text)
            };
            let mut html = format!("<div style=\"font-family:sans-serif;font-size:14px\">{body_html}");
            if !sig_plain.trim().is_empty() {
                html.push_str(&format!("<br><div>{sig_html}</div>"));
            }
            if !d.quote.trim().is_empty() {
                html.push_str(&format!("<br><blockquote style=\"margin:0 0 0 8px;padding-left:8px;border-left:2px solid #ccc\">{}</blockquote>", plain_to_html(&d.quote)));
            }
            html.push_str("</div>");
            Rendered { plain: join(vec![body_plain, sig_plain, quote_plain]), html: Some(html) }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Signature, SignatureBinding};

    #[test]
    fn inline_marks_become_html() {
        assert_eq!(markup_to_html("**a** *b* __c__ ~~d~~ `e`"), "<div><strong>a</strong> <em>b</em> <u>c</u> <s>d</s> <code>e</code></div>");
        assert_eq!(markup_to_html("{c:#157efb|blu} {h:#fff3a3|giallo}"), "<div><span style=\"color:#157efb\">blu</span> <span style=\"background-color:#fff3a3\">giallo</span></div>");
    }

    #[test]
    fn typed_html_is_escaped_and_bad_links_are_refused() {
        assert_eq!(markup_to_html("<script>x</script> & co"), "<div>&lt;script&gt;x&lt;/script&gt; &amp; co</div>");
        assert!(markup_to_html("[qui](https://a.it)").contains("<a href=\"https://a.it\">qui</a>"));
        let bad = markup_to_html("[x](javascript:alert(1))");
        assert!(!bad.contains("href"), "{bad}");
        assert!(!markup_to_html("{c:red;x|y}").contains("style"));
    }

    #[test]
    fn blocks_and_lists() {
        let h = markup_to_html("# T\n- a\n- b\n1. c\n> q\n    rientro");
        assert!(h.starts_with("<h1>T</h1><ul><li>a</li><li>b</li></ul><ol><li>c</li></ol><blockquote"), "{h}");
        assert!(h.contains("<div style=\"margin-left:24px\">rientro</div>"));
    }

    #[test]
    fn plain_alternative_drops_the_marks() {
        assert_eq!(markup_to_plain("**a** [b](https://c.it)\n- d"), "a b (https://c.it)\n• d");
    }

    #[test]
    fn unmatched_marks_stay_as_text_and_accents_survive() {
        assert_eq!(markup_to_plain("2 * 3 è più **"), "2 * 3 è più **");
        assert_eq!(markup_to_plain("\\*non corsivo\\*"), "*non corsivo*");
    }

    #[test]
    fn modes_convert() {
        assert_eq!(convert_body("**a**", BodyFormat::Rich, BodyFormat::Plain), "a");
        assert_eq!(convert_body("a<b", BodyFormat::Plain, BodyFormat::Html), "<div>a&lt;b</div>");
        assert_eq!(convert_body("<div>ciao</div>", BodyFormat::Html, BodyFormat::Plain), "ciao");
    }

    fn store_with_signature() -> Store {
        let mut s = Store::default();
        s.signatures.push(Signature { id: 5, name: "Lavoro".into(), body: "**Alfio**".into(), modified: 0, html: false });
        s
    }

    #[test]
    fn visual_editor_html_goes_out_as_written() {
        let s = store_with_signature();
        let d = Draft { body: "<div><strong>Ciao</strong></div>".into(), signature_id: Some(5), format: BodyFormat::Html, ..Default::default() };
        let r = render_draft(&s, &d);
        assert!(r.html.unwrap().contains("<div><strong>Ciao</strong></div>"));
        assert_eq!(r.plain, "Ciao\n\nAlfio");
    }

    #[test]
    fn message_order_is_body_signature_quote() {
        let s = store_with_signature();
        let d = Draft { body: "Ciao".into(), quote: "Citazione".into(), signature_id: Some(5), format: BodyFormat::Plain, ..Default::default() };
        let r = render_draft(&s, &d);
        assert_eq!(r.plain, "Ciao\n\nAlfio\n\nCitazione");
        assert!(r.html.is_none());
        let d = Draft { format: BodyFormat::Rich, ..d };
        let r = render_draft(&s, &d);
        let h = r.html.unwrap();
        let (a, b, c) = (h.find("Ciao").unwrap(), h.find("<strong>Alfio").unwrap(), h.find("Citazione").unwrap());
        assert!(a < b && b < c, "{h}");
        assert_eq!(r.plain, "Ciao\n\nAlfio\n\nCitazione");
    }

    #[test]
    fn no_signature_means_none_in_the_message() {
        let s = store_with_signature();
        let d = Draft { body: "Ciao".into(), signature_id: None, format: BodyFormat::Plain, ..Default::default() };
        assert_eq!(render_draft(&s, &d).plain, "Ciao");
        let _ = SignatureBinding::None;
    }
}
