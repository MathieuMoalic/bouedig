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
use axum::routing::{get, patch, post};
use axum::{Json, Router};
use shared::{GroceryItem, GroceryUpdate, Ingredient, NewGroceryItem, Recipe, RecipeDetail, RecipeInput};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use tower_http::cors::CorsLayer;
use tower_http::services::{ServeDir, ServeFile};

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
        }
    }
}

/// Internal server state threaded through handlers.
#[derive(Clone)]
pub struct AppState {
    db: SqlitePool,
    data_dir: PathBuf,
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
            "/recipes/{id}",
            get(recipe_detail).put(update_recipe).delete(delete_recipe),
        )
        .route(
            "/grocery",
            get(list_grocery).post(add_grocery_item),
        )
        .route("/grocery/{id}", patch(update_grocery_item))
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
    let app = build_router(AppState { db: pool, data_dir: config.data_dir.clone() }, &config);
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
    let app = build_router(AppState { db: pool, data_dir: config.data_dir.clone() }, &config);
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
    }
}

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

/// Decode the instructions JSON column.
fn decode_instructions(json: Option<String>) -> Result<Vec<String>, ApiError> {
    match json {
        None => Ok(Vec::new()),
        Some(json) => serde_json::from_str(&json)
            .map_err(|e| ApiError::internal(anyhow::anyhow!("bad instructions JSON: {e}"))),
    }
}

/// Assemble the full detail view (recipe row + sections + ordered ingredients).
async fn load_recipe_detail(db: &SqlitePool, id: i64) -> Result<Option<RecipeDetail>, ApiError> {
    use sqlx::Row;
    let row = sqlx::query(
        "SELECT id, name, instructions, image_path, thumb_path FROM recipes WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(db)
    .await?;
    let Some(row) = row else {
        return Ok(None);
    };
    let (image, thumb) = recipe_urls(&row);
    let instructions = decode_instructions(row.get("instructions"))?;
    let sections: Vec<String> = sqlx::query(
        "SELECT name FROM recipe_sections WHERE recipe_id = ? ORDER BY position ASC",
    )
    .bind(id)
    .fetch_all(db)
    .await?
    .iter()
    .map(|r| r.get::<String, _>("name"))
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
        image,
        thumb,
    }))
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
    let instructions =
        serde_json::to_string(&input.instructions).context("failed to encode instructions")?;
    // Every section referenced by an ingredient must exist.
    let sections = complete_sections(input);

    let mut tx = db.begin().await?;
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO recipes (name, instructions, image_path, thumb_path) \
         VALUES (?, ?, ?, ?) RETURNING id",
    )
    .bind(input.name.trim())
    .bind(&instructions)
    .bind(image_path.clone())
    .bind(thumb_path.clone())
    .fetch_one(&mut *tx)
    .await?;

    for (position, section) in sections.iter().enumerate() {
        sqlx::query("INSERT INTO recipe_sections (recipe_id, position, name) VALUES (?, ?, ?)")
            .bind(id)
            .bind(position as i64)
            .bind(section)
            .execute(&mut *tx)
            .await?;
    }

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
    tx.commit().await?;

    Ok(Recipe {
        id,
        name: input.name.trim().to_string(),
        image: image_path.map(|p| format!("/api/images/{p}")),
        thumb: thumb_path.map(|p| format!("/api/images/{p}")),
    })
}

/// The input's section list, plus any extra sections referenced by
/// ingredients (appended in first-seen order).
fn complete_sections(input: &RecipeInput) -> Vec<String> {
    let mut sections = input
        .sections
        .iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>();
    for ingredient in &input.ingredients {
        if let Some(section) = ingredient.section.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if !sections.iter().any(|s| s == section) {
                sections.push(section.to_string());
            }
        }
    }
    sections
}

/// Replace the sections and structured ingredients of a recipe.
async fn replace_details(
    tx: &mut sqlx::SqliteConnection,
    recipe_id: i64,
    input: &RecipeInput,
) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM recipe_sections WHERE recipe_id = ?")
        .bind(recipe_id)
        .execute(&mut *tx)
        .await?;
    for (position, section) in complete_sections(input).iter().enumerate() {
        sqlx::query("INSERT INTO recipe_sections (recipe_id, position, name) VALUES (?, ?, ?)")
            .bind(recipe_id)
            .bind(position as i64)
            .bind(section)
            .execute(&mut *tx)
            .await?;
    }

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
    Ok(())
}

fn validate_recipe_input(input: &RecipeInput) -> Result<(), ApiError> {
    if input.name.trim().is_empty() {
        return Err(ApiError(
            (StatusCode::UNPROCESSABLE_ENTITY, "recipe name must not be empty").into_response(),
        ));
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

/// Parse `name`, `ingredients` (JSON array), `instructions` (JSON array) and
/// an optional `image` file field.
async fn parse_recipe_multipart(
    state: &AppState,
    multipart: &mut axum::extract::Multipart,
) -> Result<MultipartRecipe, ApiError> {
    let mut name = String::new();
    let mut sections = String::new();
    let mut ingredients = String::new();
    let mut instructions = String::new();
    let mut image: Option<(String, String)> = None;

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
    let instructions: Vec<String> = if instructions.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(&instructions).map_err(|e| {
            ApiError::client(StatusCode::UNPROCESSABLE_ENTITY, format!("bad instructions JSON: {e}"))
        })?
    };

    Ok(MultipartRecipe {
        input: RecipeInput { name, sections, ingredients, instructions },
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

    let instructions = serde_json::to_string(&input.instructions)
        .context("failed to encode instructions")?;
    let mut tx = state.db.begin().await?;
    let (new_image, new_thumb) = match &image {
        Some((img, thumb)) => {
            delete_image_files(&state.data_dir, old_image.as_deref(), old_thumb.as_deref());
            (Some(img.clone()), Some(thumb.clone()))
        }
        None => (old_image, old_thumb),
    };
    let updated = sqlx::query(
        "UPDATE recipes SET name = ?, instructions = ?, image_path = ?, thumb_path = ? \
         WHERE id = ? RETURNING id",
    )
    .bind(input.name.trim())
    .bind(&instructions)
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
    let rows = sqlx::query("SELECT id, name, bought, category FROM grocery_items ORDER BY id ASC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows.iter().map(row_to_item).collect()))
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
    let category = item
        .category
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty())
        .unwrap_or(shared::DEFAULT_CATEGORY);
    let row = sqlx::query(
        "INSERT INTO grocery_items (name, category) VALUES (?, ?) \
         RETURNING id, name, bought, category",
    )
    .bind(name)
    .bind(category)
    .fetch_one(&state.db)
    .await?;
    Ok((StatusCode::CREATED, Json(row_to_item(&row))))
}

async fn update_grocery_item(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(update): Json<GroceryUpdate>,
) -> Result<Json<GroceryItem>, ApiError> {
    let row = sqlx::query(
        "UPDATE grocery_items SET bought = ? WHERE id = ? \
         RETURNING id, name, bought, category",
    )
        .bind(update.bought as i64)
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .context("no such grocery item")?;
    Ok(Json(row_to_item(&row)))
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
        }
    }

    pub(crate) async fn test_router(base_path: Option<&str>) -> Router {
        test_router_with_config(base_path).await.0
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
            build_router(AppState { db: pool, data_dir: config.data_dir.clone() }, &config),
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
                   "instructions":["Boil water","Add salt"]}"#,
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
        assert_eq!(detail.instructions, vec!["Boil water", "Add salt"]);

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
        let boundary = "SecBNd";
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
            Some(r#"{"name":"Old","ingredients":[{"quantity":1,"unit":null,"name":"onion","prep":null}],"instructions":["a"]}"#),
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
            "[\"step one\",\"step two\"]\r\n",
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

        let (status, body) = json_response(
            app.clone(),
            "PATCH",
            &format!("/api/grocery/{id}"),
            Some(r#"{"bought":true}"#),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.contains("\"bought\":true"), "{body}");

        let (_, body) = json_response(app, "GET", "/api/grocery", None).await;
        assert!(body.contains("\"name\":\"Rice\"") && body.contains("\"bought\":true"), "{body}");
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
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"instructions\"\r\n\r\n[\"Mix\",\"Cook\"]\r\n").as_bytes(),
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
        assert_eq!(detail.instructions, vec!["Mix", "Cook"]);

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
