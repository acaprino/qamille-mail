use qamille_server::password::{self, PasswordPolicyError, TEMPORARY_ALPHABET, TEMPORARY_LEN};

#[test]
fn length_limits_count_characters_not_bytes() {
    assert_eq!(
        password::validate(&"a".repeat(9)),
        Err(PasswordPolicyError::TooShort)
    );
    assert_eq!(password::validate(&"a".repeat(10)), Ok(()));
    assert_eq!(password::validate(&"a".repeat(256)), Ok(()));
    assert_eq!(
        password::validate(&"a".repeat(257)),
        Err(PasswordPolicyError::TooLong)
    );

    // Caratteri accentati: due byte ciascuno, contano uno.
    assert_eq!(
        password::validate(&"è".repeat(9)),
        Err(PasswordPolicyError::TooShort)
    );
    assert_eq!(password::validate(&"è".repeat(10)), Ok(()));
    assert_eq!(password::validate(&"è".repeat(256)), Ok(()));
    assert_eq!(
        password::validate(&"è".repeat(257)),
        Err(PasswordPolicyError::TooLong)
    );

    assert_eq!(
        PasswordPolicyError::TooShort.to_string(),
        "La password deve avere almeno 10 caratteri."
    );
    assert_eq!(
        PasswordPolicyError::TooLong.to_string(),
        "La password può avere al più 256 caratteri."
    );
}

#[test]
fn hash_is_argon2id_phc_with_the_agreed_parameters() {
    let hash = password::hash("correct horse battery").unwrap();
    assert!(
        hash.starts_with("$argon2id$v=19$m=19456,t=2,p=1$"),
        "formato inatteso: {hash}"
    );
    assert!(!hash.contains("correct horse battery"));
    // Sale casuale: due hash della stessa password sono diversi.
    assert_ne!(hash, password::hash("correct horse battery").unwrap());
    assert!(password::verify(Some(&hash), "correct horse battery"));
    assert!(!password::verify(Some(&hash), "correct horse batterY"));
}

#[test]
fn verify_without_hash_is_false() {
    assert!(!password::verify(None, "qualunque password"));
    assert!(!password::verify(
        Some("non è un hash"),
        "qualunque password"
    ));
}

#[test]
fn temporary_passwords_use_the_unambiguous_alphabet() {
    let first = password::temporary();
    let second = password::temporary();
    for value in [&first, &second] {
        assert_eq!(value.len(), TEMPORARY_LEN);
        assert!(
            value.bytes().all(|b| TEMPORARY_ALPHABET.contains(&b)),
            "{value}"
        );
        assert!(!value.contains(['i', 'l', 'o', '0', '1']));
        assert_eq!(password::validate(value), Ok(()));
    }
    assert_ne!(first, second);
}

#[test]
fn the_most_common_passwords_are_refused() {
    for common in ["password123", "Password123", "  QWERTYUIOP ", "1234567890", "iloveyou123", "1q2w3e4r5t"] {
        assert_eq!(password::validate(common), Err(PasswordPolicyError::TooCommon), "{common}");
    }
    // The length rules come first, and an ordinary password goes through.
    assert_eq!(password::validate("123456"), Err(PasswordPolicyError::TooShort));
    assert_eq!(password::validate("una password lunga"), Ok(()));
    assert_eq!(password::validate("nuova password 1"), Ok(()));
    assert!(PasswordPolicyError::TooCommon.to_string().contains("più usate"));
}
