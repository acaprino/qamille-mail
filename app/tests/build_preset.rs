//! A Professional build with an invalid preset server must not build (spec A6). Heavy: it checks
//! the crate in a temporary target folder. Run at the wave gate:
//!   cargo test --jobs 2 --test build_preset -- --ignored
use std::process::Command;

#[test]
#[ignore = "heavy: cargo check in a temporary target folder"]
fn an_invalid_preset_server_fails_the_build() {
    let target = std::env::temp_dir().join(format!("qamille-build-preset-{}", std::process::id()));
    let run = |preset: &str| {
        Command::new(env!("CARGO"))
            .args(["check", "--jobs", "2", "--lib", "--no-default-features", "--manifest-path"])
            .arg(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml"))
            .env("CARGO_TARGET_DIR", &target)
            .env("QAMILLE_PRESET_SERVER", preset)
            .output()
            .expect("cargo runs")
    };
    let refused = run("http://calcolo.ufficio.it");
    let stderr = String::from_utf8_lossy(&refused.stderr);
    assert!(!refused.status.success(), "an http preset must fail the build");
    assert!(stderr.contains("QAMILLE_PRESET_SERVER is not a valid server address"), "{stderr}");
    let accepted = run("https://calcolo.ufficio.it");
    assert!(accepted.status.success(), "{}", String::from_utf8_lossy(&accepted.stderr));
    let _ = std::fs::remove_dir_all(&target);
}
