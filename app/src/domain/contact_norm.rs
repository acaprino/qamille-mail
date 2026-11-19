//! Pure normalization of contact values. No I/O.
use unicode_normalization::UnicodeNormalization;

const TITLES: [&str; 10] = ["dott", "dr", "sig", "mr", "mrs", "ms", "prof", "ing", "avv", "dottssa"];
const DEFAULT_ROLES: [&str; 13] = [
    "info", "vendite", "sales", "support", "supporto", "noreply", "no-reply", "admin", "ufficio",
    "segreteria", "amministrazione", "contabilita", "famiglia",
];

/// Calling code and whether national numbers drop a leading trunk zero.
fn calling_code(region: &str) -> Option<(&'static str, bool)> {
    match region.to_ascii_uppercase().as_str() {
        "IT" => Some(("39", false)),
        "US" | "CA" => Some(("1", true)),
        "GB" => Some(("44", true)),
        "FR" => Some(("33", true)),
        "DE" => Some(("49", true)),
        "ES" => Some(("34", false)),
        "CH" => Some(("41", true)),
        _ => None,
    }
}

pub fn normalize_email(value: &str) -> String {
    value.trim().to_lowercase()
}

/// E.164 form, or `None` when the number cannot be placed in a country.
pub fn normalize_phone(value: &str, region: &str) -> Option<String> {
    let trimmed = value.trim();
    let digits: String = trimmed.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 7 {
        return None; // short codes and extensions identify no one
    }
    if trimmed.starts_with('+') {
        return Some(format!("+{digits}"));
    }
    if let Some(rest) = digits.strip_prefix("00") {
        return Some(format!("+{rest}"));
    }
    let (code, drops_zero) = calling_code(region)?;
    let national = if drops_zero { digits.trim_start_matches('0') } else { digits.as_str() };
    Some(format!("+{code}{national}"))
}

/// Last nine digits, the weak key used when no region is known.
pub fn phone_tail(value: &str) -> Option<String> {
    let digits: String = value.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() < 7 {
        return None;
    }
    Some(digits[digits.len().saturating_sub(9)..].to_string())
}

pub fn normalize_name(value: &str) -> String {
    let folded: String = value
        .nfd()
        .filter(|c| !('\u{300}'..='\u{36f}').contains(c) && !matches!(c, '\'' | '\u{2019}' | '`'))
        .map(|c| if c.is_alphanumeric() { c.to_lowercase().collect::<String>() } else { " ".into() })
        .collect();
    let mut tokens: Vec<&str> = folded
        .split_whitespace()
        .filter(|t| !TITLES.contains(t))
        .collect();
    tokens.sort_unstable();
    tokens.join(" ")
}

/// `"it-IT"` becomes `"IT"`; a locale without a region gives an empty string.
pub fn region_of(locale: &str) -> String {
    locale
        .split(['-', '_'])
        .skip(1)
        .find(|part| part.len() == 2 && part.chars().all(|c| c.is_ascii_alphabetic()))
        .map(|part| part.to_ascii_uppercase())
        .unwrap_or_default()
}

pub fn is_role_address(email: &str, extra: &[String]) -> bool {
    let address = normalize_email(email);
    let local = address.split('@').next().unwrap_or("");
    DEFAULT_ROLES.contains(&local) || extra.iter().any(|role| role.trim().eq_ignore_ascii_case(local))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_is_lowercased_but_dots_and_plus_tags_stay() {
        assert_eq!(normalize_email(" Mario.Rossi+x@Example.COM "), "mario.rossi+x@example.com");
    }

    #[test]
    fn numbers_too_short_to_identify_anyone_are_not_keys() {
        for short in ["12345", "123 456", "112"] {
            assert_eq!(normalize_phone(short, "IT"), None, "{short}");
        }
        assert_eq!(normalize_phone("123 4567", "IT"), Some("+391234567".into()));
    }

    #[test]
    fn phone_normalizes_to_e164_for_italy() {
        for input in ["333 1234567", "+39 333 123 4567", "0039 333 1234567"] {
            assert_eq!(normalize_phone(input, "IT"), Some("+393331234567".into()), "{input}");
        }
        assert_eq!(normalize_phone("02 1234567", "IT"), Some("+39021234567".into()));
    }

    #[test]
    fn phone_without_region_is_not_e164() {
        assert_eq!(normalize_phone("333 1234567", ""), None);
        assert_eq!(phone_tail("333 1234567"), Some("331234567".into()));
        assert_eq!(phone_tail("123"), None);
    }

    #[test]
    fn name_ignores_case_accents_punctuation_and_order() {
        let a = normalize_name("D'Angelo, Mario");
        assert_eq!(a, normalize_name("Mario dangelo"));
        assert_eq!(a, normalize_name("MARIO D\u{2019}ANGELO"));
        assert_eq!(normalize_name("Dott. Mario Rossi"), "mario rossi");
        assert_eq!(normalize_name("Zoë Müller"), normalize_name("MULLER zoe"));
    }

    #[test]
    fn empty_inputs_normalize_to_empty() {
        assert_eq!(normalize_name("  "), "");
        assert_eq!(normalize_email(""), "");
    }

    #[test]
    fn role_addresses_include_defaults_and_extras() {
        assert!(is_role_address("info@x.it", &[]));
        assert!(!is_role_address("mario@x.it", &[]));
        assert!(is_role_address("ciao@x.it", &["ciao".into()]));
    }

    #[test]
    fn region_of_reads_the_locale_subtag() {
        assert_eq!(region_of("it-IT"), "IT");
        assert_eq!(region_of("en_us"), "US");
        assert_eq!(region_of("it"), "");
        assert_eq!(region_of(""), "");
    }
}
