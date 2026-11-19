//! Public-fixture smoke test of the compute server (spec D3): extraction, retrieval and, with
//! an image path, vision. QAMILLE_DATA must point to a test directory, never an email account
//! store; the local server is found or started there (QAMILLE_SERVER_BIN overrides its path).
fn main() -> Result<(), String> {
    let client = qamille::infrastructure::compute_factory::current();
    let extracted = client.extract_document("example.txt", b"Temporary attachment contents").map_err(|e| e.message)?;
    assert_eq!(extracted, "Temporary attachment contents");
    let query = client.embed_query("cats sitting on a sofa").map_err(|e| e.message)?;
    let relevant = client.embed_document("Cats", "Two cats resting together on a sofa").map_err(|e| e.message)?;
    let unrelated = client.embed_document("Invoice", "Payment for consulting services due next month").map_err(|e| e.message)?;
    let similarity = |vector: &[f32]| query.iter().zip(vector).map(|(a, b)| a * b).sum::<f32>();
    assert_eq!(query.len(), 256);
    assert!(query.iter().all(|v| v.is_finite()));
    assert!(similarity(&relevant) > similarity(&unrelated));
    println!("Text retrieval OK.");
    if let Some(path) = std::env::args().nth(1) {
        let image = std::fs::read(path).map_err(|e| e.to_string())?;
        let visual = client.embed_image(&image).map_err(|e| e.message)?;
        assert_eq!(visual.len(), 256);
        assert!(visual.iter().all(|v| v.is_finite()));
        println!("Image/text similarity: {:.4}; irrelevant text: {:.4}", similarity(&visual), similarity(&unrelated));
        assert!(similarity(&visual) > similarity(&unrelated));
        println!("Vision retrieval OK.");
    }
    Ok(())
}
