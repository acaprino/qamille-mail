fn main() {
    if matches!(std::env::args().nth(1).as_deref(), Some("--skill" | "skill")) {
        print!("{}", qamille::infrastructure::assistant_packages::skill_text());
        return;
    }
    if std::env::args().any(|arg| arg == "--worker") {
        qamille::mcp::server::worker();
    } else {
        qamille::mcp::serve();
    }
}
