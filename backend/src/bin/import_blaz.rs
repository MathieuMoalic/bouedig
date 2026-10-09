//! One-shot migration: copy the active recipes of a blaz SQLite database
//! into bouedig through its own HTTP API (`POST /api/recipes/photo`), so
//! photos go through the app's storage and thumbnail pipeline and the
//! payload passes the same validation as a manual import.
//!
//! ```text
//! cargo run -q -p backend --bin import_blaz -- \
//!     --blaz-db=/path/blaz.sqlite --media-dir=/path/media \
//!     [--target=http://127.0.0.1:3987] [--password=…] \
//!     [--limit=5] [--dry-run]
//! ```
//!
//! Mapping: title→name, source/yield/notes as-is; ingredients keep their
//! structured quantity/unit/prep, and lines without a quantity (blaz's
//! legacy recipes carry the whole line in the name) go through the same
//! conservative parser as cart pushes; the JSON string list becomes
//! instruction steps. Blaz has no sections; prep reminders and macros have
//! no bouedig equivalent and are dropped. No skip-existing: running the
//! importer twice creates every recipe twice.

use anyhow::Context as _;
use serde::Deserialize;
use shared::{Ingredient, InstructionStep};

/// One ingredient line of a blaz recipe. blaz stores extra bookkeeping
/// fields (food_id, resolution_source, …) that serde ignores.
#[derive(Deserialize)]
struct BlazIngredient {
    quantity: Option<f64>,
    unit: Option<String>,
    name: String,
    prep: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let mut blaz_db = String::new();
    let mut media_dir = String::new();
    let mut target = "http://127.0.0.1:3987".to_string();
    let mut password = std::env::var("BOUEDIG_PASSWORD").ok().filter(|p| !p.is_empty());
    let mut limit: Option<usize> = None;
    let mut dry_run = false;
    for arg in std::env::args().skip(1) {
        match arg.as_str() {
            "--dry-run" => dry_run = true,
            _ if arg.starts_with("--blaz-db=") => {
                blaz_db = arg["--blaz-db=".len()..].to_string();
            }
            _ if arg.starts_with("--media-dir=") => {
                media_dir = arg["--media-dir=".len()..].to_string();
            }
            _ if arg.starts_with("--target=") => {
                target = arg["--target=".len()..].to_string();
            }
            _ if arg.starts_with("--password=") => {
                password = Some(arg["--password=".len()..].to_string());
            }
            _ if arg.starts_with("--limit=") => {
                limit = Some(arg["--limit=".len()..].parse().context("bad --limit")?);
            }
            other => {
                eprintln!("unknown argument: {other}");
                eprintln!(
                    "usage: import_blaz --blaz-db=<blaz.sqlite> --media-dir=<blaz media dir> \
                     [--target=http://host:port] [--password=…] [--limit=N] [--dry-run]"
                );
                std::process::exit(2);
            }
        }
    }
    if blaz_db.is_empty() || media_dir.is_empty() {
        eprintln!("usage: import_blaz --blaz-db=<blaz.sqlite> --media-dir=<blaz media dir> …");
        std::process::exit(2);
    }

    let blaz = sqlx::sqlite::SqliteConnectOptions::new()
        .filename(&blaz_db)
        .read_only(true);
    let db = sqlx::SqlitePool::connect_with(blaz).await?;

    let client = reqwest::Client::builder()
        .cookie_store(true)
        .timeout(std::time::Duration::from_secs(120))
        .build()?;
    if let Some(password) = &password {
        let status = client
            .post(format!("{target}/api/login"))
            .json(&serde_json::json!({ "password": password }))
            .send()
            .await?;
        anyhow::ensure!(
            status.status().is_success(),
            "login failed: {}",
            status.status()
        );
        println!("logged in to {target}");
    }

    let rows = sqlx::query_as::<_, BlazRecipeRow>(
        "SELECT id, title, source, \"yield\" AS yield_amount, notes, \
         ingredients, instructions, image_path_full, image_path_small \
         FROM recipes WHERE deleted_at IS NULL ORDER BY id",
    )
    .fetch_all(&db)
    .await?;
    let total = rows.len();
    let rows: Vec<_> = match limit {
        Some(n) => rows.into_iter().take(n).collect(),
        None => rows,
    };
    println!(
        "importing {} of {total} active blaz recipes into {target}{}",
        rows.len(),
        if dry_run { " (dry run)" } else { "" }
    );

    let mut ok = 0usize;
    let mut failed = 0usize;
    for (index, row) in rows.iter().enumerate() {
        let photo = row
            .image_path_full
            .as_deref()
            .or(row.image_path_small.as_deref())
            .map(|rel| {
                std::fs::read(std::path::Path::new(&media_dir).join(rel))
                    .context("reading the photo file")
            })
            .transpose();
        match import_one(&client, &target, row, photo, dry_run).await {
            Ok(ImportOutcome::Created(new_id)) => {
                ok += 1;
                println!("[{}/{total}] ok #{} {}", index + 1, new_id, row.title);
            }
            Ok(ImportOutcome::DryRun) => {
                ok += 1;
                println!("[{}/{total}] would import: {}", index + 1, row.title);
            }
            Ok(ImportOutcome::Skipped(reason)) => {
                println!("[{}/{total}] skipped ({reason}): {}", index + 1, row.title);
            }
            Err(error) => {
                failed += 1;
                println!("[{}/{total}] FAIL {}: {}", index + 1, row.title, error);
            }
        }
    }
    println!("done: {ok} imported, {failed} failed");
    if failed > 0 {
        std::process::exit(1);
    }
    Ok(())
}

#[derive(sqlx::FromRow)]
struct BlazRecipeRow {
    id: i64,
    title: String,
    source: String,
    yield_amount: String,
    notes: String,
    ingredients: String,
    instructions: String,
    image_path_full: Option<String>,
    image_path_small: Option<String>,
}

enum ImportOutcome {
    Created(i64),
    DryRun,
    Skipped(&'static str),
}

/// Import one recipe.
async fn import_one(
    client: &reqwest::Client,
    target: &str,
    row: &BlazRecipeRow,
    photo: anyhow::Result<Option<Vec<u8>>>,
    dry_run: bool,
) -> anyhow::Result<ImportOutcome> {
    let name = row.title.trim();
    if name.is_empty() {
        return Ok(ImportOutcome::Skipped("empty title"));
    }
    let blaz_ingredients: Vec<BlazIngredient> = serde_json::from_str(&row.ingredients)
        .with_context(|| format!("bad ingredients JSON in blaz recipe {}", row.id))?;
    let mut ingredients = Vec::with_capacity(blaz_ingredients.len());
    for line in blaz_ingredients {
        let name = line.name.trim();
        if name.is_empty() {
            continue;
        }
        match line.quantity {
            // Structured blaz line: quantity/unit/prep carry over directly.
            Some(q) if q.is_finite() && q > 0.0 => ingredients.push(Ingredient {
                quantity: Some(q),
                unit: line.unit.map(|u| u.trim().to_string()).filter(|u| !u.is_empty()),
                name: name.to_string(),
                prep: line.prep.map(|p| p.trim().to_string()).filter(|p| !p.is_empty()),
                section: None,
            }),
            // Legacy line — the whole text sits in the name. The
            // conservative parser splits a leading quantity/unit the same
            // way cart pushes do; unparsed lines keep the full text. A
            // zero/negative parse ("0g mushrooms") degrades to an
            // unquantified line — bouedig rejects non-positive quantities.
            _ => {
                let mut parsed =
                    backend::recipe_import::normalize::parse_ingredient_line(name);
                if parsed.quantity.is_some_and(|q| !q.is_finite() || q <= 0.0) {
                    parsed.quantity = None;
                    parsed.unit = None;
                }
                ingredients.push(parsed);
            }
        }
    }
    let blaz_instructions: Vec<String> = serde_json::from_str(&row.instructions)
        .with_context(|| format!("bad instructions JSON in blaz recipe {}", row.id))?;
    let instructions: Vec<InstructionStep> = blaz_instructions
        .into_iter()
        .map(|text| InstructionStep { text, section: None })
        .filter(|step| !step.text.trim().is_empty())
        .collect();

    if dry_run {
        // Everything above had to parse; the photo just has to exist.
        if let Some(bytes) = photo? {
            anyhow::ensure!(!bytes.is_empty(), "photo file is empty");
        }
        return Ok(ImportOutcome::DryRun);
    }

    let mut form = reqwest::multipart::Form::new()
        .text("name", name.to_string())
        .text("sections", "[]")
        .text("ingredients", serde_json::to_string(&ingredients)?)
        .text("instructions", serde_json::to_string(&instructions)?)
        .text("instruction_sections", "[]")
        .text("notes", row.notes.clone())
        .text("yield", row.yield_amount.clone())
        .text("source", row.source.clone());
    if let Some(bytes) = photo? {
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name("photo.webp")
            .mime_str("image/webp")?;
        form = form.part("image", part);
    }
    let response = client
        .post(format!("{target}/api/recipes/photo"))
        .multipart(form)
        .send()
        .await?;
    let status = response.status();
    anyhow::ensure!(
        status.is_success(),
        "POST /api/recipes/photo answered {} ({})",
        status,
        response.text().await.unwrap_or_default()
    );
    let created: serde_json::Value = response.json().await?;
    Ok(ImportOutcome::Created(created["id"].as_i64().unwrap_or_default()))
}
