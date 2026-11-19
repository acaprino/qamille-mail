fn main() {
    println!("{}", serde_json::to_string_pretty(&qamille::model::sample_store()).unwrap());
}
