//! Origine della richiesta e connessione sicura.
//!
//! L'origine è l'indirizzo del socket. Solo se il socket è un proxy di
//! `trusted_proxies` vale l'ultimo indirizzo di `X-Forwarded-For`, e solo allora
//! `X-Forwarded-Proto: https` rende sicura la connessione. Il loopback è sicuro solo
//! se non è un proxy fidato: un proxy sullo stesso computer che inoltra una
//! richiesta in chiaro (`X-Forwarded-Proto: http`) non la rende sicura.

use std::net::{IpAddr, SocketAddr};

use axum::http::HeaderMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ClientOrigin {
    pub ip: IpAddr,
    pub secure: bool,
}

/// Ultimo valore di un'intestazione ripetibile e separata da virgole.
fn last_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get_all(name)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(','))
        .map(str::trim)
        .rfind(|v| !v.is_empty())
}

pub fn resolve(
    socket: SocketAddr,
    headers: &HeaderMap,
    trusted_proxies: &[IpAddr],
) -> ClientOrigin {
    resolve_with_plaintext(socket, headers, trusted_proxies, &[])
}

/// Come [`resolve`], ma un socket di `plaintext_from` (che non sia un proxy fidato)
/// è sicuro: conta solo l'indirizzo del socket, mai le intestazioni `X-Forwarded-*`.
pub fn resolve_with_plaintext(
    socket: SocketAddr,
    headers: &HeaderMap,
    trusted_proxies: &[IpAddr],
    plaintext_from: &[IpAddr],
) -> ClientOrigin {
    let socket_ip = socket.ip().to_canonical();
    let loopback = socket_ip.is_loopback();
    if !trusted_proxies.contains(&socket_ip) {
        return ClientOrigin {
            ip: socket_ip,
            secure: loopback || plaintext_from.contains(&socket_ip),
        };
    }
    let forwarded_ip = last_value(headers, "x-forwarded-for")
        .and_then(|v| v.parse::<IpAddr>().ok())
        .map(|ip| ip.to_canonical());
    // Dal proxy fidato la sicurezza viene solo da `X-Forwarded-Proto`, anche quando
    // il proxy è sul loopback.
    let https =
        last_value(headers, "x-forwarded-proto").is_some_and(|v| v.eq_ignore_ascii_case("https"));
    ClientOrigin {
        ip: forwarded_ip.unwrap_or(socket_ip),
        secure: https,
    }
}
