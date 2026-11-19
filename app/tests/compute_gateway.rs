//! Indexing and search through the compute port (spec D3, Compito 1): a fake gateway (no model,
//! no server) reaches every inference the app makes, and its pauses keep the D2 semantics.
//!
//! One test function: `QAMILLE_DATA` is process-wide.
use qamille::application::compute::codes;
use qamille::application::indexing_ports::{IndexContent, IndexMedia};
use qamille::application::ports::SearchGateway;
use qamille::domain::model::{Account, Attachment, Email, Store};
use qamille::infrastructure::compute_factory;
use qamille::infrastructure::fake_compute::FakeCompute;
use qamille::infrastructure::gateways::LocalSearch;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

/// A message as the indexer leaves it once its passages are committed (version 2).
fn email(id: u64, subject: &str, body: &str) -> Email {
    let mut email = Email {
        id,
        account_id: 1,
        folder: "Inbox".into(),
        subject: subject.into(),
        from_name: "Maria".into(),
        from_addr: "maria@example.com".into(),
        body: body.into(),
        remote_content_hash: format!("hash-{id}"),
        date: 1_700_000_000 + id as i64,
        ..Default::default()
    };
    email.search_index.version = 2;
    email.search_index.content_hash = email.remote_content_hash.clone();
    email
}

fn content(email: &Email) -> IndexContent {
    IndexContent { email: email.clone(), media: vec![], warnings: vec![] }
}

#[test]
fn indexing_and_search_go_through_the_compute_gateway() {
    let root = std::env::temp_dir().join(format!("qamille-compute-gateway-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    unsafe { std::env::set_var("QAMILLE_DATA", &root) };
    let invoice = email(1, "Fattura", "La fattura di ottobre è già stata pagata con bonifico. Grazie.");
    let trip = email(2, "Viaggio", "Il treno per Cefalù parte alle nove dal binario tre.");
    let mut store = Store::default();
    store.accounts.push(Account { id: 1, email: "me@example.com".into(), ..Default::default() });
    store.emails = vec![invoice.clone(), trip.clone()];
    std::fs::write(Store::path(), serde_json::to_vec(&store).unwrap()).unwrap();

    let fake = Arc::new(FakeCompute::default());
    let search = LocalSearch::new(fake.clone());
    assert!(search.installed(), "the model state comes from the gateway's health");
    let status = search.install_status();
    assert!(status.installed && !status.running);
    let cancel = AtomicBool::new(false);
    for mail in [&invoice, &trip] {
        let report = search.index_document(&store, &content(mail), &cancel).unwrap();
        assert!(report.indexed);
    }
    let calls = fake.calls();
    assert!(calls.iter().any(|c| c == "chunks"), "{calls:?}");
    assert!(calls.iter().any(|c| c.starts_with("documents:")), "passages are embedded in batches: {calls:?}");

    let hits = search.search(&store, "bonifico pagata ottobre", 10).unwrap();
    assert_eq!(hits.first().map(|h| h.email_id), Some(1), "the closest passage wins");
    assert!(hits.first().is_some_and(|hit| hit.passage.contains("pagata")), "the passage is quoted from its verified source");
    assert!(fake.calls().iter().any(|c| c == "query"));

    // An image is embedded through the gateway; a bad one costs only that image.
    let pictures = email(3, "Foto", "In allegato le foto del sopralluogo.");
    store.emails.push(pictures.clone());
    std::fs::write(Store::path(), serde_json::to_vec(&store).unwrap()).unwrap();
    let mut with_media = content(&pictures);
    with_media.media = vec![
        IndexMedia { attachment_id: 3001, mime: "image/png".into(), bytes: vec![1, 2, 3] },
        IndexMedia { attachment_id: 3002, mime: "image/png".into(), bytes: vec![] },
    ];
    let report = search.index_document(&store, &with_media, &cancel).unwrap();
    assert!(report.indexed);
    assert_eq!(report.warnings.len(), 1, "{:?}", report.warnings);
    assert!(report.warnings[0].starts_with("Image part 3002"));

    // A pause of the server says nothing about the message: it is never an attempt (INV-3).
    let paused = email(4, "Pausa", "Questo messaggio aspetta il server di calcolo.");
    store.emails.push(paused.clone());
    std::fs::write(Store::path(), serde_json::to_vec(&store).unwrap()).unwrap();
    for code in [codes::MODEL_MISSING, codes::PAUSED_FOR_MEMORY, codes::PAUSED_AFTER_FAILURE, codes::UNREACHABLE, codes::RATE_LIMITED] {
        *fake.fail.lock().unwrap() = Some((code, "pausa".into()));
        let error = search.index_document(&store, &content(&paused), &cancel).err().expect("a pause");
        assert_eq!(error.code, "index_transient", "{code}");
    }
    *fake.fail.lock().unwrap() = Some((codes::INTERNAL, "Embedding failed: tensor".into()));
    assert_eq!(search.index_document(&store, &content(&paused), &cancel).err().expect("a failure").code, "external");
    // Without vectors for the query, search keeps the exact-word results.
    *fake.fail.lock().unwrap() = Some((codes::UNREACHABLE, "down".into()));
    let lexical = search.search(&store, "Cefalù", 10).unwrap();
    assert_eq!(lexical.first().map(|h| h.email_id), Some(2));
    *fake.fail.lock().unwrap() = None;

    // Attachment text: documents go to the gateway, plain text stays in the app; a pause
    // while reading a document leaves the whole message for later (INV-3).
    compute_factory::set_override(Some(fake.clone()));
    let before = fake.calls().len();
    let raw = b"From: maria@example.com\r\nSubject: Contratto\r\nMIME-Version: 1.0\r\nContent-Type: multipart/mixed; boundary=b\r\n\r\n--b\r\nContent-Type: text/plain\r\n\r\nVedi allegati.\r\n--b\r\nContent-Type: application/pdf\r\nContent-Disposition: attachment; filename=contratto.pdf\r\n\r\nclausola di riservatezza\r\n--b\r\nContent-Type: text/plain\r\nContent-Disposition: attachment; filename=note.txt\r\n\r\nnote a margine\r\n--b--\r\n";
    let original = email(5, "Contratto", "");
    let parsed = qamille::infrastructure::mail_index_source::parse_content(raw, &original).unwrap();
    let texts: Vec<_> = parsed.email.attachments.iter().map(|a: &Attachment| (a.name.clone(), a.inline_text.clone())).collect();
    assert!(texts.contains(&("contratto.pdf".into(), Some("clausola di riservatezza".into()))), "{texts:?}");
    assert!(texts.contains(&("note.txt".into(), Some("note a margine".into()))), "{texts:?}");
    assert_eq!(fake.calls()[before..].iter().filter(|c| *c == "extract").count(), 1, "only the document went to the server");
    *fake.fail.lock().unwrap() = Some((codes::UNREACHABLE, "down".into()));
    let error = qamille::infrastructure::mail_index_source::parse_content(raw, &original).err().expect("a pause");
    assert_eq!(error.code, "index_transient");
    *fake.fail.lock().unwrap() = Some((codes::DOCUMENT_UNREADABLE, "PDF: broken".into()));
    let skipped = qamille::infrastructure::mail_index_source::parse_content(raw, &original).unwrap();
    assert!(skipped.warnings.iter().any(|w| w == "contratto.pdf: PDF: broken"), "{:?}", skipped.warnings);
    assert_eq!(qamille::infrastructure::attach::bytes_to_text("scheda.docx", b"testo").unwrap_err(), "PDF: broken");
    *fake.fail.lock().unwrap() = None;
    assert_eq!(qamille::infrastructure::attach::bytes_to_text("scheda.docx", b"testo").unwrap(), "testo");
    compute_factory::set_override(None);

    // Without a server (the gateway says `compute_not_connected`) the words part is still made,
    // the vectors wait, and the search names the missing server.
    let offline = Arc::new(FakeCompute::default());
    *offline.fail.lock().unwrap() = Some((codes::NOT_CONNECTED, "Collega un server.".into()));
    let search = LocalSearch::new(offline.clone());
    assert!(search.needs_server());
    assert_eq!(search.semantic_readiness().unwrap(), false, "the words now, the vectors later");
    let words_only = email(6, "Parole", "Il preventivo del falegname arriva lunedì mattina.");
    store.emails.push(words_only.clone());
    std::fs::write(Store::path(), serde_json::to_vec(&store).unwrap()).unwrap();
    let report = search.index_lexical(&store, &content(&words_only), &cancel).unwrap();
    assert!(report.indexed, "the words part needs no server");
    let hits = search.search(&store, "falegname", 10).unwrap();
    assert_eq!(hits.first().map(|h| h.email_id), Some(6), "found by words without a server");
    let _ = std::fs::remove_dir_all(&root);
}
