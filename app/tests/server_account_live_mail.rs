//! The app's sign-up and password reset against a running qamille-server whose SMTP goes to a
//! local sink that appends every message to QAMILLE_LIVE_MAILBOX (manual check, access wave 3).
use qamille::{
    application::server_account::{Credentials, LoginOutcome, RemoteEndpoint, ServerAccount, ServerAddress, SignUp},
    infrastructure::server_account::HttpServerAccount,
};
use std::time::{Duration, Instant};

/// The newest 8-symbol code in the mailbox after `seen` messages.
fn code_after(mailbox: &str, seen: usize) -> (String, usize) {
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        let text = std::fs::read_to_string(mailbox).unwrap_or_default();
        let messages: Vec<&str> = text.split("=====").filter(|m| !m.trim().is_empty()).collect();
        if messages.len() > seen {
            let body = messages[messages.len() - 1];
            let code = body
                .split(|c: char| !c.is_ascii_alphanumeric())
                .find(|w| w.len() == 8 && w.chars().all(|c| "abcdefghjkmnpqrstuvwxyz23456789".contains(c)))
                .unwrap_or_else(|| panic!("no code in: {body}"));
            return (code.to_string(), messages.len());
        }
        assert!(Instant::now() < deadline, "no mail arrived");
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
#[ignore = "needs a running qamille-server with sign-up and a local SMTP sink"]
fn sign_up_and_reset_from_the_app_against_the_real_server() {
    let url = std::env::var("QAMILLE_LIVE_SERVER").unwrap();
    let mailbox = std::env::var("QAMILLE_LIVE_MAILBOX").unwrap();
    let http = HttpServerAccount;
    let address = ServerAddress { url: url.clone(), ca_pem: None };
    let info = http.info(&address).unwrap();
    assert!(info.signup && info.password_reset, "{info:?}");
    http.signup_start(&address, "anna@example.com").unwrap();
    let (code, seen) = code_after(&mailbox, 0);
    let wrong = SignUp { email: "anna@example.com", code: "aaaaaaaa", password: "una password lunga", username: None, device_name: "Prova", accepted_terms: None };
    assert_eq!(http.signup_confirm(&address, &wrong).err().unwrap().code, "invalid_code");
    let signup = SignUp { email: "anna@example.com", code: &code, password: "una password lunga", username: None, device_name: "Prova", accepted_terms: None };
    let grant = http.signup_confirm(&address, &signup).unwrap();
    println!("signed up as {}", grant.username);
    let endpoint = RemoteEndpoint { url: url.clone(), device_key: grant.device_key, ca_pem: None, timeout: Duration::from_secs(10) };
    assert_eq!(http.devices(&endpoint).unwrap().len(), 1);
    // EX-21: the same answer for a taken email; its mail says to sign in instead.
    http.signup_start(&address, "anna@example.com").unwrap();
    let _ = code_after(&mailbox, seen);
    // Reset: every device signed out, the new password signs in.
    http.reset_start(&address, "anna@example.com").unwrap();
    let (reset_code, _) = code_after(&mailbox, seen + 1);
    http.reset_confirm(&address, "anna@example.com", &reset_code, "una password diversa").unwrap();
    assert_eq!(http.devices(&endpoint).err().unwrap().code, "unauthorized", "EX-15: the old key is out");
    let login = Credentials { identifier: "anna@example.com", password: "una password diversa", device_name: "Prova 2" };
    assert!(matches!(http.login(&address, &login).unwrap(), LoginOutcome::Signed(_)));
}
