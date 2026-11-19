//! The fixed compute credential exists only for the tests of the compute routes: no serving
//! code may use it.

use std::path::{Path, PathBuf};

fn rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn the_serving_code_never_uses_fixed_auth() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files = Vec::new();
    rust_files(&root.join("src"), &mut files);
    let enterprise = root.join("../enterprise/server/src");
    if enterprise.is_dir() {
        rust_files(&enterprise, &mut files);
    }
    assert!(files.len() > 10, "the sources were found");
    for file in files.iter().filter(|f| !f.ends_with(Path::new("compute").join("mod.rs"))) {
        let text = std::fs::read_to_string(file).unwrap();
        assert!(
            !text.contains("ComputeAuth::Fixed") && !text.contains("fixed_router") && !text.contains("ComputeAuth::fixed"),
            "{file:?}"
        );
    }
}
