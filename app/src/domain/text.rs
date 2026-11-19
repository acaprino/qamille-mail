//! Plain text conversion, with no filesystem or UI dependencies.
pub fn strip_xml(s: &str) -> String {
    let mut out = String::with_capacity(s.len() / 2);
    let mut in_tag = false;
    let mut tag = String::new();
    for ch in s.chars() {
        match ch {
            '<' => {
                in_tag = true;
                tag.clear();
            }
            '>' => {
                in_tag = false;
                let t = tag.to_lowercase();
                if t.starts_with("/w:p") || t.starts_with("br") || t.starts_with("/p") || t.starts_with("/tr") || t.starts_with("/div") || t.starts_with("/si") || t.starts_with("/a:p") {
                    out.push('\n');
                } else if t.starts_with("w:tab") || t.starts_with("/td") {
                    out.push('\t');
                }
            }
            c if in_tag => tag.push(c),
            c => out.push(c),
        }
    }
    out.replace("&amp;", "&").replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&nbsp;", " ")
}

pub fn html_to_text(html: &str) -> String {
    // Drop <style> and <script> blocks first.
    let mut s = html.to_string();
    for tag in ["style", "script", "head"] {
        while let Some(a) = s.to_lowercase().find(&format!("<{tag}")) {
            let lower = s.to_lowercase();
            let end = lower[a..].find(&format!("</{tag}>")).map(|i| a + i + tag.len() + 3).unwrap_or(s.len());
            s.replace_range(a..end, "");
        }
    }
    let t = strip_xml(&s);
    let mut out = String::new();
    let mut blank = 0;
    for line in t.lines() {
        let l = line.split_whitespace().collect::<Vec<_>>().join(" ");
        if l.is_empty() {
            blank += 1;
            if blank <= 1 {
                out.push('\n');
            }
        } else {
            blank = 0;
            out.push_str(&l);
            out.push('\n');
        }
    }
    out.trim().to_string()
}

