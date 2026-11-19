use qamille::{
    application::ports::{AttachmentGateway, StoreRepository},
    infrastructure::{gateways::LocalAttachments, portable_files, repository::JsonStoreRepository},
    model::{Attachment, Draft, Email, Meeting, Store},
};
use std::path::{Path, PathBuf};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let root = std::env::temp_dir().join(format!(
            "qamille-portable-attachments-{}-{stamp}",
            std::process::id()
        ));
        std::fs::create_dir(&root).unwrap();
        Self(root)
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let root = self.0.canonicalize().unwrap();
        assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
        assert!(
            root.file_name()
                .unwrap()
                .to_string_lossy()
                .starts_with("qamille-portable-attachments-")
        );
        std::fs::remove_dir_all(root).unwrap();
    }
}

fn path_text(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

#[test]
fn selected_attachments_are_owned_copies_with_original_filenames() {
    let scratch = Scratch::new();
    let external = scratch.0.join("external");
    let workspace = scratch.0.join("workspace");
    for folder in [external.join("first"), external.join("second")] {
        std::fs::create_dir_all(folder).unwrap();
    }
    let a = external.join("first/report.txt");
    let b = external.join("second/report.txt");
    std::fs::write(&a, b"first attachment").unwrap();
    std::fs::write(&b, b"second attachment").unwrap();
    let owned =
        portable_files::import_draft_attachments(&workspace, &[a.clone(), b.clone()]).unwrap();
    assert_ne!(owned[0], owned[1]);
    for path in &owned {
        assert!(
            Path::new(path)
                .canonicalize()
                .unwrap()
                .starts_with(workspace.canonicalize().unwrap())
        );
        assert_eq!(Path::new(path).file_name().unwrap(), "report.txt");
    }
    std::fs::write(&a, b"edited original").unwrap();
    std::fs::remove_file(&b).unwrap();
    assert_eq!(std::fs::read(&owned[0]).unwrap(), b"first attachment");
    assert_eq!(std::fs::read(&owned[1]).unwrap(), b"second attachment");
}

#[test]
fn messages_staged_from_memory_keep_their_eml_names_and_reject_paths() {
    let scratch = Scratch::new();
    let workspace = scratch.0.join("workspace");
    let first = b"Subject: Avviso\r\n\r\nbody".to_vec();
    let staged = portable_files::stage_draft_attachment_bytes(
        &workspace,
        vec![
            ("email-7-Avviso.eml".into(), first.clone()),
            ("email-8-Altro.eml".into(), b"Subject: Altro\r\n\r\nbody".to_vec()),
        ],
    )
    .unwrap();
    assert_eq!(staged.paths.len(), 2);
    for (path, name) in staged.paths.iter().zip(["email-7-Avviso.eml", "email-8-Altro.eml"]) {
        assert_eq!(Path::new(path).file_name().unwrap(), name);
        assert!(
            Path::new(path)
                .canonicalize()
                .unwrap()
                .starts_with(workspace.canonicalize().unwrap())
        );
    }
    assert_eq!(std::fs::read(&staged.paths[0]).unwrap(), first);
    for bad in ["../escape.eml", "nested/name.eml", "nested\\name.eml", "..", ""] {
        assert!(
            portable_files::stage_draft_attachment_bytes(&workspace, vec![(bad.into(), b"x".to_vec())])
                .is_err(),
            "{bad:?} must be refused"
        );
    }
    portable_files::release_draft_attachment_imports(&workspace, &[staged.lease_id.unwrap()])
        .unwrap();
}

#[test]
fn moved_workspace_keeps_mail_draft_outbox_and_legacy_audio_paths_readable() {
    let scratch = Scratch::new();
    let first = scratch.0.join("first/data");
    std::fs::create_dir_all(first.join("attachments")).unwrap();
    std::fs::create_dir_all(first.join("recordings")).unwrap();
    let attachment = first.join("attachments/report.txt");
    let recording = first.join("recordings/meeting.wav");
    std::fs::write(&attachment, b"portable attachment").unwrap();
    std::fs::write(&recording, b"portable recording").unwrap();
    let mut store = Store::default();
    store.cache_removals.push(path_text(&attachment));
    store.emails.push(Email {
        id: 1,
        attachments: vec![Attachment {
            name: "report.txt".into(),
            path: Some(path_text(&attachment.canonicalize().unwrap())),
            ..Default::default()
        }],
        ..Default::default()
    });
    store.drafts.push(Draft {
        id: 2,
        attachments: vec![path_text(&attachment)],
        ..Default::default()
    });
    store.meetings.push(Meeting {
        audio_path: Some(path_text(&recording)),
        ..Default::default()
    });
    store.outbox.push(qamille::domain::outbox::Delivery {
        id: 3,
        key: "synthetic-key".into(),
        request_hash: "synthetic-hash".into(),
        draft: Draft {
            id: 2,
            attachments: vec![path_text(&attachment)],
            ..Default::default()
        },
        plain: String::new(),
        html: None,
        sender_name: String::new(),
        attachment_bytes: 19,
        status: qamille::domain::outbox::DeliveryStatus::Failed,
        created_at: 0,
        updated_at: 0,
        send_at: 0,
        requested_send_at: None,
        error: None,
        owner_pid: None,
        requires_mcp_send: false,
        source_profile: None,
    });
    std::fs::write(
        first.join("store.json"),
        serde_json::to_vec(&store).unwrap(),
    )
    .unwrap();
    let repository = JsonStoreRepository::new(first.join("store.json"));
    repository.transaction(&mut |_| Ok(())).unwrap();
    let moved = scratch.0.join("moved/data");
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&first, &moved).unwrap();
    let reopened = JsonStoreRepository::new(moved.join("store.json"))
        .read()
        .unwrap();
    assert_eq!(
        LocalAttachments
            .read_bytes(&reopened.emails[0].attachments[0])
            .unwrap(),
        b"portable attachment"
    );
    assert_eq!(
        std::fs::read(&reopened.drafts[0].attachments[0]).unwrap(),
        b"portable attachment"
    );
    assert_eq!(
        std::fs::read(&reopened.outbox[0].draft.attachments[0]).unwrap(),
        b"portable attachment"
    );
    assert_eq!(
        std::fs::read(reopened.meetings[0].audio_path.as_ref().unwrap()).unwrap(),
        b"portable recording"
    );
    assert_eq!(
        std::fs::read(&reopened.cache_removals[0]).unwrap(),
        b"portable attachment"
    );
}

#[test]
fn external_legacy_paths_are_preserved_without_importing_or_deleting_them() {
    let scratch = Scratch::new();
    let external = scratch.0.join("external.txt");
    std::fs::write(&external, b"external original").unwrap();
    let workspace = scratch.0.join("data");
    std::fs::create_dir(&workspace).unwrap();
    let mut store = Store::default();
    store.drafts.push(Draft {
        attachments: vec![path_text(&external)],
        ..Default::default()
    });
    let repository = JsonStoreRepository::new(workspace.join("store.json"));
    repository
        .transaction(&mut |current| {
            *current = store.clone();
            Ok(())
        })
        .unwrap();
    assert_eq!(
        repository.read().unwrap().drafts[0].attachments[0],
        path_text(&external)
    );
    assert_eq!(std::fs::read(external).unwrap(), b"external original");
}

#[test]
fn workspace_references_cannot_escape_the_data_directory() {
    let scratch = Scratch::new();
    let mut store = Store::default();
    store.drafts.push(Draft {
        attachments: vec!["workspace:/../outside.txt".into()],
        ..Default::default()
    });
    let bytes = serde_json::to_vec(&store).unwrap();
    let path = scratch.0.join("store.json");
    std::fs::write(&path, &bytes).unwrap();
    assert!(JsonStoreRepository::new(&path).read().is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn managed_mcp_file_handles_survive_moving_the_workspace() {
    struct Environment(Option<std::ffi::OsString>);
    impl Drop for Environment {
        fn drop(&mut self) {
            unsafe {
                match &self.0 {
                    Some(value) => std::env::set_var("QAMILLE_DATA", value),
                    None => std::env::remove_var("QAMILLE_DATA"),
                }
            }
        }
    }
    let scratch = Scratch::new();
    let first = scratch.0.join("first/data");
    let moved = scratch.0.join("moved/data");
    let _environment = Environment(std::env::var_os("QAMILLE_DATA"));
    // Only this test in the binary uses the process workspace; the others use explicit paths.
    unsafe { std::env::set_var("QAMILLE_DATA", &first) };
    let created =
        qamille::mcp::files::write("report.txt", "text/plain", b"portable MCP file").unwrap();
    std::fs::create_dir_all(moved.parent().unwrap()).unwrap();
    std::fs::rename(&first, &moved).unwrap();
    unsafe { std::env::set_var("QAMILLE_DATA", &moved) };
    let resolved = qamille::mcp::files::resolve(created["handle"].as_str().unwrap()).unwrap();
    assert!(resolved.starts_with(moved.canonicalize().unwrap()));
    assert_eq!(std::fs::read(resolved).unwrap(), b"portable MCP file");
}
