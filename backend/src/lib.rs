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
        }
    }
}

/// Internal server state threaded through handlers.
#[derive(Clone)]
pub struct AppState {
    db: SqlitePool,
}

/// Open (creating if needed) the SQLite pool for `db_url`.
pub async fn open_db(db_url: &str) -> anyhow::Result<SqlitePool> {
    let options = SqliteConnectOptions::from_str(db_url)
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
        .route("/recipes", post(create_recipe).join(get(list_recipes)))
        .route("/grocery", get(list_grocery).join(post(add_grocery_item)))
        .route("/grocery/{id}", patch(update_grocery_item))
        .layer(CorsLayer::very_permissive())
        .with_state(state);

    let mut app = Router::new().merge(api);
    if let Some(static_dir) = &config.static_dir {
        let index = static_dir.join("index.html");
        let serve = ServeDir::new(static_dir)
            .append_index_html_on_directories(true)
            .fallback(ServeFile::new(index));
        app = app.fallback_service(serve);
    }

    // When mounted behind a reverse proxy that does not strip the prefix
    // (e.g. /bouedig/api/... hits us directly), nest everything under it.
    if let Some(base_path) = config.base_path.as_deref().filter(|p| p != "/") {
        Router::new().nest(base_path, app)
    } else {
        app
    }
}

/// Bind, migrate and serve. Resolves when the server shuts down.
pub async fn run(config: Config) -> anyhow::Result<()> {
    let pool = open_db(&config.db_url).await?;
    run_migrations(&pool).await?;
    let app = build_router(AppState { db: pool }, &config);
    tracing::info!("listening on http://{}", config.addr);
    let listener = tokio::net::TcpListener::bind(config.addr)
        .await
        .context("failed to bind address")?;
    axum::serve(listener, app).await.context("server error")
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Uniform error response for handlers.
struct ApiError(anyhow::Error);

impl IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        tracing::error!("api error: {:#}", self.0);
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(serde_json::json!({ "error": self.0.to_string() })),
        )
            .into_response()
    }
}

impl<E: Into<anyhow::Error>> From<E> for ApiError {
    fn from(err: E) -> Self {
        Self(err.into())
    }
}

fn row_to_recipe(row: &sqlx::sqlite::SqliteRow) -> Recipe {
    use sqlx::Row;
    Recipe {
        id: row.get::<i64, _>("id"),
        name: row.get::<String, _>("name"),
        ingredients: row.get::<String, _>("ingredients"),
    }
}

fn row_to_item(row: &sqlx::sqlite::SqliteRow) -> GroceryItem {
    use sqlx::Row;
    GroceryItem {
        id: row.get::<i64, _>("id"),
        name: row.get::<String, _>("name"),
        bought: row.get::<i64, _>("bought") != 0,
    }
}

async fn create_recipe(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(recipe): Json<NewRecipe>,
) -> Result<(StatusCode, Json<Recipe>), ApiError> {
    let name = recipe.name.trim();
    anyhow::ensure!(!name.is_empty(), "recipe name must not be empty");

    let mut tx = state.db.begin().await?;
    let row = sqlx::query("INSERT INTO recipes (name, ingredients) VALUES (?, ?) RETURNING id, name, ingredients")
        .bind(name)
        .bind(&recipe.ingredients)
        .fetch_one(&mut *tx)
        .await?;
    let recipe = row_to_recipe(&row);

    // Every parsed ingredient lands on the grocery list.
    for ingredient in shared::parse_ingredients(&recipe.ingredients) {
        sqlx::query("INSERT INTO grocery_items (name) VALUES (?)")
            .bind(ingredient)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;

    Ok((StatusCode::CREATED, Json(recipe)))
}

async fn list_recipes(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<Vec<Recipe>>, ApiError> {
    let rows = sqlx::query("SELECT id, name, ingredients FROM recipes ORDER BY id DESC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows.iter().map(row_to_recipe).collect()))
}

async fn list_grocery(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> Result<Json<Vec<GroceryItem>>, ApiError> {
    let rows = sqlx::query("SELECT id, name, bought FROM grocery_items ORDER BY id ASC")
        .fetch_all(&state.db)
        .await?;
    Ok(Json(rows.iter().map(row_to_item).collect()))
}

async fn add_grocery_item(
    axum::extract::State(state): axum::extract::State<AppState>,
    Json(item): Json<NewGroceryItem>,
) -> Result<(StatusCode, Json<GroceryItem>), ApiError> {
    let name = item.name.trim();
    anyhow::ensure!(!name.is_empty(), "item name must not be empty");
    let row = sqlx::query("INSERT INTO grocery_items (name) VALUES (?) RETURNING id, name, bought")
        .bind(name)
        .fetch_one(&state.db)
        .await?;
    Ok((StatusCode::CREATED, Json(row_to_item(&row))))
}

async fn update_grocery_item(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Path(id): axum::extract::Path<i64>,
    Json(update): Json<GroceryUpdate>,
) -> Result<Json<GroceryItem>, ApiError> {
    let row = sqlx::query("UPDATE grocery_items SET bought = ? WHERE id = ? RETURNING id, name, bought")
        .bind(update.bought as i64)
        .bind(id)
        .fetch_optional(&state.db)
        .await?
        .context("no such grocery item")?;
    Ok(Json(row_to_item(&row)))
}
