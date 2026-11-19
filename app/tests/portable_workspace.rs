use qamille::model::Store;
use std::{path::PathBuf, process::Command};

#[test]
fn workspace_location_is_independent_of_user_profile_and_working_directory() {
    let executable = std::env::current_exe().unwrap();
    let expected = executable.parent().unwrap().join("data");
    let output = Command::new(&executable)
        .args(["--exact", "workspace_location_probe", "--nocapture"])
        .env_remove("QAMILLE_DATA")
        .env("QAMILLE_EXPECTED_WORKSPACE", &expected)
        .env("APPDATA", std::env::temp_dir().join("unused-qamille-profile"))
        .env("LOCALAPPDATA", std::env::temp_dir().join("unused-qamille-local-profile"))
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn explicit_workspace_override_is_shared_by_helpers_and_test_processes() {
    let executable = std::env::current_exe().unwrap();
    let expected = executable.parent().unwrap().join("portable-test-override");
    let output = Command::new(&executable)
        .args(["--exact", "workspace_location_probe", "--nocapture"])
        .env("QAMILLE_DATA", &expected)
        .env("QAMILLE_EXPECTED_WORKSPACE", &expected)
        .current_dir(std::env::temp_dir())
        .output()
        .unwrap();
    assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stdout));
}

#[test]
fn relative_workspace_override_is_absolute_from_the_process_working_directory() {
    let executable = std::env::current_exe().unwrap();
    let working_directory = executable.parent().unwrap().join(format!(
        "portable-relative-cwd-{}", std::process::id(),
    ));
    std::fs::create_dir_all(&working_directory).unwrap();
    let expected = working_directory.join("relative-workspace");
    let output = Command::new(&executable)
        .args(["--exact", "workspace_location_probe", "--nocapture"])
        .env("QAMILLE_DATA", "relative-workspace")
        .env("QAMILLE_EXPECTED_WORKSPACE", &expected)
        .current_dir(&working_directory)
        .output()
        .unwrap();
    std::fs::remove_dir_all(&working_directory).unwrap();
    assert!(output.status.success(), "{}\n{}",
        String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
}

#[test]
fn workspace_location_probe() {
    let Some(expected) = std::env::var_os("QAMILLE_EXPECTED_WORKSPACE") else { return };
    let expected = PathBuf::from(expected);
    assert_eq!(Store::data_dir(), expected);
    assert_eq!(Store::path(), expected.join("store.json"));
}
