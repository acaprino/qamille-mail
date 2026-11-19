//! The revocation of a Google token (`oauth::revoke`) against a scripted endpoint.
#[path = "support/http_fake.rs"]
mod http_fake;

use http_fake::{Fake, Reply, reply};
use qamille::oauth::{forget_access, revoke};

#[test]
fn the_token_is_posted_as_a_form_and_success_is_reported() {
    let fake = Fake::start(vec![reply(200, "{}")]);
    revoke(&format!("{}/revoke", fake.url), "1//the-token").unwrap();
    let seen = fake.requests();
    assert_eq!(seen[0].request_line, "POST /revoke HTTP/1.1");
    assert_eq!(seen[0].body, "token=1%2F%2Fthe-token");
    assert!(seen[0].header("content-type").unwrap().starts_with("application/x-www-form-urlencoded"));
    assert!(seen[0].header("authorization").is_none(), "no other credential goes with it");
}

#[test]
fn a_token_google_does_not_know_counts_as_revoked() {
    let fake = Fake::start(vec![reply(400, r#"{"error":"invalid_token","error_description":"Token expired or revoked"}"#)]);
    assert!(revoke(&format!("{}/revoke", fake.url), "1//gone").is_ok());
}

#[test]
fn a_refusal_or_a_server_error_is_a_failure_without_the_token_in_the_text() {
    for (status, body) in [(400, r#"{"error":"invalid_request"}"#), (500, "oops"), (403, "")] {
        let fake = Fake::start(vec![reply(status, body)]);
        let error = revoke(&format!("{}/revoke", fake.url), "1//secret-token").unwrap_err();
        assert_eq!(error, "Google did not revoke the access");
    }
    let error = revoke("http://127.0.0.1:1/revoke", "1//secret-token").unwrap_err();
    assert!(!error.contains("secret-token"), "{error}");
}

#[test]
fn a_redirect_is_not_followed() {
    let other = Fake::start(vec![reply(200, "{}")]);
    let redirect = Reply { status: 307, headers: vec![("Location".into(), format!("{}/x", other.url))], body: String::new() };
    let fake = Fake::start(vec![redirect]);
    assert!(revoke(&format!("{}/revoke", fake.url), "1//secret-token").is_err());
    assert!(other.requests().is_empty(), "the token did not follow the redirect");
}

#[test]
fn forgetting_an_access_token_of_an_unknown_address_is_harmless() {
    forget_access("nessuno@example.com");
}
