//! Import recipes from URLs and print a readable report — the manual
//! companion to the live regression suite (`backend/tests/live_import.rs`).
//!
//! ```text
//! cargo run -q -p backend --bin import_check -- <url> [url…]
//! ```
//!
//! Nothing is persisted; this runs the same pipeline as
//! `POST /api/recipes/import` (with the private-target escape hatch off).

use backend::recipe_import;

#[tokio::main]
async fn main() {
    let urls: Vec<String> = std::env::args().skip(1).collect();
    if urls.is_empty() {
        eprintln!("usage: import_check <url> [url…]");
        std::process::exit(2);
    }

    let mut failures = 0usize;
    for url in &urls {
        let started = std::time::Instant::now();
        println!("═══ {url}");
        match recipe_import::import(url, false).await {
            Ok(preview) => {
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
