mod support;

use std::process::{Command, Output};

use qamille_server::password::TEMPORARY_ALPHABET;
use support::TempDir;

fn run(dir: &TempDir, args: &[&str]) -> Output {
    run_with(dir, args, &[])
}

fn run_with(dir: &TempDir, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qamille-server"));
    cmd.args(args)
        .env_clear()
        .env("QAMILLE_DATA_DIR", dir.path())
        .current_dir(dir.path());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("avvio del binario")
}

fn stdout(o: &Output) -> String {
    String::from_utf8_lossy(&o.stdout).into_owned()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn create_prints_only_the_temporary_password() {
    let dir = TempDir::new();
    let out = run(
        &dir,
        &[
            "user",
            "create",
            "mario",
            "--email",
            "mario@example.com",
            "--max-devices",
            "2",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = stdout(&out);
    assert!(text.ends_with('\n'));
    let password = text.strip_suffix('\n').unwrap();
    assert_eq!(password.len(), 20, "{text:?}");
    assert!(password.bytes().all(|b| TEMPORARY_ALPHABET.contains(&b)));
    assert!(!stderr(&out).contains(password));
    assert!(dir.path().join("server.db").is_file());

    // Duplicato: codice 1, stdout vuoto.
    let out = run(&dir, &["user", "create", "MARIO"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(out.stdout.is_empty());
    assert!(stderr(&out).contains("username"), "{}", stderr(&out));
}

#[test]
fn the_other_user_commands() {
    let dir = TempDir::new();
    assert_eq!(
        run(&dir, &["user", "create", "mario"]).status.code(),
        Some(0)
    );

    let out = run(&dir, &["user", "list"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("mario"));
    assert!(stdout(&out).contains("provvisoria"));

    let out = run(&dir, &["user", "reset-password", "Mario"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_eq!(stdout(&out).trim_end().len(), 20);
    assert_eq!(stdout(&out).lines().count(), 1);

    let out = run(&dir, &["user", "max-devices", "mario", "5"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&run(&dir, &["user", "list"])).contains("\t5\t"));
    assert_eq!(
        run(&dir, &["user", "max-devices", "mario", "default"])
            .status
            .code(),
        Some(0)
    );
    assert_eq!(
        run(&dir, &["user", "max-devices", "mario", "molti"])
            .status
            .code(),
        Some(2)
    );

    assert_eq!(
        run(&dir, &["user", "disable", "mario"]).status.code(),
        Some(0)
    );
    assert!(stdout(&run(&dir, &["user", "list"])).contains("disattivato"));
    assert_eq!(
        run(&dir, &["user", "enable", "mario"]).status.code(),
        Some(0)
    );
    assert!(!stdout(&run(&dir, &["user", "list"])).contains("disattivato"));

    let out = run(&dir, &["user", "devices", "mario"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(stdout(&out).lines().count(), 1, "solo l'intestazione");

    let out = run(&dir, &["user", "revoke-device", "mario", "inesistente"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("Dispositivo non trovato"));

    assert_eq!(
        run(&dir, &["user", "disable", "nessuno"]).status.code(),
        Some(1)
    );

    // La cancellazione vuole --yes.
    assert_eq!(
        run(&dir, &["user", "delete", "mario"]).status.code(),
        Some(2)
    );
    assert!(stdout(&run(&dir, &["user", "list"])).contains("mario"));
    assert_eq!(
        run(&dir, &["user", "delete", "mario", "--yes"])
            .status
            .code(),
        Some(0)
    );
    assert!(!stdout(&run(&dir, &["user", "list"])).contains("mario"));
}

#[test]
fn serve_local_is_a_usage_error() {
    let dir = TempDir::new();
    let out = run(&dir, &["serve", "--local"]);
    assert_eq!(out.status.code(), Some(2));
    assert!(stderr(&out).contains("opzione sconosciuta per serve: --local"), "{}", stderr(&out));
}

#[test]
fn the_help_no_longer_mentions_serve_local() {
    let dir = TempDir::new();
    assert!(!stdout(&run(&dir, &["--help"])).contains("--local"));
}

#[test]
fn weak_admin_key_stops_the_server() {
    let dir = TempDir::new();
    let out = run_with(
        &dir,
        &["serve"],
        &[
            ("QAMILLE_ADMIN_KEY", "troppo-corta"),
            ("QAMILLE_BIND", "127.0.0.1:0"),
        ],
    );
    assert_eq!(out.status.code(), Some(1));
    assert!(stderr(&out).contains("QAMILLE_ADMIN_KEY"));
    assert!(!stderr(&out).contains("troppo-corta"));
}

#[test]
fn usage_errors_and_help() {
    let dir = TempDir::new();
    assert_eq!(run(&dir, &["boh"]).status.code(), Some(2));
    assert_eq!(run(&dir, &["user"]).status.code(), Some(2));
    assert_eq!(run(&dir, &["user", "create"]).status.code(), Some(2));
    assert_eq!(
        run(&dir, &["user", "create", "mario", "--nope"])
            .status
            .code(),
        Some(2)
    );
    let out = run(&dir, &["--help"]);
    assert_eq!(out.status.code(), Some(0));
    assert!(stdout(&out).contains("user create"));
}

#[test]
fn config_flag_selects_the_file() {
    let dir = TempDir::new();
    let other = TempDir::new();
    let file = dir.path().join("altro.toml");
    std::fs::write(
        &file,
        format!("data_dir = {:?}\n", other.path().to_str().unwrap()),
    )
    .unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_qamille-server"));
    let out = cmd
        .args([
            "--config",
            file.to_str().unwrap(),
            "user",
            "create",
            "mario",
        ])
        .env_clear()
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(other.path().join("server.db").is_file());
}
