//! TLS for the Qamille server (spec D3, A1): the operating system's verifier, where company
//! certificate authorities arrive by policy, or the PEM authority the person chose for that
//! server. There is no way to skip verification.
use crate::application::{AppError, AppResult};
use std::sync::Arc;
use ureq::tls::{Certificate, PemItem, RootCerts, TlsConfig, parse_pem};

/// The TLS configuration for a server: `ca_pem` replaces the system roots when given.
pub fn config(ca_pem: Option<&str>) -> AppResult<TlsConfig> {
    let roots = match ca_pem.map(str::trim).filter(|pem| !pem.is_empty()) {
        None => RootCerts::PlatformVerifier,
        Some(pem) => {
            let certificates: Vec<Certificate<'static>> = parse_pem(pem.as_bytes())
                .filter_map(|item| match item {
                    Ok(PemItem::Certificate(certificate)) => Some(certificate),
                    _ => None,
                })
                .collect();
            if certificates.is_empty() {
                return Err(AppError::validation("The certificate authority file holds no PEM certificate"));
            }
            RootCerts::Specific(Arc::new(certificates))
        }
    };
    Ok(TlsConfig::builder().root_certs(roots).build())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_file_without_certificates_is_refused() {
        assert!(super::config(Some("not a certificate")).is_err());
        assert!(super::config(None).is_ok());
    }
}
