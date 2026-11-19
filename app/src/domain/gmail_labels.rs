//! Gmail labels (`X-GM-LABELS`) to the logical folder, labels and state of a row.
//!
//! Pure. A Gmail message is one message with many labels: the folder of its row
//! comes from the system labels, every user label stays a label.

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GmailPlacement {
    /// `Inbox`, `Sent`, `Trash`, `Spam` or `Archive`.
    pub folder: String,
    /// User labels, decoded.
    pub labels: Vec<String>,
    pub starred: bool,
    pub important: bool,
    /// A draft: drafts have their own flow and never become rows here.
    pub skip: bool,
}

/// Classifies decoded labels. Precedence: Trash > Spam > Inbox > Sent > Archive
/// (Gmail can leave `\Inbox` on a message already in the trash).
pub fn classify(labels: &[String]) -> GmailPlacement {
    let mut placement = GmailPlacement::default();
    let (mut inbox, mut sent, mut trash, mut spam) = (false, false, false, false);
    for label in labels {
        match label.to_ascii_lowercase().as_str() {
            "\\inbox" | "inbox" => inbox = true,
            "\\sent" => sent = true,
            "\\trash" => trash = true,
            "\\spam" | "\\junk" => spam = true,
            "\\draft" | "\\drafts" => placement.skip = true,
            "\\starred" => placement.starred = true,
            "\\important" => placement.important = true,
            system if system.starts_with('\\') => {}
            _ => {
                if !placement.labels.contains(label) {
                    placement.labels.push(label.clone());
                }
            }
        }
    }
    placement.folder = if trash {
        "Trash"
    } else if spam {
        "Spam"
    } else if inbox {
        "Inbox"
    } else if sent {
        "Sent"
    } else {
        "Archive"
    }
    .into();
    placement
}

/// Decodes a modified UTF-7 mailbox or label name (RFC 3501, 5.1.3). A name that
/// is not valid modified UTF-7 is returned unchanged.
pub fn decode_modified_utf7(name: &str) -> String {
    fn decode(name: &str) -> Option<String> {
        let mut output = String::new();
        let mut rest = name;
        while let Some(start) = rest.find('&') {
            output.push_str(&rest[..start]);
            let after = &rest[start + 1..];
            let end = after.find('-')?;
            let encoded = &after[..end];
            rest = &after[end + 1..];
            if encoded.is_empty() {
                output.push('&');
                continue;
            }
            let mut bits = 0u32;
            let mut count = 0;
            let mut units = Vec::new();
            for byte in encoded.bytes() {
                let value = match byte {
                    b'A'..=b'Z' => byte - b'A',
                    b'a'..=b'z' => byte - b'a' + 26,
                    b'0'..=b'9' => byte - b'0' + 52,
                    b'+' => 62,
                    b',' => 63,
                    _ => return None,
                } as u32;
                bits = (bits << 6) | value;
                count += 6;
                if count >= 16 {
                    count -= 16;
                    units.push(((bits >> count) & 0xFFFF) as u16);
                }
            }
            output.push_str(&String::from_utf16(&units).ok()?);
        }
        output.push_str(rest);
        Some(output)
    }
    decode(name).unwrap_or_else(|| name.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn labels(values: &[&str]) -> Vec<String> {
        values.iter().map(|v| v.to_string()).collect()
    }

    #[test]
    fn a_message_with_three_labels_is_one_inbox_row_with_two_labels() {
        // EX-5
        let placement = classify(&labels(&["\\Inbox", "Clienti", "2024"]));
        assert_eq!(placement.folder, "Inbox");
        assert_eq!(placement.labels, ["Clienti", "2024"]);
        assert!(!placement.skip);
    }

    #[test]
    fn system_labels_choose_the_folder_with_the_documented_precedence() {
        assert_eq!(classify(&labels(&["\\Sent"])).folder, "Sent");
        assert_eq!(classify(&labels(&["\\Inbox", "\\Sent"])).folder, "Inbox");
        assert_eq!(classify(&labels(&["\\Inbox", "\\Trash"])).folder, "Trash");
        assert_eq!(classify(&labels(&["\\Spam", "\\Inbox"])).folder, "Spam");
        assert_eq!(classify(&labels(&["\\Trash", "\\Spam"])).folder, "Trash");
        assert_eq!(classify(&labels(&["Clienti"])).folder, "Archive");
        assert_eq!(classify(&[]).folder, "Archive");
    }

    #[test]
    fn starred_and_important_are_state_and_drafts_are_skipped() {
        let placement = classify(&labels(&["\\Starred", "\\Important", "\\Inbox"]));
        assert!(placement.starred && placement.important);
        assert!(placement.labels.is_empty(), "system labels never become user labels");
        assert!(classify(&labels(&["\\Draft"])).skip);
        assert!(classify(&labels(&["\\Muted"])).labels.is_empty());
    }

    #[test]
    fn modified_utf7_names_are_decoded() {
        assert_eq!(decode_modified_utf7("Fatture &AOg- ricevute"), "Fatture è ricevute");
        assert_eq!(decode_modified_utf7("&AMk-t&AOk-"), "Été");
        assert_eq!(decode_modified_utf7("Soci &- partner"), "Soci & partner");
        assert_eq!(decode_modified_utf7("&ZeVnLIqe-"), "日本語");
        assert_eq!(decode_modified_utf7("plain"), "plain");
        assert_eq!(decode_modified_utf7("broken &AOg"), "broken &AOg", "invalid input stays as it is");
    }
}
