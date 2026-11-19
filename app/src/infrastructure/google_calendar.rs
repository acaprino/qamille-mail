//! Google Calendar REST adapter. No filesystem, UI, or credential persistence.
use crate::application::{AppError, AppResult};
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

pub fn encode(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

pub struct GoogleCalendar {
    token: String,
    base: String,
    authorization: Option<Arc<dyn Fn() -> AppResult<()> + Send + Sync>>,
}
impl GoogleCalendar {
    pub fn new(token: String) -> Self {
        Self {
            token,
            base: "https://www.googleapis.com/calendar/v3".into(),
            authorization: None,
        }
    }
    pub fn with_authorization(
        mut self,
        authorization: Arc<dyn Fn() -> AppResult<()> + Send + Sync>,
    ) -> Self {
        self.authorization = Some(authorization);
        self
    }
    pub fn request(
        &self,
        method: &str,
        path: &str,
        params: &[(&str, String)],
        body: Option<&Value>,
        etag: Option<&str>,
    ) -> AppResult<Value> {
        if let Some(authorize) = &self.authorization {
            authorize()?;
        }
        let query = params
            .iter()
            .map(|(k, v)| format!("{}={}", encode(k), encode(v)))
            .collect::<Vec<_>>()
            .join("&");
        let url = format!(
            "{}{}{}{}",
            self.base,
            path,
            if query.is_empty() { "" } else { "?" },
            query
        );
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .build()
            .into();
        let auth = format!("Bearer {}", self.token);
        let empty = serde_json::json!({});
        let mut response = match method {
            "GET" => agent.get(&url).header("Authorization", &auth).call(),
            "POST" => agent
                .post(&url)
                .header("Authorization", &auth)
                .send_json(body.unwrap_or(&empty)),
            "PATCH" => agent
                .patch(&url)
                .header("Authorization", &auth)
                .header("If-Match", etag.unwrap_or("*"))
                .send_json(body.unwrap_or(&empty)),
            "PUT" => agent
                .put(&url)
                .header("Authorization", &auth)
                .header("If-Match", etag.unwrap_or("*"))
                .send_json(body.unwrap_or(&empty)),
            "DELETE" => agent
                .delete(&url)
                .header("Authorization", &auth)
                .header("If-Match", etag.unwrap_or("*"))
                .call(),
            _ => return Err(AppError::validation("Unsupported calendar HTTP method")),
        }
        .map_err(|e| {
            AppError::new(
                "provider_unavailable",
                format!("Calendar request did not complete: {e}"),
            )
        })?;
        let status = response.status().as_u16();
        if status == 204 {
            return Ok(empty);
        }
        let value: Value = response
            .body_mut()
            .read_json()
            .map_err(|_| AppError::external("Calendar provider returned an unreadable response"))?;
        if status >= 400 {
            let code = match status {
                401 => "sign_in_required",
                403 => "calendar_permission_denied",
                404 => "not_found",
                409 => "already_exists",
                410 => "sync_token_expired",
                412 => "revision_conflict",
                429 => "rate_limited",
                _ => "calendar_provider_error",
            };
            return Err(AppError::new(
                code,
                value["error"]["message"]
                    .as_str()
                    .unwrap_or("Google Calendar rejected the operation"),
            ));
        }
        Ok(value)
    }
    pub fn event_path(calendar: &str, event: Option<&str>) -> String {
        let path = format!("/calendars/{}/events", encode(calendar));
        match event {
            Some(id) => format!("{path}/{}", encode(id)),
            None => path,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    #[test]
    fn calendar_and_event_identifiers_are_encoded_as_path_components() {
        assert_eq!(
            GoogleCalendar::event_path("a/b@example.com", Some("../x")),
            "/calendars/a%2Fb%40example.com/events/..%2Fx"
        );
    }
    #[test]
    fn remote_conflicts_are_not_reported_as_success() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let mut buf = [0; 4096];
            let size = s.read(&mut buf).unwrap();
            let request = String::from_utf8_lossy(&buf[..size]).to_string();
            let body = r#"{"error":{"message":"Changed elsewhere"}}"#;
            write!(s,"HTTP/1.1 412 Precondition Failed\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            request
        });
        let gateway = GoogleCalendar {
            token: "synthetic-token".into(),
            base,
            authorization: None,
        };
        let err = gateway
            .request("GET", "/calendars/a/events/x", &[], None, None)
            .unwrap_err();
        assert_eq!(err.code, "revision_conflict");
        assert!(server.join().unwrap().contains("Bearer synthetic-token"));
    }
    #[test]
    fn patch_sends_conditional_revision_and_notification_policy() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut bytes = Vec::new();
            let mut buffer = [0; 4096];
            loop {
                let n = stream.read(&mut buffer).unwrap();
                assert!(n > 0);
                bytes.extend_from_slice(&buffer[..n]);
                let request = String::from_utf8_lossy(&bytes);
                if let Some(end) = request.find("\r\n\r\n") {
                    let length = request[..end].lines().find_map(|line| {
                        line.to_ascii_lowercase()
                            .strip_prefix("content-length: ")
                            .and_then(|v| v.trim().parse::<usize>().ok())
                    });
                    let complete = if let Some(length) = length {
                        bytes.len() >= end + 4 + length
                    } else if request[..end]
                        .to_ascii_lowercase()
                        .contains("transfer-encoding: chunked")
                    {
                        bytes[end + 4..].ends_with(b"0\r\n\r\n")
                    } else {
                        true
                    };
                    if complete {
                        break;
                    }
                }
            }
            let body = r#"{"id":"e","etag":"v2"}"#;
            write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).unwrap();
            // The JSON sender may split even a JSON property across HTTP
            // chunks. Assertions inspect decoded content, not chunk boundaries.
            let end = bytes
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .unwrap()
                + 4;
            if String::from_utf8_lossy(&bytes[..end])
                .to_ascii_lowercase()
                .contains("transfer-encoding: chunked")
            {
                let mut position = end;
                let mut body = Vec::new();
                loop {
                    let line_end = position
                        + bytes[position..]
                            .windows(2)
                            .position(|part| part == b"\r\n")
                            .unwrap();
                    let size = usize::from_str_radix(
                        std::str::from_utf8(&bytes[position..line_end]).unwrap(),
                        16,
                    )
                    .unwrap();
                    if size == 0 {
                        break;
                    }
                    position = line_end + 2;
                    body.extend_from_slice(&bytes[position..position + size]);
                    position += size + 2;
                }
                bytes.truncate(end);
                bytes.extend(body);
            }
            String::from_utf8(bytes).unwrap()
        });
        let api = GoogleCalendar {
            token: "synthetic".into(),
            base,
            authorization: None,
        };
        let result = api
            .request(
                "PATCH",
                "/calendars/primary/events/e",
                &[("sendUpdates", "all".into())],
                Some(&serde_json::json!({"summary":"Changed"})),
                Some("\"v1\""),
            )
            .unwrap();
        assert_eq!(result["etag"], "v2");
        let sent = server.join().unwrap();
        assert!(sent.starts_with("PATCH /calendars/primary/events/e?sendUpdates=all "));
        assert!(sent.to_ascii_lowercase().contains("if-match: \"v1\""));
        let body = sent
            .split_once("\r\n\r\n")
            .expect("Captured HTTP request has headers")
            .1;
        let payload: Value = serde_json::from_str(body).unwrap_or_else(|error| {
            panic!("Captured PATCH body is not JSON: {error}; request={sent:?}")
        });
        assert_eq!(
            payload,
            serde_json::json!({"summary":"Changed"}),
            "Captured request: {sent:?}"
        );
    }
    #[test]
    fn every_provider_effect_revalidates_authorization_before_network() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let checks = Arc::new(AtomicUsize::new(0));
        let count = checks.clone();
        let api = GoogleCalendar::new("unused".into()).with_authorization(Arc::new(move || {
            count.fetch_add(1, Ordering::SeqCst);
            Err(AppError::new("permission_denied", "Revoked"))
        }));
        for method in ["GET", "POST"] {
            assert_eq!(
                api.request(method, "/unused", &[], None, None)
                    .unwrap_err()
                    .code,
                "permission_denied"
            );
        }
        assert_eq!(checks.load(Ordering::SeqCst), 2);
    }
}
