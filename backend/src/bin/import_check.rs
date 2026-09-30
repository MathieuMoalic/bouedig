//! Import recipes from URLs and print a readable report — the manual
//! companion to the live regression suite (`backend/tests/live_import.rs`).
//!
//! ```text
//! cargo run -q -p backend --bin import_check -- <url> [url…]
//! ```
//!
//! Nothing is persisted; this runs the same pipeline as
//! `POST /api/recipes/import` (with the private-target escape hatch off).

use anyhow::Context as _;
use backend::recipe_import;

#[tokio::main]
async fn main() {
    // usage: import_check [--save [--base URL]] <url> [url…]
    let mut save = false;
    let mut base = "http://127.0.0.1:3000".to_string();
    let mut urls: Vec<String> = Vec::new();
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--save" => save = true,
            _ if arg.starts_with("--base=") => base = arg["--base=".len()..].to_string(),
            _ => urls.push(arg),
        }
    }
    if urls.is_empty() {
        eprintln!("usage: import_check [--save] [--base=http://host:port] <url> [url…]");
        std::process::exit(2);
    }

    let mut failures = 0usize;
    for url in &urls {
        let started = std::time::Instant::now();
        println!("═══ {url}");
        match recipe_import::import(url, false).await {
            Ok(preview) => {
                if save {
                    match save_preview(&base, &preview).await {
                        Ok(saved) => println!("  saved: #{} {}", saved.0, saved.1),
                        Err(error) => {
                            failures += 1;
                            println!("  SAVE FAILED: {error:#}");
                        }
                    }
                }
                println!(
                    "  method={}  confidence={:.0}%  elapsed={}ms",
                    serde_json::to_string(&preview.method).unwrap_or_default(),
                    preview.confidence * 100.0,
                    started.elapsed().as_millis()
                );
                println!("  name:   {}", preview.recipe.name);
                println!("  yield:  {}", preview.recipe.yield_amount);
                println!(
                    "  counts: {} ingredients, {} instructions",
                    preview.recipe.ingredients.len(),
                    preview.recipe.instructions.len()
                );
                println!("  ingredients:");
                for ingredient in &preview.recipe.ingredients {
                    let qty = ingredient
                        .quantity
                        .map(|q| format!("{q}"))
                        .unwrap_or_else(|| "–".into());
                    println!(
                        "    {qty:>6} | {:>6} | {} | {}",
                        ingredient.unit.as_deref().unwrap_or("–"),
                        ingredient.name,
                        ingredient.prep.as_deref().unwrap_or("–"),
                    );
                }
                println!("  instructions:");
                for (i, step) in preview.recipe.instructions.iter().enumerate() {
                    match &step.section {
                        Some(section) => println!("    {}. [{section}] {}", i + 1, step.text),
                        None => println!("    {}. {}", i + 1, step.text),
                    }
                }
                if !preview.warnings.is_empty() {
                    println!("  warnings:");
                    for warning in &preview.warnings {
                        println!("    - {warning}");
                    }
                }
            }
            Err(error) => {
                failures += 1;
                println!("  ERROR: {error}");
            }
        }
    }
    if failures > 0 {
        std::process::exit(1);
    }
}

/// Save an imported preview through the multipart create endpoint, carrying
/// the image URL so the backend downloads the photo at save time — the same
/// flow the web client uses.
async fn save_preview(
    base: &str,
    preview: &recipe_import::RecipePreview,
) -> anyhow::Result<(i64, String)> {
    let recipe = &preview.recipe;
    let boundary = format!("importcheck{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis());
    let field = |name: &str, value: &str| {
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n")
    };
    let mut body = String::new();
    body.push_str(&field("name", &recipe.name));
    body.push_str(&field(
        "sections",
        &serde_json::to_string(&recipe.sections)?,
    ));
    body.push_str(&field(
        "ingredients",
        &serde_json::to_string(&recipe.ingredients)?,
    ));
    body.push_str(&field(
        "instructions",
        &serde_json::to_string(&recipe.instructions)?,
    ));
    body.push_str(&field(
        "instruction_sections",
        &serde_json::to_string(&recipe.instruction_sections)?,
    ));
    body.push_str(&field("notes", &recipe.notes));
    body.push_str(&field("yield", &recipe.yield_amount));
    body.push_str(&field("source", &recipe.source));
    if let Some(image_url) = &preview.image_url {
        body.push_str(&field("image_url", image_url));
    }
    body.push_str(&format!("--{boundary}--\r\n"));

    let client = reqwest::Client::new();
    let response = client
        .post(format!("{base}/api/recipes/photo"))
        .header("Content-Type", format!("multipart/form-data; boundary={boundary}"))
        .body(body)
        .send()
        .await?;
    let status = response.status();
    let payload: serde_json::Value = response.json().await?;
    if !status.is_success() {
        anyhow::bail!("save returned {status}: {payload}");
    }
    Ok((
        payload["id"].as_i64().context("saved recipe has no id")?,
        payload["name"]
            .as_str()
            .context("saved recipe has no name")?
            .to_string(),
    ))
}
