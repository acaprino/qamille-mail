//! The parser process (`qamille-server --parser-worker`, INV-11) and its frame protocol,
//! kept from the app's `--embedding-worker`: a little-endian u32 length, a JSON header, then
//! the raw bytes the header announces. Nothing is persisted; the process reads only what it is
//! sent and dies with its stdin.
//!
//! Document readers and image decoding are third-party code that can panic, loop or allocate
//! without bound on a malformed file. Here a panic becomes an ordinary per-request error
//! ([`catch_panic`]); a crash or a hang ends only this process, never the model.

use serde::{Deserialize, Serialize};
use std::io::{Read, Write};

/// Largest JSON header.
pub const MAX_FRAME: usize = 8 * 1024 * 1024;
/// Largest raw payload after a header (one attachment, spec D2).
pub const MAX_PAYLOAD: usize = 25 * 1024 * 1024;

#[derive(Serialize, Deserialize, Debug, Clone, PartialEq)]
pub struct Request {
    /// `extract` or `image`.
    pub kind: String,
    /// File name; only its extension chooses the reader.
    pub name: String,
    /// Raw bytes after the header.
    pub bytes: usize,
}

#[derive(Serialize, Deserialize, Debug, Default, Clone, PartialEq)]
pub struct Response {
    #[serde(default)]
    pub text: String,
    /// Image tokens of the decoded picture (`image` only).
    #[serde(default)]
    pub tokens: usize,
    /// Raw bytes after the header (`image`: the patches in wire form).
    #[serde(default)]
    pub bytes: usize,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn write_frame(output: &mut impl Write, data: &[u8]) -> std::io::Result<()> {
    if data.len() > MAX_FRAME { return Err(std::io::Error::other("Embedding frame exceeds its memory budget")); }
    output.write_all(&(data.len() as u32).to_le_bytes())?;
    output.write_all(data)?;
    output.flush()
}

pub fn read_frame(input: &mut impl Read) -> std::io::Result<Vec<u8>> {
    let mut size = [0; 4];
    input.read_exact(&mut size)?;
    let size = u32::from_le_bytes(size) as usize;
    if size > MAX_FRAME { return Err(std::io::Error::other("Embedding response exceeds its memory budget")); }
    let mut data = vec![0; size];
    input.read_exact(&mut data)?;
    Ok(data)
}

/// Writes a header and its payload.
pub fn write_message<T: Serialize>(output: &mut impl Write, header: &T, payload: &[u8]) -> std::io::Result<()> {
    if payload.len() > MAX_PAYLOAD { return Err(std::io::Error::other("Attachment exceeds the embedding memory budget (25 MiB)")); }
    let json = serde_json::to_vec(header).map_err(std::io::Error::other)?;
    write_frame(output, &json)?;
    output.write_all(payload)?;
    output.flush()
}

/// Reads a header and the payload it announces in `bytes`.
pub fn read_payload(input: &mut impl Read, bytes: usize) -> std::io::Result<Vec<u8>> {
    if bytes > MAX_PAYLOAD { return Err(std::io::Error::other("Attachment exceeds the embedding memory budget (25 MiB)")); }
    let mut data = vec![0; bytes];
    input.read_exact(&mut data)?;
    Ok(data)
}

pub const PANIC_PREFIX: &str = "Document parser failed: ";

/// Third-party parsers (PDF, image) panic on malformed input. A panic would end the process
/// and cost the whole request a retry, so it is reported as an ordinary per-request error.
pub fn catch_panic<T>(work: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(work)).unwrap_or_else(|payload| {
        let detail = payload.downcast_ref::<&str>().map(|text| text.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown failure".into());
        Err(format!("{PANIC_PREFIX}{}", detail.chars().take(160).collect::<String>()))
    })
}

/// One request of the parser process.
pub fn handle(request: &Request, data: &[u8]) -> (Response, Vec<u8>) {
    let result = catch_panic(|| -> Result<(Response, Vec<u8>), String> {
        match request.kind.as_str() {
            "extract" => Ok((Response { text: crate::extract::extract(&request.name, data)?, ..Default::default() }, Vec::new())),
            "image" => {
                let pixels = crate::vision::prepare(data)?;
                let bytes = pixels.to_bytes();
                Ok((Response { tokens: pixels.tokens, bytes: bytes.len(), ..Default::default() }, bytes))
            }
            _ => Err("Unknown embedding operation".into()),
        }
    });
    result.unwrap_or_else(|error| (Response { error: Some(error), ..Default::default() }, Vec::new()))
}

/// The parser process loop: it ends at the first malformed frame or when stdin closes.
/// The binary silences the panic hook first: a panic is already an answer.
pub fn serve(input: &mut impl Read, output: &mut impl Write) {
    while let Ok(frame) = read_frame(input) {
        let Ok(request) = serde_json::from_slice::<Request>(&frame) else { break };
        let Ok(data) = read_payload(input, request.bytes) else { break };
        let (response, payload) = handle(&request, &data);
        drop(data);
        if write_message(output, &response, &payload).is_err() { break; }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_panicking_parser_becomes_an_error_instead_of_ending_the_worker() {
        let silenced = std::panic::take_hook();
        std::panic::set_hook(Box::new(|_| {}));
        let failed = catch_panic::<()>(|| { assert_eq!(1, 2, "malformed document"); Ok(()) });
        let text = catch_panic(|| -> Result<&str, String> { panic!("{}", String::from("owned payload")) });
        let fine = catch_panic(|| Ok(7));
        std::panic::set_hook(silenced);
        assert!(failed.unwrap_err().starts_with(PANIC_PREFIX));
        assert!(text.unwrap_err().contains("owned payload"));
        assert_eq!(fine.unwrap(), 7);
    }
    #[test]
    fn ipc_rejects_oversized_frames_before_allocation() {
        assert!(read_frame(&mut std::io::Cursor::new(u32::MAX.to_le_bytes())).is_err());
        assert!(read_payload(&mut std::io::Cursor::new(Vec::new()), MAX_PAYLOAD + 1).is_err());
    }
    #[test]
    fn ipc_preserves_unicode_and_rejects_truncation() {
        let mut bytes = Vec::new();
        write_frame(&mut bytes, "città allegato".as_bytes()).unwrap();
        assert_eq!(read_frame(&mut std::io::Cursor::new(&bytes)).unwrap(), "città allegato".as_bytes());
        bytes.pop();
        assert!(read_frame(&mut std::io::Cursor::new(bytes)).is_err());
    }
    #[test]
    fn the_loop_answers_each_request_in_order() {
        let mut input = Vec::new();
        write_message(&mut input, &Request { kind: "extract".into(), name: "a.txt".into(), bytes: 5 }, b"hello").unwrap();
        write_message(&mut input, &Request { kind: "unknown".into(), name: String::new(), bytes: 0 }, b"").unwrap();
        let mut output = Vec::new();
        serve(&mut std::io::Cursor::new(input), &mut output);
        let mut reader = std::io::Cursor::new(output);
        let first: Response = serde_json::from_slice(&read_frame(&mut reader).unwrap()).unwrap();
        assert_eq!(first.text, "hello");
        let second: Response = serde_json::from_slice(&read_frame(&mut reader).unwrap()).unwrap();
        assert_eq!(second.error.as_deref(), Some("Unknown embedding operation"));
    }
}
