use std::net::{IpAddr, SocketAddr};

use axum::http::{HeaderMap, HeaderValue};
use qamille_server::origin::{resolve, resolve_with_plaintext};

fn headers(pairs: &[(&'static str, &'static str)]) -> HeaderMap {
    let mut map = HeaderMap::new();
    for (name, value) in pairs {
        map.append(*name, HeaderValue::from_static(value));
    }
    map
}

fn sock(s: &str) -> SocketAddr {
    s.parse().unwrap()
}

fn ip(s: &str) -> IpAddr {
    s.parse().unwrap()
}

#[test]
fn forwarded_headers_are_ignored_without_a_trusted_proxy() {
    let h = headers(&[
        ("x-forwarded-for", "198.51.100.7"),
        ("x-forwarded-proto", "https"),
    ]);
    let origin = resolve(sock("203.0.113.9:4000"), &h, &[]);
    assert_eq!(origin.ip, ip("203.0.113.9"));
    assert!(!origin.secure);
}

#[test]
fn trusted_proxy_uses_the_last_forwarded_address() {
    let trusted = [ip("10.0.0.2")];
    let h = headers(&[
        ("x-forwarded-for", "1.2.3.4, 198.51.100.7"),
        ("x-forwarded-for", "198.51.100.8"),
        ("x-forwarded-proto", "https"),
    ]);
    let origin = resolve(sock("10.0.0.2:4000"), &h, &trusted);
    assert_eq!(origin.ip, ip("198.51.100.8"));
    assert!(origin.secure);
}

#[test]
fn trusted_proxy_over_plain_http_is_not_secure() {
    let trusted = [ip("10.0.0.2")];
    let h = headers(&[
        ("x-forwarded-for", "198.51.100.7"),
        ("x-forwarded-proto", "http"),
    ]);
    let origin = resolve(sock("10.0.0.2:4000"), &h, &trusted);
    assert_eq!(origin.ip, ip("198.51.100.7"));
    assert!(!origin.secure);

    // Senza X-Forwarded-For valido resta l'indirizzo del proxy.
    let origin = resolve(
        sock("10.0.0.2:4000"),
        &headers(&[("x-forwarded-for", "garbage")]),
        &trusted,
    );
    assert_eq!(origin.ip, ip("10.0.0.2"));
}

#[test]
fn loopback_is_secure_and_mapped_addresses_are_canonical() {
    let origin = resolve(sock("127.0.0.1:5000"), &HeaderMap::new(), &[]);
    assert!(origin.secure);
    let origin = resolve(sock("[::1]:5000"), &HeaderMap::new(), &[]);
    assert!(origin.secure);
    let origin = resolve(sock("[::ffff:203.0.113.9]:5000"), &HeaderMap::new(), &[]);
    assert_eq!(origin.ip, ip("203.0.113.9"));
    assert!(!origin.secure);
}

#[test]
fn a_trusted_proxy_on_loopback_is_secure_only_with_forwarded_https() {
    let trusted = [ip("127.0.0.1")];
    // Il proxy sullo stesso computer inoltra una richiesta in chiaro: non è sicura.
    let h = headers(&[
        ("x-forwarded-for", "198.51.100.7"),
        ("x-forwarded-proto", "http"),
    ]);
    let origin = resolve(sock("127.0.0.1:4000"), &h, &trusted);
    assert_eq!(origin.ip, ip("198.51.100.7"));
    assert!(!origin.secure);
    // Senza X-Forwarded-Proto nemmeno.
    let origin = resolve(sock("127.0.0.1:4000"), &HeaderMap::new(), &trusted);
    assert!(!origin.secure);
    // Con https sì.
    let h = headers(&[
        ("x-forwarded-for", "198.51.100.7"),
        ("x-forwarded-proto", "https"),
    ]);
    assert!(resolve(sock("127.0.0.1:4000"), &h, &trusted).secure);
    // Un loopback che non è un proxy fidato resta sicuro.
    assert!(resolve(sock("127.0.0.1:4000"), &HeaderMap::new(), &[ip("10.0.0.2")]).secure);
}

#[test]
fn an_address_in_plaintext_from_is_secure() {
    let o = resolve_with_plaintext(sock("172.30.88.1:5000"), &HeaderMap::new(), &[], &[ip("172.30.88.1")]);
    assert!(o.secure && o.ip == ip("172.30.88.1"));
}

#[test]
fn an_address_not_in_plaintext_from_is_not_secure() {
    let o = resolve_with_plaintext(sock("172.30.88.9:5000"), &HeaderMap::new(), &[], &[ip("172.30.88.1")]);
    assert!(!o.secure && o.ip == ip("172.30.88.9"));
}

#[test]
fn forwarded_headers_never_grant_plaintext() {
    let h = headers(&[("x-forwarded-for", "172.30.88.1")]);
    let o = resolve_with_plaintext(sock("203.0.113.9:5000"), &h, &[], &[ip("172.30.88.1")]);
    assert!(!o.secure);
    assert_eq!(o.ip, ip("203.0.113.9"));
}

#[test]
fn a_trusted_proxy_is_never_made_secure_by_plaintext_from() {
    let h = headers(&[("x-forwarded-proto", "http")]);
    let o = resolve_with_plaintext(sock("10.0.0.2:5000"), &h, &[ip("10.0.0.2")], &[ip("10.0.0.2")]);
    assert!(!o.secure);
}
