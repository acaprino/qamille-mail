#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Qamille: a light multi-account mail client that treats mail as tasks, with +AI helpers,
//! calendar, meeting recording and an MCP server for AI assistants.

use qamille::{mcp, model, rag, rules, ui};

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--mcp") {
        mcp::serve();
        return Ok(());
    }
    if args.iter().any(|a| a == "--worker") {
        mcp::server::worker();
        return Ok(());
    }
    if let Some(i) = args.iter().position(|a| a == "--rag-search") {
        rag_search(args.get(i + 1).map(String::as_str).unwrap_or(""));
        return Ok(());
    }
    if args.iter().any(|a| a == "--rag-index") {
        rag_index();
        return Ok(());
    }
    let screenshot = args
        .iter()
        .position(|a| a == "--screenshot")
        .and_then(|i| args.get(i + 1))
        .map(std::path::PathBuf::from);
    let _workspace_lock = qamille::infrastructure::meeting_repository::WorkspaceUiLock::acquire(&model::Store::data_dir())
        .map_err(|e| eframe::Error::AppCreation(Box::new(e)))?;
    let _legacy_writer=qamille::infrastructure::workspace_guard::legacy(&model::Store::data_dir()).map_err(|e|eframe::Error::AppCreation(Box::new(e)))?;
    let options = eframe::NativeOptions {
        persistence_path: Some(model::Store::data_dir().join("native-ui.ron")),
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1360.0, 820.0])
            .with_min_inner_size([900.0, 560.0])
            .with_title("Qamille"),
        ..Default::default()
    };
    eframe::run_native("Qamille", options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, screenshot)))))
}

/// Brings the semantic index up to date from a terminal.
fn rag_index() {
    let store = model::Store::load_or_default();
    let stop = std::sync::atomic::AtomicBool::new(false);
    let t = std::time::Instant::now();
    let compute = qamille::infrastructure::compute_factory::current();
    match rag::update_index(&store.emails, &stop, &|d, n| println!("{d}/{n}"), &*compute) {
        Ok((idx, r)) => println!(
            "{} passages from {} emails. Added {}, removed {}, unreadable attachments {}. {:.1}s",
            idx.passages.len(),
            idx.email_count(),
            r.added_emails,
            r.removed_emails,
            r.unreadable_attachments,
            t.elapsed().as_secs_f32()
        ),
        Err(e) => println!("{e}"),
    }
}

/// Runs one search from a terminal.
fn rag_search(query: &str) {
    let store = model::Store::load_or_default();
    let idx = rag::Index::load();
    let compute = qamille::infrastructure::compute_factory::current();
    let (hits, note) = rag::hybrid_search(&store, &idx, query, rules::SEARCH_LIMIT, &*compute);
    if let Some(n) = note {
        println!("note: {n}");
    }
    for h in hits {
        let subject = store.email(h.email_id).map(|e| e.subject.clone()).unwrap_or_default();
        println!("{:.3} | {} | {}", h.score, subject, h.passage.chars().take(90).collect::<String>().replace('\n', " "));
    }
}
