//! Bouedig backend: Axum HTTP server over SQLite (sqlx).
//!
//! Runs standalone (default port 3000) and is also designed to sit behind a
//! reverse proxy in production:
//!   * `BOUEDIG_BASE_PATH=/bouedig` serves the app under a sub-path.
//!   * `BOUEDIG_STATIC_DIR=./dist` serves a built web client (SPA fallback to
//!     index.html), so the proxy only has to forward to a single origin.

use std::net::SocketAddr;
use std::path::PathBuf;

use anyhow::Context as _;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, delete, post};
use axum::{Json, Router};
use serde::Deserialize;
use shared::{
    GroceryItem, GroceryPatch, GroceryUpdate, Ingredient, InstructionStep, MealPlanEntry,
    NewGroceryBatch, NewGroceryItem, Recipe, RecipeDetail, RecipeInput,
};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

/// The web-recipe importer: fetch + JSON-LD/HTML extraction + normalization.
/// Used by `POST /api/recipes/import`, the `import_check` bin and the live
/// regression suite (`backend/tests/live_import.rs`).
pub mod recipe_import;

/// Grocery-category classification via Jev on OpenRouter (cache-first,
/// background — see the module docs).
pub mod classifier;

use recipe_import::{ImportError, RecipePreview};

/// Runtime configuration, all overridable via environment variables.
#[derive(Debug, Clone)]
pub struct Config {
    /// Address to bind, e.g. `127.0.0.1:3000`.
    pub addr: SocketAddr,
    /// SQLite URL, e.g. `sqlite://bouedig.db?mode=rwc`.
    pub db_url: String,
    /// Optional base path when mounted behind a reverse proxy.
    pub base_path: Option<String>,
    /// Optional directory containing the built web client to serve.
    pub static_dir: Option<PathBuf>,
    /// Directory for uploaded recipe photos (`images/` + `images/thumbs/`).
    pub data_dir: PathBuf,
    /// Test/dev escape hatch: allow the importer to fetch loopback/private
    /// targets (`BOUEDIG_IMPORT_ALLOW_PRIVATE=1`). Never request-controlled.
    pub import_allow_private: bool,
    /// OpenRouter key enabling Jev grocery categorization
    /// (`BOUEDIG_OPENROUTER_KEY`). Absent → items stay in "Other".
    pub openrouter_key: Option<String>,
    /// OpenRouter model slug for the classifier (`BOUEDIG_CLASSIFIER_MODEL`).
    pub classifier_model: Option<String>,
    /// Classifier endpoint override (`BOUEDIG_CLASSIFIER_ENDPOINT`), used by
    /// tests to point at a local mock.
    pub classifier_endpoint: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let port: u16 = std::env::var("BOUEDIG_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(3000);
        let addr = std::env::var("BOUEDIG_ADDR")
            .ok()
            .and_then(|a| a.parse().ok())
            .unwrap_or(SocketAddr::from(([127, 0, 0, 1], port)));
        Self {
            addr,
            db_url: std::env::var("BOUEDIG_DB_URL")
                .unwrap_or_else(|_| "sqlite://bouedig.db?mode=rwc".into()),
            base_path: std::env::var("BOUEDIG_BASE_PATH").ok().filter(|p| !p.is_empty()),
            static_dir: std::env::var("BOUEDIG_STATIC_DIR")
                .ok()
                .filter(|p| !p.is_empty())
                .map(PathBuf::from),
            data_dir: std::env::var("BOUEDIG_DATA_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("data")),
            import_allow_private: std::env::var("BOUEDIG_IMPORT_ALLOW_PRIVATE")
                .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                .unwrap_or(false),
            openrouter_key: std::env::var("BOUEDIG_OPENROUTER_KEY").ok(),
            classifier_model: std::env::var("BOUEDIG_CLASSIFIER_MODEL").ok(),
            classifier_endpoint: std::env::var("BOUEDIG_CLASSIFIER_ENDPOINT").ok(),
        }
    }
}

/// Internal server state threaded through handlers.
#[derive(Clone)]
pub struct AppState {
    db: SqlitePool,
    data_dir: PathBuf,
    import_allow_private: bool,
    /// Present only when an OpenRouter key is configured.
    classifier: Option<classifier::Classifier>,
}

impl AppState {
    fn from_config(db: SqlitePool, config: &Config) -> Self {
        Self {
            db,
            data_dir: config.data_dir.clone(),
            import_allow_private: config.import_allow_private,
            classifier: classifier::Classifier::from_parts(
                config.openrouter_key.clone(),
                config.classifier_model.clone(),
                config.classifier_endpoint.clone(),
            ),
        }
    }
}

/// Open (creating if needed) the SQLite pool for `db_url`.
pub async fn open_db(db_url: &str) -> anyhow::Result<SqlitePool> {
    let options = db_url
        .parse::<SqliteConnectOptions>()
        .context("invalid BOUEDIG_DB_URL")?
        .busy_timeout(std::time::Duration::from_secs(5))
        // Needed for `ON DELETE CASCADE` on recipe_ingredients.
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await
        .context("failed to open SQLite database")?;
    Ok(pool)
}

/// Apply the embedded SQL migrations. Idempotent, safe to run on every boot.
pub async fn run_migrations(pool: &SqlitePool) -> anyhow::Result<()> {
    sqlx::migrate!("./migrations")
        .run(pool)
        .await
        .context("failed to run database migrations")?;
    Ok(())
}

/// Build the complete application router (API + optional static files).
pub fn build_router(state: AppState, config: &Config) -> Router {
    let api = Router::new()
        .route("/recipes", get(list_recipes).post(create_recipe))
        .route("/recipes/photo", post(create_recipe_with_photo))
        .route(
            "/recipes/import",
            post(import_recipe_from_url),
        )
        .route(
            "/recipes/{id}",
            get(recipe_detail).put(update_recipe).delete(delete_recipe),
        )
        .route(
            "/grocery",
            get(list_grocery).post(add_grocery_item),
        )
        .route("/grocery/batch", post(add_grocery_batch))
        .route(
            "/grocery/{id}",
            delete(delete_grocery_item)
                .patch(update_grocery_item)
                .put(patch_grocery_item),
        )
        .route(
            "/meal-plan",
            get(list_meal_plan).post(add_meal_plan_entry),
        )
        .route("/meal-plan/{id}", delete(delete_meal_plan_entry))
        .route("/images/{*path}", get(serve_image))
        // Photo uploads can be several megabytes.
        .layer(axum::extract::DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(CorsLayer::very_permissive())
        .with_state(state);

    let mut app = Router::new().nest("/api", api);
    if let Some(static_dir) = &config.static_dir {
        let index = static_dir.join("index.html");
        let serve = ServeDir::new(static_dir)
            .append_index_html_on_directories(true)
            .fallback(ServeFile::new(index.clone()));
        app = app
            // Explicit index route: axum does not reliably hit the fallback
            // service for the empty remainder of a nested base path.
            .route("/", axum::routing::get_service(ServeFile::new(index)))
            .fallback_service(serve);
    }

    // When mounted behind a reverse proxy that does not strip the prefix
    // (e.g. /bouedig/api/... hits us directly), nest everything under it.
    if let Some(base_path) = config.base_path.as_deref().filter(|p| *p != "/") {
        // axum never dispatches the empty remainder of a nested prefix
        // (`/{base_path}/`) to the inner router, so redirect it to the bare
        // prefix, which is routed (and serves the SPA index).
        Router::new()
            .nest(base_path, app)
            .layer(axum::middleware::from_fn_with_state(
                base_path.to_string(),
                redirect_trailing_slash,
            ))
    } else {
        app
    }
}

/// Redirect `/{prefix}/…/` to `/{prefix}/…` (except the site root `/`).
async fn redirect_trailing_slash(
    axum::extract::State(_base): axum::extract::State<String>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let path = req.uri().path();
    if path.len() > 1 && path.ends_with('/') {
        let target = path.trim_end_matches('/');
        let location = match req.uri().query() {
            Some(q) if !q.is_empty() => format!("{target}?{q}"),
            _ => target.to_string(),
        };
        return axum::response::Redirect::permanent(&location).into_response();
    }
    next.run(req).await
}

/// Ensure the upload directories exist.
pub fn ensure_data_dirs(data_dir: &std::path::Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(data_dir.join("images").join("thumbs"))
        .with_context(|| format!("failed to create image dirs under {}", data_dir.display()))
}

/// Bind, migrate and serve. Resolves when the server shuts down.
pub async fn run(config: Config) -> anyhow::Result<()> {
    let pool = open_db(&config.db_url).await?;
    run_migrations(&pool).await?;
    ensure_data_dirs(&config.data_dir)?;
    let app = build_router(AppState::from_config(pool, &config), &config);
    tracing::info!("listening on http://{}", config.addr);
    let listener = tokio::net::TcpListener::bind(config.addr)
        .await
        .context("failed to bind address")?;
    axum::serve(listener, app).await.context("server error")
}

/// Bind, migrate and serve **in the background**, returning the bound
/// address. Used by the E2E suite, which passes port 0 to get a free port.
pub async fn spawn_server(config: Config) -> anyhow::Result<SocketAddr> {
    let pool = open_db(&config.db_url).await?;
    run_migrations(&pool).await?;
    ensure_data_dirs(&config.data_dir)?;
    let app = build_router(AppState::from_config(pool, &config), &config);
    let listener = tokio::net::TcpListener::bind(config.addr)
        .await
        .context("failed to bind address")?;
    let addr = listener.local_addr()?;
    tracing::info!("listening on http://{addr}");
    tokio::spawn(async move {
        if let Err(err) = axum::serve(listener, app).await {
            tracing::error!("server error: {err:#}");
        }
    });
    Ok(addr)
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Uniform error response for handlers: either a pre-baked status + message
/// or an internal error that is logged and reported as 500.
struct ApiError(axum::response::Response);

impl ApiError {
    fn internal(err: impl Into<anyhow::Error>) -> Self {
        let err = err.into();
        tracing::error!("api error: {:#}", err);
        Self(
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(serde_json::json!({ "error": err.to_string() })),
            )
                .into_response(),
        )
    }

    /// Client error with a dynamic message (e.g. multipart parse failures).
    fn client(status: StatusCode, msg: String) -> Self {
        Self(
            (
                status,
                Json(serde_json::json!({ "error": msg })),
            )
                .into_response(),
        )
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        self.0
    }
}

impl From<(StatusCode, &'static str)> for ApiError {
    fn from((status, msg): (StatusCode, &'static str)) -> Self {
        Self((status, Json(serde_json::json!({ "error": msg }))).into_response())
    }
}

impl From<anyhow::Error> for ApiError {
    fn from(err: anyhow::Error) -> Self {
        Self::internal(err)
    }
}

impl From<sqlx::Error> for ApiError {
    fn from(err: sqlx::Error) -> Self {
        Self::internal(err)
    }
}

fn row_to_item(row: &sqlx::sqlite::SqliteRow) -> GroceryItem {
    use sqlx::Row;
    GroceryItem {
        id: row.get::<i64, _>("id"),
        name: row.get::<String, _>("name"),
        bought: row.get::<i64, _>("bought") != 0,
        category: row.get::<String, _>("category"),
        recipe: None,
    }
}

/// Grocery row joined with its provenance recipe (`g.*` + `r.*` aliased
/// columns); `recipe` is `None` when the item has no linked recipe.
fn row_to_joined_item(row: &sqlx::sqlite::SqliteRow) -> GroceryItem {
    use sqlx::Row;
    let recipe = row.get::<Option<i64>, _>("recipe_id").map(|id| {
        let url = |col: Option<String>| col.map(|p| format!("/api/images/{p}"));
        Recipe {
            id,
            name: row.get::<String, _>("recipe_name"),
            image: url(row.get::<Option<String>, _>("image_path")),
            thumb: url(row.get::<Option<String>, _>("thumb_path")),
        }
    });
    GroceryItem {
        id: row.get::<i64, _>("id"),
        name: row.get::<String, _>("name"),
        bought: row.get::<i64, _>("bought") != 0,
        category: row.get::<String, _>("category"),
        recipe,
    }
}

/// The grocery SELECT with provenance joined in; `r.id` aliases as
/// `recipe_id` so it never collides with `g.id`.
const GROCERY_SELECT: &str = "SELECT g.id, g.name, g.bought, g.category, \
     r.id AS recipe_id, r.name AS recipe_name, r.image_path, r.thumb_path \
     FROM grocery_items g LEFT JOIN recipes r ON r.id = g.recipe_id";

/// DB stores relative paths under the image dir; expose them as URLs.
fn recipe_urls(row: &sqlx::sqlite::SqliteRow) -> (Option<String>, Option<String>) {
    use sqlx::Row;
    let url = |col: Option<String>| col.map(|p| format!("/api/images/{p}"));
    (
        url(row.get::<Option<String>, _>("image_path")),
        url(row.get::<Option<String>, _>("thumb_path")),
    )
}

fn row_to_recipe(row: &sqlx::sqlite::SqliteRow) -> Recipe {
    use sqlx::Row;
    let (image, thumb) = recipe_urls(row);
    Recipe {
        id: row.get::<i64, _>("id"),
        name: row.get::<String, _>("name"),
        image,
        thumb,
    }
}

/// Assemble the full detail view (recipe + sections + ingredients + steps).
async fn load_recipe_detail(db: &SqlitePool, id: i64) -> Result<Option<RecipeDetail>, ApiError> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT id, name, notes, yield_amount, source, image_path, thumb_path \
         FROM recipes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let (image, thumb) = recipe_urls(&row);
    let sections = load_section_names(db, id, "ingredient").await?;
    let instruction_sections = load_section_names(db, id, "instruction").await?;
    let steps = sqlx::query(
        "SELECT section, text FROM recipe_instructions \
         WHERE recipe_id = ? ORDER BY position ASC",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    let instructions = steps
        .iter()
        .map(|row| InstructionStep {
            text: row.get::<String, _>("text"),
            section: row.get::<Option<String>, _>("section"),
        })
        .collect();
    let rows = sqlx::query(
        "SELECT quantity, unit, name, prep, section FROM recipe_ingredients \
         WHERE recipe_id = ? ORDER BY position ASC",
    )
    .bind(id)
    .fetch_all(db)
    .await?;
    let ingredients = rows
        .iter()
        .map(|row| {
            Ingredient {
                quantity: row.get::<Option<f64>, _>("quantity"),
                unit: row.get::<Option<String>, _>("unit"),
                name: row.get::<String, _>("name"),
                prep: row.get::<Option<String>, _>("prep"),
                section: row.get::<Option<String>, _>("section"),
            }
        })
        .collect();
    Ok(Some(RecipeDetail {
        id,
        name: row.get("name"),
        sections,
        ingredients,
        instructions,
        instruction_sections,
        notes: row.get("notes"),
        yield_amount: row.get("yield_amount"),
        source: row.get("source"),
        image,
        thumb,
    }))
}

/// Load one kind of ordered section names for a recipe.
async fn load_section_names(
    db: &SqlitePool,
    recipe_id: i64,
    kind: &str,
) -> Result<Vec<String>, ApiError> {
    use sqlx::Row;
    Ok(sqlx::query(
        "SELECT name FROM recipe_sections \
         WHERE recipe_id = ? AND kind = ? ORDER BY position ASC",
    )
    .bind(recipe_id)
    .bind(kind)
    .fetch_all(db)
    .await?
    .iter()
    .map(|r| r.get::<String, _>("name"))
    .collect())
}

/// Remove the stored photo files of a recipe (best effort).
fn delete_image_files(data_dir: &std::path::Path, image_path: Option<&str>, thumb_path: Option<&str>) {
    for path in [image_path, thumb_path].into_iter().flatten() {
        match std::fs::remove_file(data_dir.join(path)) {
            Ok(()) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
            Err(err) => tracing::warn!("failed to remove image {path}: {err}"),
        }
    }
}

/// Insert a recipe with structured ingredients. The grocery list is **not**
/// touched here — adding ingredients to it is an explicit user action.
async fn insert_recipe(
    db: &SqlitePool,
    input: &RecipeInput,
    image: Option<(String, String)>,
) -> Result<Recipe, ApiError> {
    let (image_path, thumb_path) = match image {
        Some((i, t)) => (Some(i), Some(t)),
        None => (None, None),
    };
    // Every section referenced by an ingredient/step must exist.
    let sections = complete_sections(&input.sections, input.ingredients.iter().map(|i| i.section.clone()));
    let instruction_sections = complete_sections(
        &input.instruction_sections,
        input.instructions.iter().map(|s| s.section.clone()),
    );
    let mut tx = db.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO recipes (name, notes, yield_amount, source, image_path, thumb_path) \
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(input.name.trim())
    .bind(input.notes.trim())
    .bind(input.yield_amount.trim())
    .bind(input.source.trim())
    .bind(image_path.clone())
    .bind(thumb_path.clone())
    .fetch_one(&mut *tx)
    .await?;

    insert_sections(&mut tx, id, "ingredient", &sections).await?;
    insert_sections(&mut tx, id, "instruction", &instruction_sections).await?;

    for (position, ingredient) in input.ingredients.iter().enumerate() {
        sqlx::query(
            "INSERT INTO recipe_ingredients \
             (recipe_id, position, quantity, unit, name, prep, section) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(id)
        .bind(position as i64)
        .bind(ingredient.quantity)
        .bind(ingredient.unit.as_deref().map(str::trim).filter(|u| !u.is_empty()))
        .bind(ingredient.name.trim())
        .bind(ingredient.prep.as_deref().map(str::trim).filter(|p| !p.is_empty()))
        .bind(ingredient.section.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .execute(&mut *tx)
        .await?;
    }

    for (position, step) in input.instructions.iter().enumerate() {
        sqlx::query(
            "INSERT INTO recipe_instructions (recipe_id, position, section, text) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(id)
        .bind(position as i64)
        .bind(step.section.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(step.text.trim())
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;

    Ok(Recipe {
        id,
        name: input.name.trim().to_string(),
        image: image_path.map(|p| format!("/api/images/{p}")),
        thumb: thumb_path.map(|p| format!("/api/images/{p}")),
    })
}

/// The provided section list, plus any extra sections referenced by the
/// items (appended in first-seen order).
fn complete_sections<I>(provided: &[String], referenced: I) -> Vec<String>
where
    I: IntoIterator<Item = Option<String>>,
{
    let mut sections: Vec<String> = provided
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    for section in referenced.into_iter().flatten() {
        let section = section.trim().to_string();
        if !section.is_empty() && !sections.iter().any(|s| *s == section) {
            sections.push(section);
        }
    }
    sections
}

/// Insert ordered section names of one kind.
async fn insert_sections(
    tx: &mut sqlx::SqliteConnection,
    recipe_id: i64,
    kind: &str,
    sections: &[String],
) -> Result<(), ApiError> {
    for (position, section) in sections.iter().enumerate() {
        sqlx::query("INSERT INTO recipe_sections (recipe_id, kind, position, name) VALUES (?, ?, ?, ?)")
            .bind(recipe_id)
            .bind(kind)
            .bind(position as i64)
            .bind(section)
            .execute(&mut *tx)
            .await?;
    }
    Ok(())
}

/// Replace the sections, ingredients and instruction steps of a recipe.
async fn replace_details(
    tx: &mut sqlx::SqliteConnection,
    recipe_id: i64,
    input: &RecipeInput,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM recipe_sections WHERE recipe_id = ?")
        .bind(recipe_id)
        .execute(&mut *tx)
        .await?;
    let sections = complete_sections(&input.sections, input.ingredients.iter().map(|i| i.section.clone()));
    let instruction_sections = complete_sections(
        &input.instruction_sections,
        input.instructions.iter().map(|s| s.section.clone()),
    );
    insert_sections(tx, recipe_id, "ingredient", &sections).await?;
    insert_sections(tx, recipe_id, "instruction", &instruction_sections).await?;

    sqlx::query("DELETE FROM recipe_ingredients WHERE recipe_id = ?")
        .bind(recipe_id)
        .execute(&mut *tx)
        .await?;
    for (position, ingredient) in input.ingredients.iter().enumerate() {
        sqlx::query(
            "INSERT INTO recipe_ingredients \
             (recipe_id, position, quantity, unit, name, prep, section) VALUES (?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(recipe_id)
        .bind(position as i64)
        .bind(ingredient.quantity)
        .bind(ingredient.unit.as_deref().map(str::trim).filter(|u| !u.is_empty()))
        .bind(ingredient.name.trim())
        .bind(ingredient.prep.as_deref().map(str::trim).filter(|p| !p.is_empty()))
        .bind(ingredient.section.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .execute(&mut *tx)
        .await?;
    }

    sqlx::query("DELETE FROM recipe_instructions WHERE recipe_id = ?")
        .bind(recipe_id)
        .execute(&mut *tx)
        .await?;
    for (position, step) in input.instructions.iter().enumerate() {
        sqlx::query(
            "INSERT INTO recipe_instructions (recipe_id, position, section, text) \
             VALUES (?, ?, ?, ?)",
        )
        .bind(recipe_id)
        .bind(position as i64)
        .bind(step.section.as_deref().map(str::trim).filter(|s| !s.is_empty()))
        .bind(step.text.trim())
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

fn validate_recipe_input(input: &RecipeInput) -> Result<(), ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError(
            (StatusCode::UNPROCESSABLE_ENTITY, "recipe name must not be empty").into_response(),
        ));
    }
    // Quantities, when present, must be positive and finite.
    for ingredient in &input.ingredients {
        if let Some(quantity) = ingredient.quantity {
            if !quantity.is_finite() || quantity <= 0.0 {
                return Err(ApiError(
                    (
                        StatusCode::UNPROCESSABLE_ENTITY,
                        "ingredient quantities must be positive numbers",
                    )
                        .into_response(),
                ));
            }
        }
    }
    Ok(())
}

async fn create_recipe(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(input): Json<RecipeInput>,
) -> Result<(StatusCode, Json<Recipe>), ApiError> {
    validate_recipe_input(&input)?;
    let recipe = insert_recipe(&state.db, &input, None).await?;
    Ok((StatusCode::CREATED, Json(recipe)))
}

/// Save an uploaded photo: original bytes kept as-is for the details page,
/// plus a compressed JPEG thumbnail (max width 480px) for the recipe grid.
/// The format is detected from the bytes (not the declared content type).
/// Returns the relative paths (under the image dir) for both files.
fn save_recipe_image(data_dir: &std::path::Path, bytes: &[u8]) -> anyhow::Result<(String, String)> {
    let reader = image::ImageReader::new(std::io::Cursor::new(bytes))
        .with_guessed_format()
        .context("failed to read image")?;
    let format = reader
        .format()
        .context("unknown image format (use JPEG, PNG or WebP)")?;
    let ext = match format {
        image::ImageFormat::Jpeg => "jpg",
        image::ImageFormat::Png => "png",
        image::ImageFormat::WebP => "webp",
        other => anyhow::bail!("unsupported image format: {other:?}"),
    };
    let id = uuid::Uuid::new_v4();

    let image_path = format!("images/{id}.{ext}");
    std::fs::write(data_dir.join(&image_path), bytes)
        .context("failed to write full-resolution image")?;

    // Compressed thumbnail for the list view.
    let img = reader.decode().context("failed to decode image")?;
    let thumb = img.thumbnail(480, u32::MAX);
    let thumb_path = format!("images/thumbs/{id}.jpg");
    thumb
        .save(data_dir.join(&thumb_path))
        .context("failed to write thumbnail")?;
    Ok((image_path, thumb_path))
}

/// A parsed multipart recipe payload: structured fields + optional photo.
struct MultipartRecipe {
    input: RecipeInput,
    image: Option<(String, String)>,
}

/// Parse `name`, structured lists (JSON), the meta strings and an optional
/// `image` file field.
async fn parse_recipe_multipart(
    state: &AppState,
    multipart: &mut axum::extract::Multipart,
) -> Result<MultipartRecipe, ApiError> {
    let mut name = String::new();
    let mut sections = String::new();
    let mut ingredients = String::new();
    let mut instructions = String::new();
    let mut instruction_sections = String::new();
    let mut notes = String::new();
    let mut yield_amount = String::new();
    let mut source = String::new();
    let mut image: Option<(String, String)> = None;
    let mut image_url = String::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::client(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?
    {
        match field.name().unwrap_or_default() {
            "name" => name = field.text().await.unwrap_or_default(),
            "sections" => sections = field.text().await.unwrap_or_default(),
            "ingredients" => ingredients = field.text().await.unwrap_or_default(),
            "instructions" => instructions = field.text().await.unwrap_or_default(),
            "instruction_sections" => instruction_sections = field.text().await.unwrap_or_default(),
            "notes" => notes = field.text().await.unwrap_or_default(),
            "yield" => yield_amount = field.text().await.unwrap_or_default(),
            "source" => source = field.text().await.unwrap_or_default(),
            "image_url" => image_url = field.text().await.unwrap_or_default(),
            "image" => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::client(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
                if !bytes.is_empty() {
                    image =
                        Some(save_recipe_image(&state.data_dir, &bytes).map_err(ApiError::from)?);
                }
            }
            _ => {}
        }
    }

    // A remote image URL (recipe import): download it now — only for recipes
    // actually being saved. A manual photo upload always wins; failures are
    // soft (the recipe saves without a photo).
    if image.is_none() && !image_url.trim().is_empty() {
        match recipe_import::fetch::download_image(image_url.trim(), state.import_allow_private).await {
            Ok(bytes) => match save_recipe_image(&state.data_dir, &bytes) {
                Ok(paths) => image = Some(paths),
                Err(err) => {
                    tracing::warn!("imported image could not be stored: {err:#}");
                }
            },
            Err(err) => {
                tracing::warn!("imported image could not be downloaded: {err}");
            }
        }
    }

    let sections: Vec<String> = if sections.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&sections).map_err(|e| {
            ApiError::client(StatusCode::UNPROCESSABLE_ENTITY, format!("bad sections JSON: {e}"))
        })?
    };

    let ingredients: Vec<Ingredient> = if ingredients.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&ingredients).map_err(|e| {
            ApiError::client(StatusCode::UNPROCESSABLE_ENTITY, format!("bad ingredients JSON: {e}"))
        })?
    };
    let instructions: Vec<InstructionStep> = if instructions.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&instructions).map_err(|e| {
            ApiError::client(StatusCode::UNPROCESSABLE_ENTITY, format!("bad instructions JSON: {e}"))
        })?
    };
    let instruction_sections: Vec<String> = if instruction_sections.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&instruction_sections).map_err(|e| {
            ApiError::client(
                StatusCode::UNPROCESSABLE_ENTITY,
                format!("bad instruction_sections JSON: {e}"),
            )
        })?
    };

    Ok(MultipartRecipe {
        input: RecipeInput {
            name,
            sections,
            ingredients,
            instructions,
            instruction_sections,
            notes,
            yield_amount,
            source,
        },
        image,
    })
}

/// `POST /api/recipes/photo`: multipart create with an optional photo.
async fn create_recipe_with_photo(
    axum::extract::State(state): axum::extract::State<AppState>,
    mut multipart: axum::extract::Multipart,
) -> Result<(StatusCode, Json<Recipe>), ApiError> {
    let MultipartRecipe { input, image } =
        parse_recipe_multipart(&state, &mut multipart).await?;
    validate_recipe_input(&input)?;
    let recipe = insert_recipe(&state.db, &input, image).await?;
    Ok((StatusCode::CREATED, Json(recipe)))
}

/// `GET /api/recipes/{id}`: full detail view.
async fn recipe_detail(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<Json<RecipeDetail>, ApiError> {
    load_recipe_detail(&state.db, id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError((StatusCode::NOT_FOUND, "not found").into_response()))
}

/// `PUT /api/recipes/{id}`: multipart update, optional photo replacement.
async fn update_recipe(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    mut multipart: axum::extract::Multipart,
) -> Result<Json<RecipeDetail>, ApiError> {
    let MultipartRecipe { input, image } =
        parse_recipe_multipart(&state, &mut multipart).await?;
    validate_recipe_input(&input)?;

    let existing = sqlx::query(
        "SELECT image_path, thumb_path FROM recipes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(&state.db)
    .await?
    .context("no such recipe")?;
    let (old_image, old_thumb): (Option<String>, Option<String>) = {
        use sqlx::Row;
        (existing.get("image_path"), existing.get("thumb_path"))
    };

    let mut tx = state.db.begin().await?;
    let (new_image, new_thumb) = match &image {
        Some((img, thumb)) => {
            delete_image_files(&state.data_dir, old_image.as_deref(), old_thumb.as_deref());
            (Some(img.clone()), Some(thumb.clone()))
        }
        None => (old_image, old_thumb),
    };
    let updated = sqlx::query(
        "UPDATE recipes SET name = ?, notes = ?, yield_amount = ?, source = ?, \
         image_path = ?, thumb_path = ? WHERE id = ? RETURNING id",
    )
    .bind(input.name.trim())
    .bind(input.notes.trim())
    .bind(input.yield_amount.trim())
    .bind(input.source.trim())
    .bind(new_image)
    .bind(new_thumb)
    .bind(id)
    .fetch_optional(&mut *tx)
    .await?;
    if updated.is_none() {
        return Err(ApiError((StatusCode::NOT_FOUND, "no such recipe").into_response()));
    }
    replace_details(&mut tx, id, &input).await?;
    tx.commit().await?;

    load_recipe_detail(&state.db, id)
        .await?
        .map(Json)
        .ok_or_else(|| ApiError((StatusCode::NOT_FOUND, "not found").into_response()))
}

/// `DELETE /api/recipes/{id}`: remove the row (ingredients cascade) and the
/// stored photo files.
async fn delete_recipe(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<StatusCode, ApiError> {
    let row = sqlx::query("SELECT image_path, thumb_path FROM recipes WHERE id = ?")
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .ok_or_else(|| ApiError((StatusCode::NOT_FOUND, "not found").into_response()))?;
    {
        use sqlx::Row;
        delete_image_files(
            &state.data_dir,
            row.get::<Option<String>, _>("image_path").as_deref(),
            row.get::<Option<String>, _>("thumb_path").as_deref(),
        );
    }
    sqlx::query("DELETE FROM recipes WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Serve an uploaded image from the data dir (path-sanitised).
async fn serve_image(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(path): axum::extract::Path<String>,
) -> Result<axum::response::Response, ApiError> {
    if path.contains("..") || path.starts_with('/') || path.is_empty() {
        return Err(ApiError((StatusCode::NOT_FOUND, "not found").into_response()));
    }
    let mime = match path.rsplit('.').next().unwrap_or_default() {
        "jpg" | "jpeg" => "image/jpeg",
        "png" => "image/png",
        "webp" => "image/webp",
        "avif" => "image/avif",
        _ => "application/octet-stream",
    };
    let bytes = tokio::fs::read(state.data_dir.join(&path))
        .await
        .map_err(|_| ApiError((StatusCode::NOT_FOUND, "not found").into_response()))?;
    use axum::http::header;
    Ok((
        [(header::CONTENT_TYPE, mime)],
        bytes,
    )
        .into_response())
}

/// `POST /api/recipes/import`: fetch a recipe webpage, extract a structured
/// recipe and return it as a **preview**. Nothing is persisted here — the
/// client reviews/edits the preview and saves it through the normal
/// create/update endpoints.
#[derive(Debug, Deserialize)]
pub struct RecipeImportRequest {
    pub url: String,
}

async fn import_recipe_from_url(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(request): Json<RecipeImportRequest>,
) -> Result<(StatusCode, Json<RecipePreview>), ApiError> {
    match recipe_import::import(&request.url, state.import_allow_private).await {
        Ok(preview) => Ok((StatusCode::OK, Json(preview))),
        // Bad input / disallowed target / unreachable page → 4xx, extraction
        // failures → 422 (the page is fine, we just cannot read a recipe
        // from it); unexpected errors land in the generic 500 path below.
        Err(err @ (ImportError::InvalidUrl(_)
        | ImportError::DisallowedTarget(_)
        | ImportError::FetchFailed(_))) => Err(ApiError::client(
            StatusCode::BAD_REQUEST,
            err.to_string(),
        )),
        Err(err @ ImportError::ExtractionFailed(_)) => Err(ApiError::client(
            StatusCode::UNPROCESSABLE_ENTITY,
            err.to_string(),
        )),
    }
}

async fn list_recipes(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<Vec<Recipe>>, ApiError> {
    let rows = sqlx::query(
        "SELECT id, name, image_path, thumb_path FROM recipes ORDER BY id DESC",
    )
    .fetch_all(&state.db)
    .await?;
    Ok(Json(rows.iter().map(row_to_recipe).collect()))
}

async fn list_grocery(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<Vec<GroceryItem>>, ApiError> {
    let rows = sqlx::query(&format!("{GROCERY_SELECT} ORDER BY g.id ASC"))
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows.iter().map(row_to_joined_item).collect()))
}

/// Resolve the category for a newly added item: an explicitly typed group
/// always wins; otherwise the cached classification applies instantly; only
/// a cache miss lands in "Other" and is queued for background classification.
/// Returns `(category, needs_classification)`.
async fn resolve_category(db: &SqlitePool, item: &NewGroceryItem) -> (String, bool) {
    if let Some(explicit) = item.category.as_deref().map(str::trim).filter(|c| !c.is_empty()) {
        return (explicit.to_string(), false);
    }
    let cached: Option<String> =
        sqlx::query_scalar("SELECT category FROM ingredient_categories WHERE name = ?")
            .bind(classifier::cache_key(&item.name))
            .fetch_optional(db)
            .await
            .ok()
            .flatten();
    match cached {
        Some(cached) => (cached, false),
        None => (shared::DEFAULT_CATEGORY.to_string(), true),
    }
}

async fn add_grocery_item(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(item): Json<NewGroceryItem>,
) -> Result<(StatusCode, Json<GroceryItem>), ApiError> {
    let name = item.name.trim();
    if name.is_empty() {
        return Err(ApiError(
            (StatusCode::UNPROCESSABLE_ENTITY, "item name must not be empty").into_response(),
        ));
    }
    let (category, needs_classification) = resolve_category(&state.db, &item).await;
    let row = sqlx::query(
        "INSERT INTO grocery_items (name, category) VALUES (?, ?) \
         RETURNING id, name, bought, category",
    )
    .bind(name)
    .bind(category)
    .fetch_one(&state.db)
    .await?;
    if needs_classification {
        if let Some(classifier) = state.classifier.clone() {
            let id = sqlx::Row::get::<i64, _>(&row, "id");
            let db = state.db.clone();
            let key = classifier::cache_key(name);
            tokio::spawn(async move {
                classifier::classify_pending(db, classifier, vec![(id, key)]).await;
            });
        }
    }
    Ok((StatusCode::CREATED, Json(row_to_item(&row))))
}

/// Adds several grocery items in one call. Each becomes its own line —
/// duplicates are intentional (the caller picked them). The whole batch is
/// rejected (and nothing inserted) if it is empty or any name is blank.
/// `recipe_id`, when given, is stamped onto every line as provenance.
async fn add_grocery_batch(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(batch): Json<NewGroceryBatch>,
) -> Result<(StatusCode, Json<Vec<GroceryItem>>), ApiError> {
    if batch.items.is_empty() {
        return Err(ApiError(
            (StatusCode::UNPROCESSABLE_ENTITY, "batch must contain at least one item")
                .into_response(),
        ));
    }
    let mut prepared = Vec::with_capacity(batch.items.len());
    for item in &batch.items {
        let name = item.name.trim();
        if name.is_empty() {
            return Err(ApiError(
                (StatusCode::UNPROCESSABLE_ENTITY, "item name must not be empty").into_response(),
            ));
        }
        let (category, needs_classification) = resolve_category(&state.db, item).await;
        prepared.push((name.to_string(), category, needs_classification));
    }
    // Provenance recipe: validated up front so a bad id rejects the whole
    // batch instead of failing halfway through the inserts.
    let recipe = match batch.recipe_id {
        Some(id) => {
            let row = sqlx::query("SELECT id, name, image_path, thumb_path FROM recipes WHERE id = ?")
                .bind(id)
                .fetch_optional(&state.db)
                .await?
                .ok_or_else(|| {
                    ApiError((StatusCode::NOT_FOUND, "no such recipe").into_response())
                })?;
            Some(row_to_recipe(&row))
        }
        None => None,
    };
    let mut tx = state.db.begin().await?;
    let mut created = Vec::with_capacity(prepared.len());
    let mut pending = Vec::new();
    for (name, category, needs_classification) in prepared {
        let row = sqlx::query(
            "INSERT INTO grocery_items (name, category, recipe_id) VALUES (?, ?, ?) \
             RETURNING id, name, bought, category",
        )
        .bind(&name)
        .bind(&category)
        .bind(batch.recipe_id)
        .fetch_one(&mut *tx)
        .await?;
        if needs_classification {
            pending.push((sqlx::Row::get::<i64, _>(&row, "id"), classifier::cache_key(&name)));
        }
        let mut item = row_to_item(&row);
        item.recipe = recipe.clone();
        created.push(item);
    }
    tx.commit().await?;
    if !pending.is_empty() {
        if let Some(classifier) = state.classifier.clone() {
            let db = state.db.clone();
            tokio::spawn(async move {
                classifier::classify_pending(db, classifier, pending).await;
            });
        }
    }
    Ok((StatusCode::CREATED, Json(created)))
}

async fn update_grocery_item(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(update): Json<GroceryUpdate>,
) -> Result<StatusCode, ApiError> {
    if update.bought {
        let rows = sqlx::query("DELETE FROM grocery_items WHERE id = ?")
            .bind(id)
            .execute(&state.db)
            .await?;
        if rows.rows_affected() == 0 {
            return Err(ApiError((StatusCode::NOT_FOUND, "no such grocery item").into_response()));
        }
        Ok(StatusCode::NO_CONTENT)
    } else {
        let _row = sqlx::query(
            "UPDATE grocery_items SET bought = 0 WHERE id = ? \
             RETURNING id, name, bought, category",
        )
        .bind(id)
        .fetch_optional(&state.db)
        .await?;
        if _row.is_none() {
            return Err(ApiError((StatusCode::NOT_FOUND, "no such grocery item").into_response()));
        }
        Ok(StatusCode::NO_CONTENT)
    }
}

/// Edits a grocery item (rename and/or regroup). A blank name is rejected;
/// a blank or missing category resets the item to the default group. A
/// manual category change is pinned in the cache so future adds of the same
/// item land there and the classifier never overrides it.
async fn patch_grocery_item(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(patch): Json<GroceryPatch>,
) -> Result<Json<GroceryItem>, ApiError> {
    let name = patch.name.trim();
    if name.is_empty() {
        return Err(ApiError(
            (StatusCode::UNPROCESSABLE_ENTITY, "item name must not be empty").into_response(),
        ));
    }
    let category = patch
        .category
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or(shared::DEFAULT_CATEGORY);
    let old: Option<String> =
        sqlx::query_scalar("SELECT category FROM grocery_items WHERE id = ?")
            .bind(id)
            .fetch_optional(&state.db)
            .await?;
    let result = sqlx::query("UPDATE grocery_items SET name = ?, category = ? WHERE id = ?")
        .bind(name)
        .bind(category)
        .bind(id)
        .execute(&state.db)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError((StatusCode::NOT_FOUND, "no such grocery item").into_response()));
    }
    if old.as_deref() != Some(category) {
        classifier::pin_manual(&state.db, name, category).await;
    }
    let row = sqlx::query(&format!("{GROCERY_SELECT} WHERE g.id = ?"))
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok(Json(row_to_joined_item(&row)))
}

async fn delete_grocery_item(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<StatusCode, ApiError> {
    let rows = sqlx::query("DELETE FROM grocery_items WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;
    if rows.rows_affected() == 0 {
        return Err(ApiError((StatusCode::NOT_FOUND, "no such grocery item").into_response()));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Meal plan
// ---------------------------------------------------------------------------

/// One meal-plan row joined with its recipe summary.
fn row_to_meal_plan_entry(row: &sqlx::sqlite::SqliteRow) -> MealPlanEntry {
    use sqlx::Row;
    let (image, thumb) = recipe_urls(row);
    MealPlanEntry {
        id: row.get::<i64, _>("id"),
        date: row.get::<String, _>("date"),
        recipe: Recipe {
            id: row.get::<i64, _>("recipe_id"),
            name: row.get::<String, _>("name"),
            image,
            thumb,
        },
    }
}

const MEAL_PLAN_SELECT: &str = "SELECT m.id, m.date, r.id AS recipe_id, r.name, \
     r.image_path, r.thumb_path \
     FROM meal_plan_entries m JOIN recipes r ON r.id = m.recipe_id";

/// `GET /api/meal-plan`: every entry, ordered by day then insertion order.
async fn list_meal_plan(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<Vec<MealPlanEntry>>, ApiError> {
    let rows = sqlx::query(&format!("{MEAL_PLAN_SELECT} ORDER BY m.date, m.id"))
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows.iter().map(row_to_meal_plan_entry).collect()))
}

/// `POST /api/meal-plan`: schedule a recipe on a day. The date must be a
/// real calendar day in `YYYY-MM-DD` form; duplicates are allowed (leftovers
/// are a thing).
#[derive(Debug, Deserialize)]
struct NewMealPlanEntry {
    date: String,
    recipe_id: i64,
}

async fn add_meal_plan_entry(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(entry): Json<NewMealPlanEntry>,
) -> Result<(StatusCode, Json<MealPlanEntry>), ApiError> {
    if chrono::NaiveDate::parse_from_str(entry.date.trim(), "%Y-%m-%d").is_err() {
        return Err(ApiError(
            (
                StatusCode::UNPROCESSABLE_ENTITY,
                "date must be a real calendar day in YYYY-MM-DD form",
            )
                .into_response(),
        ));
    }
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO meal_plan_entries (date, recipe_id) VALUES (?, ?) RETURNING id",
    )
    .bind(entry.date.trim())
    .bind(entry.recipe_id)
    .fetch_optional(&state.db)
    .await
    .map_err(|err| match &err {
        sqlx::Error::Database(db) if db.message().contains("FOREIGN KEY") => {
            ApiError((StatusCode::NOT_FOUND, "no such recipe").into_response())
        }
        _ => ApiError::from(err),
    })?
    .ok_or_else(|| ApiError((StatusCode::NOT_FOUND, "no such recipe").into_response()))?;
    let row = sqlx::query(&format!("{MEAL_PLAN_SELECT} WHERE m.id = ?"))
        .bind(id)
        .fetch_one(&state.db)
        .await?;
    Ok((StatusCode::CREATED, Json(row_to_meal_plan_entry(&row))))
}

/// `DELETE /api/meal-plan/{id}`: unschedule one entry.
async fn delete_meal_plan_entry(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
) -> Result<StatusCode, ApiError> {
    let rows = sqlx::query("DELETE FROM meal_plan_entries WHERE id = ?")
        .bind(id)
        .execute(&state.db)
        .await?;
    if rows.rows_affected() == 0 {
        return Err(ApiError((StatusCode::NOT_FOUND, "no such meal plan entry").into_response()));
    }
    Ok(StatusCode::NO_CONTENT)
}

// ---------------------------------------------------------------------------
// Tests (router level, no HTTP server or browser required)
// ---------------------------------------------------------------------------

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    pub(crate) fn test_config(base_path: Option<&str>, static_dir: Option<PathBuf>) -> Config {
        Config {
            addr: SocketAddr::from(([127, 0, 0, 1], 0)),
            db_url: String::new(),
            base_path: base_path.map(String::from),
            static_dir,
            data_dir: std::env::temp_dir().join(format!("bouedig-test-{}", uuid::Uuid::new_v4())),
            import_allow_private: false,
            openrouter_key: None,
            classifier_model: None,
            classifier_endpoint: None,
        }
    }

    pub(crate) async fn test_router(base_path: Option<&str>) -> Router {
        test_router_with_config(base_path).await.0
    }

    /// Router (plus its pool for cache seeding/inspection) with a classifier
    /// pointed at `endpoint` — a local mock server in the tests.
    pub(crate) async fn test_router_with_classifier(
        endpoint: String,
    ) -> (Router, SqlitePool) {
        let config = test_config(None, None);
        ensure_data_dirs(&config.data_dir).unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        run_migrations(&pool).await.unwrap();
        let state = AppState {
            db: pool.clone(),
            data_dir: config.data_dir.clone(),
            import_allow_private: true,
            classifier: classifier::Classifier::from_parts(
                Some("test-key".into()),
                Some("typesafe-ai/jev".into()),
                Some(endpoint),
            ),
        };
        (build_router(state.clone(), &config), state.db)
    }

    /// HTTP server that counts requests and always answers `body` with
    /// `status` — a stand-in for the OpenRouter chat endpoint.
    pub(crate) fn counting_server(
        body: &'static str,
        status: &'static str,
        hits: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    ) -> SocketAddr {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buffer = [0u8; 8192];
                use std::io::{Read, Write};
                let _ = stream.read(&mut buffer);
                hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let response = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        addr
    }

    /// An OpenRouter chat response whose message content is `content`.
    fn openrouter_answer(content: &str) -> &'static str {
        let body = serde_json::json!({
            "choices": [{"message": {"content": content}}]
        })
        .to_string();
        Box::leak(body.into_boxed_str())
    }

    /// Router whose importer may fetch loopback targets (fixture servers).
    pub(crate) async fn test_router_import() -> Router {
        let config = test_config(None, None);
        ensure_data_dirs(&config.data_dir).unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        run_migrations(&pool).await.unwrap();
        let config = Config {
            import_allow_private: true,
            ..config
        };
        build_router(AppState::from_config(pool, &config), &config)
    }

    pub(crate) async fn test_router_with_config(base_path: Option<&str>) -> (Router, Config) {
        let config = test_config(base_path, None);
        ensure_data_dirs(&config.data_dir).unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        run_migrations(&pool).await.unwrap();
        (
            build_router(AppState::from_config(pool, &config), &config),
            config,
        )
    }

    pub(crate) async fn json_response(
        app: Router,
        method: &str,
        uri: &str,
        body: Option<&str>,
    ) -> (StatusCode, String) {
        let builder = Request::builder().method(method).uri(uri);
        let request = match body {
            Some(b) => builder
                .header("content-type", "application/json")
                .body(Body::from(b.to_string()))
                .unwrap(),
            None => builder.body(Body::empty()).unwrap(),
        };
        let resp = app.oneshot(request).await.unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, String::from_utf8_lossy(&bytes).into_owned())
    }

    #[tokio::test]
    async fn create_recipe_stores_details_and_leaves_grocery_alone() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/recipes",
            Some(
                r#"{"name":"Soup","ingredients":[
                     {"quantity":300,"unit":"ml","name":"water","prep":null},
                     {"quantity":1,"unit":"tbsp","name":"salt","prep":"to taste"}],
                   "instructions":[{"text":"Boil water"},{"text":"Add salt"}]}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let recipe: Recipe = serde_json::from_str(&body).unwrap();
        let id = recipe.id;

        // Detail view round-trips the structured data.
        let (status, body) = json_response(app.clone(), "GET", &format!("/api/recipes/{id}"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let detail: RecipeDetail = serde_json::from_str(&body).unwrap();
        assert_eq!(detail.name, "Soup");
        assert_eq!(detail.ingredients.len(), 2);
        assert_eq!(detail.ingredients[0].name, "water");
        assert_eq!(detail.ingredients[0].unit.as_deref(), Some("ml"));
        assert_eq!(detail.ingredients[1].quantity, Some(1.0));
        assert_eq!(detail.instructions, vec![InstructionStep { text: "Boil water".into(), section: None }, InstructionStep { text: "Add salt".into(), section: None }]);

        // The shopping list must NOT receive recipe ingredients anymore.
        let (status, body) = json_response(app, "GET", "/api/grocery", None).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body.trim(), "[]", "grocery list must stay empty: {body}");
    }

    #[tokio::test]
    async fn sections_round_trip() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/recipes",
            Some(
                r#"{"name":"Crêpe","sections":["Crêpes","Filling"],
                    "ingredients":[
                      {"quantity":180,"unit":"g","name":"buckwheat flour","section":"Crêpes"},
                      {"quantity":3,"name":"tomatoes","section":"Filling"}],
                    "instructions":[]}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let recipe: Recipe = serde_json::from_str(&body).unwrap();

        let (status, body) = json_response(app.clone(), "GET", &format!("/api/recipes/{}", recipe.id), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let detail: RecipeDetail = serde_json::from_str(&body).unwrap();
        assert_eq!(detail.sections, vec!["Crêpes", "Filling"]);
        assert_eq!(detail.ingredients[0].section.as_deref(), Some("Crêpes"));

        // PUT with a renamed section keeps order; empty sections persist.
        let _boundary = "SecBNd";
        let payload = concat!(
            "--SecBNd\r\n",
            "Content-Disposition: form-data; name=\"name\"\r\n\r\n",
            "Crêpe\r\n",
            "--SecBNd\r\n",
            "Content-Disposition: form-data; name=\"sections\"\r\n\r\n",
            "[\"Base\",\"Empty\",\"Filling\"]\r\n",
            "--SecBNd\r\n",
            "Content-Disposition: form-data; name=\"ingredients\"\r\n\r\n",
            "[{\"quantity\":1,\"name\":\"tomato\",\"section\":\"Filling\"}]\r\n",
            "--SecBNd--\r\n",
        );
        let request = axum::http::Request::builder()
            .method("PUT")
            .uri(format!("/api/recipes/{}", recipe.id))
            .header("content-type", "multipart/form-data; boundary=SecBNd")
            .body(Body::from(payload))
            .unwrap();
        let resp = app.clone().oneshot(request).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let detail: RecipeDetail =
            serde_json::from_slice(&to_bytes(resp.into_body(), usize::MAX).await.unwrap().as_ref()).unwrap();
        assert_eq!(detail.sections, vec!["Base", "Empty", "Filling"]);
        assert_eq!(detail.ingredients.len(), 1);
        assert_eq!(detail.ingredients[0].section.as_deref(), Some("Filling"));
    }

    #[tokio::test]
    async fn update_recipe_replaces_fields() {
        let app = test_router(None).await;
        let (_, body) = json_response(
            app.clone(),
            "POST",
            "/api/recipes",
            Some(r#"{"name":"Old","ingredients":[{"quantity":1,"unit":null,"name":"onion","prep":null}],"instructions":[{"text":"a"}]}"#),
        )
        .await;
        let recipe: Recipe = serde_json::from_str(&body).unwrap();

        // PUT is multipart (same format as the client sends).
        let payload = concat!(
            "--UpDbOuNd\r\n",
            "Content-Disposition: form-data; name=\"name\"\r\n\r\n",
            "New\r\n",
            "--UpDbOuNd\r\n",
            "Content-Disposition: form-data; name=\"ingredients\"\r\n\r\n",
            "[{\"quantity\":2,\"unit\":\"g\",\"name\":\"carrot\",\"prep\":\"grated\"}]\r\n",
            "--UpDbOuNd\r\n",
            "Content-Disposition: form-data; name=\"instructions\"\r\n\r\n",
            "[{\"text\":\"step one\"},{\"text\":\"step two\"}]\r\n",
            "--UpDbOuNd--\r\n",
        );
        let request = axum::http::Request::builder()
            .method("PUT")
            .uri(format!("/api/recipes/{}", recipe.id))
            .header("content-type", "multipart/form-data; boundary=UpDbOuNd")
            .body(Body::from(payload))
            .unwrap();
        let resp = app.clone().oneshot(request).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let detail: RecipeDetail = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(detail.name, "New");
        assert_eq!(detail.ingredients.len(), 1);
        assert_eq!(detail.ingredients[0].name, "carrot");
        assert_eq!(detail.ingredients[0].prep.as_deref(), Some("grated"));
        assert_eq!(detail.instructions.len(), 2);
    }

    #[tokio::test]
    async fn delete_recipe_removes_row() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/recipes",
            Some(r#"{"name":"Goner","ingredients":[],"instructions":[]}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let recipe: Recipe = serde_json::from_str(&body).unwrap();

        let (status, _) = json_response(app.clone(), "DELETE", &format!("/api/recipes/{}", recipe.id), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, _) = json_response(app, "GET", &format!("/api/recipes/{}", recipe.id), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn grocery_toggle_and_manual_add_persist() {
        let app = test_router(None).await;
        let (status, body) = json_response(app.clone(), "POST", "/api/grocery", Some(r#"{"name":"Rice"}"#)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let id: serde_json::Value = serde_json::from_str(&body).unwrap();
        let id = id["id"].as_i64().unwrap();

        // Toggle to bought -> item should be deleted
        let (status, _) = json_response(
            app.clone(),
            "PATCH",
            &format!("/api/grocery/{id}"),
            Some(r#"{"bought":true}"#),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

        // Item should no longer be in the list
        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        assert_eq!(body.trim(), "[]", "grocery list must be empty after toggle: {body}");
    }

    #[tokio::test]
    async fn grocery_batch_add_creates_every_line_in_order() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/grocery/batch",
            Some(
                r#"{"items":[{"name":"2 tbsp soy sauce"},{"name":"1 onion, thinly sliced"},{"name":"  Salt  "}]}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let created: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(created.as_array().unwrap().len(), 3, "{body}");
        assert_eq!(created[2]["name"], "Salt", "names must be trimmed: {body}");
        assert_eq!(created[0]["category"], "Other", "default category: {body}");

        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        let names: Vec<&str> = list
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            vec!["2 tbsp soy sauce", "1 onion, thinly sliced", "Salt"],
            "batch order must be preserved: {body}"
        );
    }

    #[tokio::test]
    async fn grocery_batch_add_supports_explicit_categories() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/grocery/batch",
            Some(r#"{"items":[{"name":"Basil","category":"Fresh"},{"name":"Pasta"}]}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list[0]["category"], "Fresh", "{body}");
        assert_eq!(list[1]["category"], "Other", "{body}");
    }

    #[tokio::test]
    async fn grocery_batch_add_rejects_empty_batch() {
        let app = test_router(None).await;
        let (status, body) =
            json_response(app, "POST", "/api/grocery/batch", Some(r#"{"items":[]}"#)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    }

    #[tokio::test]
    async fn grocery_batch_add_rejects_blank_name_without_partial_inserts() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/grocery/batch",
            Some(r#"{"items":[{"name":"Milk"},{"name":"   "},{"name":"Eggs"}]}"#),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        assert_eq!(
            body.trim(),
            "[]",
            "a rejected batch must not leave partial rows: {body}"
        );
    }

    #[tokio::test]
    async fn grocery_batch_add_keeps_order_over_many_lines() {
        let app = test_router(None).await;
        let items: Vec<String> = (0..20).map(|i| format!(r#"{{"name":"item {i}"}}"#)).collect();
        let payload = format!(r#"{{"items":[{}]}}"#, items.join(","));
        let (status, body) =
            json_response(app.clone(), "POST", "/api/grocery/batch", Some(&payload)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        for (i, item) in list.as_array().unwrap().iter().enumerate() {
            assert_eq!(item["name"], format!("item {i}"), "order broken at {i}: {body}");
        }
    }

    #[tokio::test]
    async fn grocery_batch_stamps_recipe_provenance() {
        let app = test_router(None).await;
        let recipe_id = create_test_recipe(app.clone(), "Provenance Soup").await;

        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/grocery/batch",
            Some(&format!(
                r#"{{"items":[{{"name":"2 tbsp soy sauce"}},{{"name":"Salt"}}],"recipe_id":{recipe_id}}}"#
            )),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        for item in list.as_array().unwrap() {
            assert_eq!(item["recipe"]["id"], recipe_id, "{body}");
            assert_eq!(item["recipe"]["name"], "Provenance Soup", "{body}");
        }
    }

    #[tokio::test]
    async fn grocery_batch_unknown_recipe_rejects_everything() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/grocery/batch",
            Some(r#"{"items":[{"name":"Milk"}],"recipe_id":424242}"#),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        assert_eq!(
            body.trim(), "[]",
            "a batch with an unknown recipe must not insert anything: {body}"
        );
    }

    #[tokio::test]
    async fn grocery_manual_add_has_no_provenance() {
        let app = test_router(None).await;
        let (status, _) =
            json_response(app.clone(), "POST", "/api/grocery", Some(r#"{"name":"Rice"}"#)).await;
        assert_eq!(status, StatusCode::CREATED);
        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert!(
            list[0]["recipe"].is_null(),
            "manual adds must not carry provenance: {body}"
        );
    }

    #[tokio::test]
    async fn grocery_put_edit_renames_and_regroups() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/grocery",
            Some(r#"{"name":"Oats","category":"Pantry"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let id = serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
            .as_i64()
            .unwrap();

        let (status, body) = json_response(
            app.clone(),
            "PUT",
            &format!("/api/grocery/{id}"),
            Some(r#"{"name":"Rolled oats","category":"Breakfast"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let edited: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(edited["name"], "Rolled oats", "{body}");
        assert_eq!(edited["category"], "Breakfast", "{body}");

        // Blank category resets to the default group.
        let (status, body) = json_response(
            app.clone(),
            "PUT",
            &format!("/api/grocery/{id}"),
            Some(r#"{"name":"Rolled oats","category":"  "}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let edited: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(edited["category"], "Other", "{body}");

        // Blank name → 422, unknown id → 404.
        let (status, _) = json_response(
            app.clone(),
            "PUT",
            &format!("/api/grocery/{id}"),
            Some(r#"{"name":"   "}"#),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let (status, _) = json_response(app, "PUT", "/api/grocery/999999", Some(r#"{"name":"X"}"#))
            .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn grocery_provenance_clears_when_recipe_deleted() {
        let app = test_router(None).await;
        let recipe_id = create_test_recipe(app.clone(), "Doomed Dish").await;
        let (status, _) = json_response(
            app.clone(),
            "POST",
            "/api/grocery/batch",
            Some(&format!(
                r#"{{"items":[{{"name":"1 onion"}}],"recipe_id":{recipe_id}}}"#
            )),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED);

        let (status, _) =
            json_response(app.clone(), "DELETE", &format!("/api/recipes/{recipe_id}"), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        // The item survives; its provenance is cleared.
        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list.as_array().unwrap().len(), 1, "{body}");
        assert_eq!(list[0]["name"], "1 onion", "{body}");
        assert!(list[0]["recipe"].is_null(), "{body}");
    }

    /// Creates a minimal recipe and returns its id.
    async fn create_test_recipe(app: axum::Router, name: &str) -> i64 {
        let payload = format!(
            r#"{{"name":"{name}","sections":[],"ingredients":[{{"quantity":1.0,"unit":"tbsp","name":"oil","prep":null,"section":null}}],"instructions":[],"instruction_sections":[],"notes":"","yield":"","source":""}}"#
        );
        let (status, body) = json_response(app, "POST", "/api/recipes", Some(&payload)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        serde_json::from_str::<serde_json::Value>(&body).unwrap()["id"]
            .as_i64()
            .unwrap()
    }

    #[tokio::test]
    async fn grocery_cache_hit_skips_the_api() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server = counting_server(
            openrouter_answer(r#"{"category":"Vegan","confidence":0.9}"#),
            "200 OK",
            hits.clone(),
        );
        let (app, pool) = test_router_with_classifier(format!("http://{server}/v1/chat/completions")).await;
        sqlx::query("INSERT INTO ingredient_categories (name, category) VALUES ('onion', 'Vegetables')")
            .execute(&pool)
            .await
            .unwrap();

        let (status, body) =
            json_response(app.clone(), "POST", "/api/grocery", Some(r#"{"name":"Onion"}"#)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let created: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(created["category"], "Vegetables", "cache hit must apply instantly: {body}");

        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0, "cached names must never reach the API");
    }

    #[tokio::test]
    async fn grocery_background_classification_updates_and_remembers() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server = counting_server(
            openrouter_answer(r#"{"category":"Vegan","confidence":0.9}"#),
            "200 OK",
            hits.clone(),
        );
        let (app, pool) = test_router_with_classifier(format!("http://{server}/v1/chat/completions")).await;

        // The add answers immediately with the default; the flip happens later.
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/grocery/batch",
            Some(r#"{"items":[{"name":"1 lb firm tofu"}]}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let created: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(created[0]["category"], "Other", "cache miss starts in Other: {body}");

        // The background task moves it (and everything of that name) out.
        let mut flipped = false;
        for _ in 0..40 {
            let (_, body) = json_response(app.clone(), "GET", "/api/grocery", None).await;
            let list: serde_json::Value = serde_json::from_str(&body).unwrap();
            if list[0]["category"] == "Vegan" {
                flipped = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(flipped, "background classification never applied the category");
        let cached: String =
            sqlx::query_scalar("SELECT category FROM ingredient_categories WHERE name = '1 lb firm tofu'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cached, "Vegan");

        // A repeat add resolves from the cache — the API was called once.
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/grocery",
            Some(r#"{"name":"1 lb Firm Tofu"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let created: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(created["category"], "Vegan", "repeat add must use the cache: {body}");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn grocery_low_confidence_stays_other_and_is_not_cached() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server = counting_server(
            openrouter_answer(r#"{"category":"Vegan","confidence":0.3}"#),
            "200 OK",
            hits.clone(),
        );
        let (app, pool) = test_router_with_classifier(format!("http://{server}/v1/chat/completions")).await;

        let (status, _) =
            json_response(app.clone(), "POST", "/api/grocery", Some(r#"{"name":"mysterygoo"}"#)).await;
        assert_eq!(status, StatusCode::CREATED);
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list[0]["category"], "Other", "{body}");
        let cached: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ingredient_categories")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(cached, 0, "low-confidence answers must not be cached");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1, "the API was consulted once");
    }

    #[tokio::test]
    async fn grocery_api_failure_stays_other() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server = counting_server("{}", "500 Internal Server Error", hits.clone());
        let (app, pool) = test_router_with_classifier(format!("http://{server}/v1/chat/completions")).await;

        let (status, body) =
            json_response(app.clone(), "POST", "/api/grocery", Some(r#"{"name":"carrot"}"#)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        tokio::time::sleep(std::time::Duration::from_millis(400)).await;

        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        let list: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(list[0]["category"], "Other", "API failure must fall back to Other: {body}");
        let cached: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM ingredient_categories")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(cached, 0);
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn grocery_explicit_category_skips_classification() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server = counting_server(
            openrouter_answer(r#"{"category":"Vegan","confidence":0.9}"#),
            "200 OK",
            hits.clone(),
        );
        let (app, _pool) = test_router_with_classifier(format!("http://{server}/v1/chat/completions")).await;

        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/grocery",
            Some(r#"{"name":"Dish soap","category":" Non-Food "}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let created: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(created["category"], "Non-Food", "{body}");
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0, "explicit groups must never reach the API");
    }

    #[tokio::test]
    async fn grocery_manual_move_pins_the_category_over_the_classifier() {
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let server = counting_server(
            openrouter_answer(r#"{"category":"Vegan","confidence":0.9}"#),
            "200 OK",
            hits.clone(),
        );
        let (app, pool) = test_router_with_classifier(format!("http://{server}/v1/chat/completions")).await;

        // JEV says Vegan and the add lands there.
        let (status, body) =
            json_response(app.clone(), "POST", "/api/grocery", Some(r#"{"name":"milk"}"#)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let created: serde_json::Value = serde_json::from_str(&body).unwrap();
        let id = created["id"].as_i64().unwrap();
        let mut flipped = false;
        for _ in 0..40 {
            let (_, body) = json_response(app.clone(), "GET", "/api/grocery", None).await;
            if body.contains("\"Vegan\"") {
                flipped = true;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        }
        assert!(flipped, "JEV answer never applied");

        // The user manually moves it to Drinks.
        let (status, body) = json_response(
            app.clone(),
            "PUT",
            &format!("/api/grocery/{id}"),
            Some(r#"{"name":"milk","category":"Drinks"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let pinned: (String, i64) =
            sqlx::query_as("SELECT category, pinned FROM ingredient_categories WHERE name = 'milk'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(pinned, ("Drinks".into(), 1), "manual choice must be pinned");

        // Mark done (row deleted), then re-add: the pinned choice applies…
        let (status, _) =
            json_response(app.clone(), "PATCH", &format!("/api/grocery/{id}"), Some(r#"{"bought":true}"#))
                .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (status, body) =
            json_response(app.clone(), "POST", "/api/grocery", Some(r#"{"name":"milk"}"#)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let recreated: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(recreated["category"], "Drinks", "re-add must land in the pinned category: {body}");

        // …and a later JEV answer for the same name cannot overwrite the pin.
        classifier::remember(&pool, "milk", "Vegan").await;
        let pinned: (String, i64) =
            sqlx::query_as("SELECT category, pinned FROM ingredient_categories WHERE name = 'milk'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(pinned, ("Drinks".into(), 1), "classifier must not overwrite a pin");
    }

    #[tokio::test]
    async fn grocery_delete_removes_only_the_target_item() {
        let app = test_router(None).await;
        let mut ids = Vec::new();
        for name in ["Rice", "Tofu", "Soy sauce"] {
            let (status, body) =
                json_response(app.clone(), "POST", "/api/grocery", Some(&format!(r#"{{"name":"{name}"}}"#)))
                    .await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
            let item: GroceryItem = serde_json::from_str(&body).unwrap();
            ids.push(item.id);
        }

        // DELETE the middle item.
        let (status, body) = json_response(app.clone(), "DELETE", &format!("/api/grocery/{}", ids[1]), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT, "{body}");

        let (_, body) = json_response(app.clone(), "GET", "/api/grocery", None).await;
        let items: Vec<GroceryItem> = serde_json::from_str(&body).unwrap();
        assert_eq!(
            items.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(),
            vec!["Rice", "Soy sauce"],
            "only the deleted item must disappear: {body}"
        );

        // Deleting again (or an unknown id) is a 404, not a 500.
        let (status, _) = json_response(app.clone(), "DELETE", &format!("/api/grocery/{}", ids[1]), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // PATCH to bought=false unmarks without deleting.
        let (status, _) = json_response(
            app.clone(),
            "PATCH",
            &format!("/api/grocery/{}", ids[0]),
            Some(r#"{"bought":false}"#),
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        let items: Vec<GroceryItem> = serde_json::from_str(&body).unwrap();
        assert_eq!(items.len(), 2, "bought=false must not delete: {body}");
        assert!(!items[0].bought);
    }

    #[tokio::test]
    async fn grocery_patch_unknown_id_is_404() {
        let app = test_router(None).await;
        for payload in [r#"{"bought":true}"#, r#"{"bought":false}"#] {
            let (status, _) = json_response(app.clone(), "PATCH", "/api/grocery/9999", Some(payload)).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "payload {payload}");
        }
    }

    #[tokio::test]
    async fn grocery_add_after_removal_keeps_the_list_consistent() {
        // Simulates the client flow behind the "removed items come back" bug:
        // remove an item, then add another; the removed one must stay gone.
        let app = test_router(None).await;
        let (status, body) = json_response(app.clone(), "POST", "/api/grocery", Some(r#"{"name":"Bananas"}"#)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let item: GroceryItem = serde_json::from_str(&body).unwrap();

        let (status, _) = json_response(app.clone(), "DELETE", &format!("/api/grocery/{}", item.id), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);

        let (status, body) = json_response(app.clone(), "POST", "/api/grocery", Some(r#"{"name":"Flour"}"#)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        let items: Vec<GroceryItem> = serde_json::from_str(&body).unwrap();
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        assert_eq!(names, vec!["Flour"], "removed item must not come back: {body}");
    }

    #[tokio::test]
    async fn non_positive_quantities_are_rejected() {
        let app = test_router(None).await;
        for bad in ["-2", "0", "0.0"] {
            let payload = format!(
                r#"{{"name":"X","ingredients":[{{"quantity":{bad},"name":"salt"}}],"instructions":[]}}"#
            );
            let (status, body) = json_response(app.clone(), "POST", "/api/recipes", Some(&payload)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "qty {bad}: {body}");
        }
    }

    #[tokio::test]
    async fn meta_fields_round_trip() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/recipes",
            Some(
                r#"{"name":"Crêpe","notes":"Keep the batter cold.",
                    "yield":"12 crêpes","source":"Grandma",
                    "ingredients":[],"instructions":[]}"#,
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let recipe: Recipe = serde_json::from_str(&body).unwrap();
        let (status, body) =
            json_response(app, "GET", &format!("/api/recipes/{}", recipe.id), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let detail: RecipeDetail = serde_json::from_str(&body).unwrap();
        assert_eq!(detail.notes, "Keep the batter cold.");
        assert_eq!(detail.yield_amount, "12 crêpes");
        assert_eq!(detail.source, "Grandma");
    }

    #[tokio::test]
    async fn empty_recipe_name_is_rejected() {
        let app = test_router(None).await;
        let (status, _) = json_response(
            app,
            "POST",
            "/api/recipes",
            Some(r#"{"name":"   ","ingredients":[],"instructions":[]}"#),
        )
        .await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[tokio::test]
    async fn base_path_nests_api_and_redirects_trailing_slash() {
        let app = test_router(Some("/bouedig")).await;
        let (status, body) = json_response(app.clone(), "GET", "/bouedig/api/grocery", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");

        let (status, _) = json_response(app, "GET", "/bouedig/", None).await;
        assert_eq!(status, StatusCode::PERMANENT_REDIRECT);
    }

    // -------------------------------------------------------------------
    // Recipe import (`POST /api/recipes/import`)
    // -------------------------------------------------------------------

    /// A minimal one-shot HTTP server serving `body` for every request on a
    /// free loopback port. Detached: lives until the test process ends.
    fn fixture_server(body: &'static str) -> SocketAddr {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buffer = [0u8; 4096];
                use std::io::{Read, Write};
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                );
                let _ = stream.write_all(response.as_bytes());
            }
        });
        addr
    }

    fn fixture_page(name: &str) -> &'static str {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/recipe_import")
            .join(name);
        std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fixture {name} unreadable: {e}"))
            .leak()
    }

    #[tokio::test]
    async fn import_rejects_invalid_urls() {
        let app = test_router(None).await;
        for url in ["", "not a url", "file:///etc/passwd", "ftp://example.com/x"] {
            let payload = serde_json::json!({ "url": url }).to_string();
            let (status, body) =
                json_response(app.clone(), "POST", "/api/recipes/import", Some(&payload)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "url {url:?}: {body}");
        }
    }

    #[tokio::test]
    async fn import_rejects_private_network_targets() {
        // Without the test opt-in, loopback/private targets must be refused
        // before any request is made.
        let app = test_router(None).await;
        for url in [
            "http://127.0.0.1:9/recipe",
            "http://localhost:3000/recipe",
            "http://10.0.0.5/recipe",
            "http://192.168.1.10/recipe",
            "http://169.254.169.254/meta",
            "http://[::1]/recipe",
        ] {
            let payload = serde_json::json!({ "url": url }).to_string();
            let (status, body) =
                json_response(app.clone(), "POST", "/api/recipes/import", Some(&payload)).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "url {url}: {body}");
            assert!(body.contains("not allowed"), "{body}");
        }
    }

    #[tokio::test]
    async fn import_fetch_failure_is_a_client_error() {
        // Nothing listens on this loopback port, but the import router used
        // here allows private targets, so the failure is a fetch failure.
        let app = test_router_import().await;
        let payload = serde_json::json!({ "url": "http://127.0.0.1:9/recipe" }).to_string();
        let (status, body) =
            json_response(app.clone(), "POST", "/api/recipes/import", Some(&payload)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.contains("fetch"), "{body}");
    }

    #[tokio::test]
    async fn import_page_without_recipe_is_unprocessable() {
        let addr = fixture_server("<html><body><p>Just a blog post.</p></body></html>");
        let app = test_router_import().await;
        let payload = serde_json::json!({ "url": format!("http://{addr}/post") }).to_string();
        let (status, body) =
            json_response(app, "POST", "/api/recipes/import", Some(&payload)).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
        assert!(body.contains("could not extract"), "{body}");
    }

    /// A tiny PNG served from a loopback origin (for the image-import tests).
    fn png_server() -> SocketAddr {
        let img = image::DynamicImage::new_rgb8(8, 8);
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        let body: &'static [u8] = Box::leak(png.into_boxed_slice());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buffer = [0u8; 4096];
                use std::io::{Read, Write};
                let _ = stream.read(&mut buffer);
                let response = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: image/png\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.write_all(body);
            }
        });
        addr
    }

    /// `image_url` on the multipart save: the backend downloads the image at
    /// save time and stores full-res + thumbnail like a manual upload.
    #[tokio::test]
    async fn imported_image_url_downloads_at_save() {
        let image_addr = png_server();
        let app = test_router_import().await;
        let boundary = "ImgUrlBnd";
        let image_url = format!("http://{image_addr}/hero.png");
        let payload = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nImported Cake\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"ingredients\"\r\n\r\n[{{\"quantity\":200,\"unit\":\"g\",\"name\":\"flour\",\"prep\":null}}]\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"instructions\"\r\n\r\n[{{\"text\":\"Bake.\"}}]\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"image_url\"\r\n\r\n{image_url}\r\n\
             --{boundary}--\r\n"
        );
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/recipes/photo")
            .header("content-type", format!("multipart/form-data; boundary={boundary}"))
            .body(Body::from(payload))
            .unwrap();
        let resp = app.clone().oneshot(request).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let recipe: Recipe = serde_json::from_slice(&bytes).unwrap();
        let image = recipe.image.expect("imported image must be stored");
        assert!(image.starts_with("/api/images/images/"));
        let thumb = recipe.thumb.expect("imported thumbnail must be stored");
        assert!(thumb.starts_with("/api/images/images/thumbs/"));
    }

    /// A broken image URL never blocks the save — the recipe stores fine
    /// without a photo.
    #[tokio::test]
    async fn imported_image_url_failure_is_soft() {
        let app = test_router_import().await;
        let boundary = "ImgBadBnd";
        let payload = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nImported Soup\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"ingredients\"\r\n\r\n[]\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"instructions\"\r\n\r\n[]\r\n\
             --{boundary}\r\nContent-Disposition: form-data; name=\"image_url\"\r\n\r\nhttp://127.0.0.1:9/missing.png\r\n\
             --{boundary}--\r\n"
        );
        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/recipes/photo")
            .header("content-type", format!("multipart/form-data; boundary={boundary}"))
            .body(Body::from(payload))
            .unwrap();
        let resp = app.clone().oneshot(request).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let recipe: Recipe = serde_json::from_slice(&bytes).unwrap();
        assert!(recipe.image.is_none(), "failed download must not block the save");
    }

    /// The full plan Phase 13 journey: import preview → save → retrieve and
    /// verify sections, ingredients, instructions and metadata — plus a
    /// grocery list that stayed untouched.
    #[tokio::test]
    async fn import_preview_saves_and_retrieves_through_the_normal_api() {
        let addr = fixture_server(fixture_page("how_to_sections.html"));
        let app = test_router_import().await;

        // 1. Import preview (nothing persisted yet).
        let payload = serde_json::json!({ "url": format!("http://{addr}/lasagna") }).to_string();
        let (status, body) =
            json_response(app.clone(), "POST", "/api/recipes/import", Some(&payload)).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let preview: serde_json::Value = serde_json::from_str(&body).unwrap();
        assert_eq!(preview["method"], "json_ld", "{body}");
        assert_eq!(preview["recipe"]["name"], "Lasagna", "{body}");
        assert_eq!(preview["recipe"]["source"], format!("http://{addr}/lasagna"), "{body}");
        let confidence = preview["confidence"].as_f64().unwrap();
        assert!(confidence > 0.5, "confidence {confidence}: {body}");

        // No row was created by the import itself.
        let (_, body) = json_response(app.clone(), "GET", "/api/recipes", None).await;
        assert_eq!(body.trim(), "[]", "import must not persist: {body}");

        // 2. Save the previewed recipe through the normal endpoint.
        let recipe_input = preview["recipe"].clone().to_string();
        let (status, body) =
            json_response(app.clone(), "POST", "/api/recipes", Some(&recipe_input)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let saved: serde_json::Value = serde_json::from_str(&body).unwrap();
        let id = saved["id"].as_i64().unwrap();

        // 3. Retrieve and verify the structured data survived.
        let (status, body) =
            json_response(app.clone(), "GET", &format!("/api/recipes/{id}"), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let detail: RecipeDetail = serde_json::from_str(&body).unwrap();
        assert_eq!(detail.name, "Lasagna");
        assert_eq!(detail.ingredients.len(), 3);
        assert_eq!(detail.ingredients[0].name, "pasta sheets");
        assert_eq!(
            detail.instruction_sections,
            vec!["Meat sauce".to_string(), "Assembly".to_string()]
        );
        assert_eq!(detail.instructions.len(), 5);
        assert_eq!(detail.instructions[0].section.as_deref(), Some("Meat sauce"));
        assert_eq!(detail.instructions[4].section, None);
        assert_eq!(detail.source, format!("http://{addr}/lasagna"));

        // 4. The grocery list is untouched by the import and the save.
        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        assert_eq!(body.trim(), "[]", "grocery list must stay empty: {body}");
    }

    /// A dead recipe URL (noracooks.com/vegan-enchiladas returns exactly this
    /// from its origin) fails with a clean client error, never a 500.
    #[tokio::test]
    async fn import_of_dead_url_is_a_clean_404_error() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let _ = stream.write_all(
                    b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                );
            }
        });
        let app = test_router_import().await;
        let payload = serde_json::json!({ "url": format!("http://{addr}/vegan-enchiladas") }).to_string();
        let (status, body) =
            json_response(app, "POST", "/api/recipes/import", Some(&payload)).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(body.contains("404"), "{body}");
    }

    /// Redirect hops are followed to the final recipe page.
    #[tokio::test]
    async fn import_follows_redirects() {
        // A server that redirects once, then serves the fixture to the
        // *next* connection (two requests, one thread).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let body: &'static str = fixture_page("json_ld_graph.html");
        std::thread::spawn(move || {
            use std::io::{Read, Write};
            let mut served_redirect = false;
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut buffer = [0u8; 4096];
                let _ = stream.read(&mut buffer);
                let response = if served_redirect {
                    format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    )
                } else {
                    served_redirect = true;
                    format!(
                        "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
                    )
                };
                let _ = stream.write_all(response.as_bytes());
            }
        });

        let app = test_router_import().await;
        let payload = serde_json::json!({ "url": format!("http://{addr}/start") }).to_string();
        let (status, response_body) =
            json_response(app, "POST", "/api/recipes/import", Some(&payload)).await;
        assert_eq!(status, StatusCode::OK, "{response_body}");
        let preview: serde_json::Value = serde_json::from_str(&response_body).unwrap();
        assert_eq!(preview["recipe"]["name"], "Apple Cake", "{response_body}");
        assert_eq!(preview["recipe"]["source"], format!("http://{addr}/final"));
}


    #[tokio::test]
    async fn meal_plan_round_trip() {
        let app = test_router(None).await;
        // Seed two recipes.
        let mut ids = Vec::new();
        for name in ["Soup", "Pasta"] {
            let (status, body) = json_response(
                app.clone(),
                "POST",
                "/api/recipes",
                Some(&format!(r#"{{"name":"{name}","ingredients":[],"instructions":[]}}"#)),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
            let recipe: Recipe = serde_json::from_str(&body).unwrap();
            ids.push(recipe.id);
        }

        // Add entries: Soup today-ish, Pasta on another day. Same recipe may
        // repeat on one day (leftovers).
        for (date, recipe_id) in [
            ("2026-10-01", ids[0]),
            ("2026-10-02", ids[1]),
            ("2026-10-01", ids[1]),
            ("2026-10-01", ids[1]),
        ] {
            let (status, body) = json_response(
                app.clone(),
                "POST",
                "/api/meal-plan",
                Some(&format!(r#"{{"date":"{date}","recipe_id":{recipe_id}}}"#)),
            )
            .await;
            assert_eq!(status, StatusCode::CREATED, "{body}");
            let entry: MealPlanEntry = serde_json::from_str(&body).unwrap();
            assert_eq!(entry.date, date);
            assert_eq!(entry.recipe.id, recipe_id);
            assert_eq!(entry.recipe.name, if recipe_id == ids[0] { "Soup" } else { "Pasta" });
        }

        // GET returns everything ordered by date then insertion.
        let (status, body) = json_response(app.clone(), "GET", "/api/meal-plan", None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let entries: Vec<MealPlanEntry> = serde_json::from_str(&body).unwrap();
        assert_eq!(entries.len(), 4);
        let dates: Vec<&str> = entries.iter().map(|e| e.date.as_str()).collect();
        assert_eq!(dates, vec!["2026-10-01", "2026-10-01", "2026-10-01", "2026-10-02"]);

        // DELETE one entry; it is gone, the rest stay.
        let deleted_id = entries[0].id;
        let (status, _) =
            json_response(app.clone(), "DELETE", &format!("/api/meal-plan/{deleted_id}"), None)
                .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, body) = json_response(app.clone(), "GET", "/api/meal-plan", None).await;
        let entries: Vec<MealPlanEntry> = serde_json::from_str(&body).unwrap();
        assert_eq!(entries.len(), 3);

        // Deleting again is 404.
        let (status, _) =
            json_response(app, "DELETE", &format!("/api/meal-plan/{deleted_id}"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn meal_plan_rejects_bad_input() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/recipes",
            Some(r#"{"name":"Soup","ingredients":[],"instructions":[]}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let recipe: Recipe = serde_json::from_str(&body).unwrap();

        // Invalid dates are 422, including real-looking but impossible ones.
        for date in ["not-a-date", "2026-02-30", "2026-13-01", ""] {
            let payload = format!(r#"{{"date":"{date}","recipe_id":{}}}"#, recipe.id);
            let (status, body) =
                json_response(app.clone(), "POST", "/api/meal-plan", Some(&payload)).await;
            assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "date {date:?}: {body}");
        }

        // Unknown recipe is 404.
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/meal-plan",
            Some(r#"{"date":"2026-10-01","recipe_id":9999}"#),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
    }

    #[tokio::test]
    async fn deleting_a_recipe_cascades_its_meal_plan_entries() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/recipes",
            Some(r#"{"name":"Goner","ingredients":[],"instructions":[]}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let recipe: Recipe = serde_json::from_str(&body).unwrap();

        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/meal-plan",
            Some(&format!(r#"{{"date":"2026-10-01","recipe_id":{}}}"#, recipe.id)),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        // Delete the recipe; the plan entry cascades away.
        let (status, _) = json_response(
            app.clone(),
            "DELETE",
            &format!("/api/recipes/{}", recipe.id),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        let (_, body) = json_response(app, "GET", "/api/meal-plan", None).await;
        let entries: Vec<MealPlanEntry> = serde_json::from_str(&body).unwrap();
        assert!(entries.is_empty(), "entries must cascade: {body}");
    }
}
#[cfg(test)]
mod photo_tests {
    use super::tests::*;
    use super::*;
    use axum::body::{to_bytes, Body};
    use tower::ServiceExt;

    /// A tiny valid PNG, generated on the fly.
    fn png_bytes() -> Vec<u8> {
        let img = image::DynamicImage::new_rgb8(8, 8);
        let mut png = Vec::new();
        img.write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png
    }

    #[tokio::test]
    async fn save_recipe_image_writes_fullres_and_thumb() {
        let dir = std::env::temp_dir().join(format!("bouedig-img-{}", uuid::Uuid::new_v4()));
        ensure_data_dirs(&dir).unwrap();
        let png = png_bytes();

        let (image_path, thumb_path) = save_recipe_image(&dir, &png).unwrap();

        assert_eq!(std::fs::read(dir.join(&image_path)).unwrap(), png);
        let thumb = image::open(dir.join(&thumb_path)).unwrap();
        assert!(thumb.width() <= 480);
        assert!(thumb_path.ends_with(".jpg"));
    }

    #[tokio::test]
    async fn multipart_recipe_with_photo_persists_paths() {
        let (app, config) = test_router_with_config(None).await;
        let boundary = "XyZbOuNdArY";
        let png = png_bytes();
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nPancakes\r\n").as_bytes());
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"ingredients\"\r\n\r\n[{{\"quantity\":200,\"unit\":\"g\",\"name\":\"flour\",\"prep\":\"sifted\"}}]\r\n").as_bytes(),
        );
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"instructions\"\r\n\r\n[{{\"text\":\"Mix\"}},{{\"text\":\"Cook\"}}]\r\n").as_bytes(),
        );
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"photo.png\"\r\nContent-Type: image/png\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(&png);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());

        let request = axum::http::Request::builder()
            .method("POST")
            .uri("/api/recipes/photo")
            .header("content-type", format!("multipart/form-data; boundary={boundary}"))
            .body(Body::from(body))
            .unwrap();
        let resp = app.clone().oneshot(request).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let recipe: Recipe = serde_json::from_slice(&bytes).unwrap();
        let image_url = recipe.image.expect("image url must be set");
        let thumb_url = recipe.thumb.expect("thumb url must be set");
        assert!(image_url.starts_with("/api/images/images/"));
        assert!(thumb_url.starts_with("/api/images/images/thumbs/"));

        // Detail round-trip.
        let (status, body) =
            json_response(app.clone(), "GET", &format!("/api/recipes/{}", recipe.id), None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let detail: RecipeDetail = serde_json::from_str(&body).unwrap();
        assert_eq!(detail.ingredients[0].quantity, Some(200.0));
        assert_eq!(detail.ingredients[0].prep.as_deref(), Some("sifted"));
        assert_eq!(detail.instructions, vec![InstructionStep { text: "Mix".into(), section: None }, InstructionStep { text: "Cook".into(), section: None }]);

        // The stored files exist, and DELETE removes them together with the row.
        let image_file = config.data_dir.join(image_url.trim_start_matches("/api/images/"));
        let thumb_file = config.data_dir.join(thumb_url.trim_start_matches("/api/images/"));
        assert!(image_file.exists());
        assert!(thumb_file.exists());
        let (status, _) =
            json_response(app, "DELETE", &format!("/api/recipes/{}", recipe.id), None).await;
        assert_eq!(status, StatusCode::NO_CONTENT);
        assert!(!image_file.exists(), "full-res file must be deleted");
        assert!(!thumb_file.exists(), "thumbnail file must be deleted");
    }

    #[tokio::test]
    async fn grocery_category_defaults_and_persists() {
        let app = test_router(None).await;
        // No category -> default group.
        let (status, body) = json_response(app.clone(), "POST", "/api/grocery", Some(r#"{"name":"Rice"}"#)).await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert!(body.contains(shared::DEFAULT_CATEGORY), "{body}");

        // Explicit category is persisted and listed.
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/grocery",
            Some(r#"{"name":"Wine","category":"Online Alcohol"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        let (status, body) = json_response(app, "GET", "/api/grocery", None).await;
        assert_eq!(status, StatusCode::OK);
        assert!(body.contains("Online Alcohol"), "{body}");
    }
}
