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
use shared::{GroceryItem, GroceryUpdate, NewGroceryItem, NewRecipe, Recipe};
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
        .busy_timeout(std::time::Duration::from_secs(5));
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

fn row_to_recipe(row: &sqlx::sqlite::SqliteRow) -> Recipe {
    use sqlx::Row;
    /// DB stores a relative path under the image dir; expose it as a URL.
    fn url(col: Option<String>) -> Option<String> {
        col.map(|p| format!("/api/images/{p}"))
    }
    Recipe {
        id: row.get::<i64, _>("id"),
        name: row.get::<String, _>("name"),
        ingredients: row.get::<String, _>("ingredients"),
        image: url(row.get::<Option<String>, _>("image_path")),
        thumb: url(row.get::<Option<String>, _>("thumb_path")),
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

/// Insert a recipe (and its parsed ingredients) in one transaction.
async fn insert_recipe(
    db: &SqlitePool,
    name: &str,
    ingredients: &str,
    image: Option<(String, String)>,
) -> Result<Recipe, ApiError> {
    let (image_path, thumb_path) = match image {
        Some((i, t)) => (Some(i), Some(t)),
        None => (None, None),
    };

    let mut tx = db.begin().await?;
    let row = sqlx::query(
        "INSERT INTO recipes (name, ingredients, image_path, thumb_path) \
         VALUES (?, ?, ?, ?) RETURNING id, name, ingredients, image_path, thumb_path",
    )
    .bind(name)
    .bind(ingredients)
    .bind(image_path)
    .bind(thumb_path)
    .fetch_one(&mut *tx)
    .await?;
    let recipe = row_to_recipe(&row);

    // Every parsed ingredient lands on the grocery list.
    for ingredient in shared::parse_ingredients(ingredients) {
        sqlx::query("INSERT INTO grocery_items (name) VALUES (?)")
            .bind(ingredient)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(recipe)
}

async fn create_recipe(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(recipe): Json<NewRecipe>,
) -> Result<(StatusCode, Json<Recipe>), ApiError> {
    let name = recipe.name.trim();
    if name.is_empty() {
        return Err(ApiError(
            (StatusCode::UNPROCESSABLE_ENTITY, "recipe name must not be empty").into_response(),
        ));
    }
    let recipe = insert_recipe(&state.db, name, &recipe.ingredients, None).await?;
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

/// `POST /api/recipes/photo`: multipart form with `name`, `ingredients` and
/// an optional `image` file field.
async fn create_recipe_with_photo(
    axum::extract::State(state): axum::extract::State<AppState>,
    mut multipart: axum::extract::Multipart,
) -> Result<(StatusCode, Json<Recipe>), ApiError> {
    let mut name = String::new();
    let mut ingredients = String::new();
    let mut image: Option<(String, String)> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::client(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?
    {
        match field.name().unwrap_or_default() {
            "name" => name = field.text().await.unwrap_or_default(),
            "ingredients" => ingredients = field.text().await.unwrap_or_default(),
            "image" => {
                let bytes = field
                    .bytes()
                    .await
                    .map_err(|e| ApiError::client(StatusCode::UNPROCESSABLE_ENTITY, e.to_string()))?;
                if !bytes.is_empty() {
                    image = Some(save_recipe_image(&state.data_dir, &bytes).map_err(ApiError::from)?);
                }
            }
            _ => {}
        }
    }

    let name = name.trim();
    if name.is_empty() {
        return Err(ApiError(
            (StatusCode::UNPROCESSABLE_ENTITY, "recipe name must not be empty").into_response(),
        ));
    }
    let recipe = insert_recipe(&state.db, name, &ingredients, image).await?;
    Ok((StatusCode::CREATED, Json(recipe)))
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
        "SELECT id, name, ingredients, image_path, thumb_path FROM recipes ORDER BY id DESC",
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
        let config = test_config(base_path, None);
        ensure_data_dirs(&config.data_dir).unwrap();
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        run_migrations(&pool).await.unwrap();
        build_router(AppState { db: pool, data_dir: config.data_dir.clone() }, &config)
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
    async fn create_recipe_splits_ingredients_into_grocery_list() {
        let app = test_router(None).await;
        let (status, body) = json_response(
            app.clone(),
            "POST",
            "/api/recipes",
            Some(r#"{"name":"Soup","ingredients":"Water, Salt\nPepper"}"#),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert!(body.contains("\"Soup\""), "{body}");

        let (status, body) = json_response(app, "GET", "/api/grocery", None).await;
        assert_eq!(status, StatusCode::OK);
        for ingredient in ["Water", "Salt", "Pepper"] {
            assert!(body.contains(ingredient), "missing {ingredient}: {body}");
        }
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
            Some(r#"{"name":"   ","ingredients":""}"#),
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
        let app = test_router(None).await;
        let boundary = "XyZbOuNdArY";
        let png = png_bytes();
        let mut body: Vec<u8> = Vec::new();
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\nPancakes\r\n").as_bytes());
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"ingredients\"\r\n\r\nFlour, Milk\r\n").as_bytes());
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
        let resp = app.oneshot(request).await.unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        let recipe: Recipe = serde_json::from_slice(&bytes).unwrap();
        let image_url = recipe.image.expect("image url must be set");
        let thumb_url = recipe.thumb.expect("thumb url must be set");
        assert!(image_url.starts_with("/api/images/images/"));
        assert!(thumb_url.starts_with("/api/images/images/thumbs/"));
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
