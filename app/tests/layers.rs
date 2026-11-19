use std::path::Path;

fn sources(dir: &Path) -> Vec<String> {
    std::fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "rs"))
        .map(|path| std::fs::read_to_string(path).unwrap())
        .collect()
}

#[test]
fn domain_does_not_depend_on_io_or_desktop_frameworks() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/domain");
    for source in sources(&dir) {
        for forbidden in [
            "std::fs",
            "std::net",
            "crate::infrastructure",
            "crate::application",
            "use eframe",
            "use tauri",
            "use crate::attach",
            "use crate::mail",
            "use crate::secret",
        ] {
            assert!(
                !source.contains(forbidden),
                "Domain dependency points outward: {forbidden}"
            );
        }
    }
}

#[test]
fn services_use_ports_instead_of_concrete_adapters() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("src/application");
    for source in sources(&dir) {
        for forbidden in [
            "crate::infrastructure",
            "crate::adapters",
            "std::fs",
            "use tauri",
            "use eframe",
            "use crate::mail",
            "use crate::secret",
        ] {
            assert!(
                !source.contains(forbidden),
                "Service bypasses its port: {forbidden}"
            );
        }
    }
}
