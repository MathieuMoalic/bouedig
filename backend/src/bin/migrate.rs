//! Standalone migration binary: connects to the database and applies the
//! embedded migrations. Run with `just db-migrate`.

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();
    let db_url =
        std::env::var("BOUEDIG_DB_URL").unwrap_or_else(|_| "sqlite://bouedig.db?mode=rwc".into());
    let pool = backend::open_db(&db_url).await?;
    backend::run_migrations(&pool).await?;
    tracing::info!(%db_url, "migrations applied");
    Ok(())
}
