//! A fake `llama-server` for the tests of the built-in LLM supervisor (`llm_builtin.rs`). Never part
//! of the Docker image or of a release: it exists only to be started by tests.
//!
//! It accepts **only** the arguments the supervisor passes (`child_args`) plus the `--fake-*`
//! ones, and exits with 2 on anything else (so a typo in the supervisor shows up). The key comes
//! from `LLAMA_API_KEY` (missing: exit 3). As in the real program (checked on b12081), `GET /health`
//! is public and every other route wants `Authorization: Bearer <key>` (`401` otherwise).

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Default)]
struct Options {
    model: Option<String>,
    host: String,
    port: u16,
    report: Option<String>,
    never_ready: bool,
    exit_after: Option<u32>,
    die_after_ms: Option<u64>,
    delay_ms: u64,
    public_all: bool,
    crash_on_start: bool,
    log_prompt: bool,
    echo: bool,
    error_lines: u32,
    host_dir: Option<String>,
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let mut options = Options { host: "127.0.0.1".to_owned(), ..Options::default() };
    let mut at = 0;
    let value = |at: &mut usize| -> String {
        *at += 1;
        argv.get(*at).cloned().unwrap_or_else(|| std::process::exit(2))
    };
    while at < argv.len() {
        match argv[at].as_str() {
            "--model" => options.model = Some(value(&mut at)),
            "--host" => options.host = value(&mut at),
            "--port" => options.port = value(&mut at).parse().unwrap_or_else(|_| std::process::exit(2)),
            "--ctx-size" | "--parallel" | "--cache-ram" | "--log-verbosity" => {
                let _ = value(&mut at);
            }
            "--no-slots" | "--jinja" | "--no-webui" => {}
            "--fake-report" => options.report = Some(value(&mut at)),
            "--fake-never-ready" => options.never_ready = true,
            "--fake-exit-after" => options.exit_after = value(&mut at).parse().ok(),
            "--fake-die-after-ms" => options.die_after_ms = value(&mut at).parse().ok(),
            "--fake-delay-ms" => options.delay_ms = value(&mut at).parse().unwrap_or(0),
            "--fake-public-all" => options.public_all = true,
            "--fake-crash-on-start" => options.crash_on_start = true,
            "--fake-log-prompt" => options.log_prompt = true,
            "--fake-echo" => options.echo = true,
            "--fake-error-lines" => options.error_lines = value(&mut at).parse().unwrap_or(0),
            "--fake-host" => options.host_dir = Some(value(&mut at)),
            _ => std::process::exit(2),
        }
        at += 1;
    }
    if let Some(dir) = options.host_dir.clone() {
        host_mode(std::path::Path::new(&dir));
    }
    let Ok(key) = std::env::var("LLAMA_API_KEY") else {
        std::process::exit(3);
    };
    if let Some(ms) = options.die_after_ms {
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(ms));
            std::process::exit(1);
        });
    }
    if options.crash_on_start {
        std::process::exit(17);
    }
    if options.log_prompt {
        eprintln!("error: finto");
    }
    for n in 0..options.error_lines {
        eprintln!("error: riga di avvio numero {n}");
    }
    let report = Arc::new(Report::new(&argv, &options));
    let listener = if options.never_ready {
        loop {
            std::thread::sleep(Duration::from_secs(3600));
        }
    } else {
        match TcpListener::bind((options.host.as_str(), options.port)) {
            Ok(listener) => listener,
            Err(_) => std::process::exit(1),
        }
    };
    report.write();
    let chats = Arc::new(AtomicU32::new(0));
    let options = Arc::new(options);
    for stream in listener.incoming().flatten() {
        let (key, report, chats, options) = (key.clone(), report.clone(), chats.clone(), options.clone());
        std::thread::spawn(move || serve(stream, &key, &report, &chats, &options));
    }
}

/// What the report file says about this process.
struct Report {
    path: Option<String>,
    head: String,
    requests: Mutex<Vec<String>>,
}

fn json_string(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

impl Report {
    fn new(argv: &[String], options: &Options) -> Report {
        let pid = std::process::id();
        let names: Vec<String> = std::env::vars_os().map(|(name, _)| json_string(&name.to_string_lossy())).collect();
        let args: Vec<String> = argv.iter().map(|a| json_string(a)).collect();
        let head = format!(
            "\"pid\":{pid},\"host\":{},\"port\":{},\"argv\":[{}],\"env\":[{}],\"model\":{}",
            json_string(&options.host),
            options.port,
            args.join(","),
            names.join(","),
            json_string(options.model.as_deref().unwrap_or(""))
        );
        let path = options.report.as_ref().map(|p| p.replace("{pid}", &pid.to_string()));
        Report { path, head, requests: Mutex::new(Vec::new()) }
    }

    fn write(&self) {
        let Some(path) = &self.path else { return };
        let requests = self.requests.lock().unwrap().join(",");
        let text = format!("{{{},\"requests\":[{requests}]}}", self.head);
        let _ = std::fs::write(path, text);
    }

    fn record(&self, method: &str, path: &str, auth: bool) {
        self.requests
            .lock()
            .unwrap()
            .push(format!("{{\"method\":{},\"path\":{},\"auth\":{auth}}}", json_string(method), json_string(path)));
        self.write();
    }
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let reason = match status {
        200 => "OK",
        401 => "Unauthorized",
        _ => "Not Found",
    };
    let head = format!("HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(body.as_bytes());
    let _ = stream.flush();
    let _ = stream.shutdown(std::net::Shutdown::Write);
}

fn serve(mut stream: TcpStream, key: &str, report: &Report, chats: &AtomicU32, options: &Options) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone().unwrap());
    let mut first = String::new();
    if reader.read_line(&mut first).unwrap_or(0) == 0 {
        return;
    }
    let mut parts = first.split_whitespace();
    let (method, path) = (parts.next().unwrap_or("").to_owned(), parts.next().unwrap_or("").to_owned());
    let (mut length, mut authorization) = (0_usize, None);
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line).unwrap_or(0) == 0 || line.trim_end().is_empty() {
            break;
        }
        let lower = line.to_ascii_lowercase();
        if let Some(v) = lower.strip_prefix("content-length:") {
            length = v.trim().parse().unwrap_or(0);
        }
        if lower.starts_with("authorization:") {
            authorization = Some(line["authorization:".len()..].trim().to_owned());
        }
    }
    let mut body = vec![0_u8; length];
    let _ = reader.read_exact(&mut body);
    let body = String::from_utf8_lossy(&body).into_owned();
    report.record(&method, &path, authorization.is_some());
    let allowed = options.public_all || path == "/health" || authorization.as_deref() == Some(&format!("Bearer {key}"));
    if !allowed {
        return respond(&mut stream, 401, "{\"error\":{\"message\":\"Invalid API Key\",\"type\":\"authentication_error\",\"code\":401}}");
    }
    match (method.as_str(), path.as_str()) {
        ("GET", "/health") => respond(&mut stream, 200, "{\"status\":\"ok\"}"),
        ("POST", "/tokenize") => respond(&mut stream, 200, "{\"tokens\":[1]}"),
        ("POST", "/v1/chat/completions") => {
            if options.log_prompt {
                eprintln!("prompt: {body}");
            }
            let count = chats.fetch_add(1, Ordering::SeqCst) + 1;
            if options.exit_after.is_some_and(|n| count >= n) {
                // Dies without answering: the request in flight is cut.
                std::process::exit(0);
            }
            if options.delay_ms > 0 {
                std::thread::sleep(Duration::from_millis(options.delay_ms));
            }
            let content = if options.echo { body } else { "ok".to_owned() };
            let answer = format!(
                "{{\"choices\":[{{\"index\":0,\"message\":{{\"role\":\"assistant\",\"content\":{}}},\"finish_reason\":\"stop\"}}],\"usage\":{{\"prompt_tokens\":3,\"completion_tokens\":1,\"total_tokens\":4}}}}",
                json_string(&content)
            );
            respond(&mut stream, 200, &answer);
        }
        _ => respond(&mut stream, 404, "{}"),
    }
}

/// "Server" mode, for the test that kills its host: this process owns a supervisor whose child is
/// another copy of this program, writes both pids to `<dir>/host.json` and waits to be killed.
fn host_mode(dir: &std::path::Path) -> ! {
    use qamille_server::llm_builtin::{BuiltinSettings, LlmSupervisor, SupervisorDeps};
    let model = dir.join("model.gguf");
    std::fs::write(&model, b"GGUF fake").unwrap_or_else(|_| std::process::exit(5));
    let program = std::env::current_exe().unwrap_or_else(|_| std::process::exit(5));
    let settings = BuiltinSettings::new(
        Arc::new(move || Ok(program.clone())),
        Arc::new(move || Ok(model.clone())),
        "llm-medio".to_owned(),
        4096,
        1,
        Duration::from_secs(3600),
        0,
    );
    let supervisor = LlmSupervisor::new(settings, SupervisorDeps::default());
    let _endpoint = supervisor.endpoint().unwrap_or_else(|_| std::process::exit(6));
    let pids = format!("{{\"host\":{},\"child\":{}}}", std::process::id(), supervisor.pid().unwrap_or(0));
    let _ = std::fs::write(dir.join("host.json"), pids);
    loop {
        std::thread::sleep(Duration::from_secs(3600));
    }
}
