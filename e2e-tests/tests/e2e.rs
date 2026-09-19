//! End-to-end test suite for Bouedig.
//!
//! Drives a real headless Firefox (via geckodriver, started by
//! `just test-e2e`) against the production-style web bundle served by the
//! backend, verifying UI -> backend -> SQLite -> UI round trips. A second,
//! browser-free test exercises the same API contract directly.
//!
//! Ports come from the environment (see `.env`):
//!   * `E2E_BACKEND_PORT` - test backend bind port, 0 = pick a free port
//!   * `E2E_GECKO_PORT`   - geckodriver started by `just test-e2e`

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Context;
use shared::{GroceryItem, NewGroceryItem, NewRecipe, Recipe};
use thirtyfour::{By, Capabilities, WebDriver};

fn env_port(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(default)
}

/// Start an isolated backend (temp SQLite + built web bundle) and return its
/// bound address.
async fn spawn_test_backend() -> anyhow::Result<SocketAddr> {
    let db_dir = tempfile::tempdir()?;
    let config = backend::Config {
        addr: SocketAddr::from(([127, 0, 0, 1], env_port("E2E_BACKEND_PORT", 0))),
        db_url: format!("sqlite://{}/bouedig-test.db?mode=rwc", db_dir.path().display()),
        base_path: None,
        static_dir: Some(find_web_bundle()?),
    };
    let addr = backend::spawn_server(config).await?;
    // Keep the tempdir alive for the rest of the process.
    std::mem::forget(db_dir);
    Ok(addr)
}

/// Geckodriver host:port, started by `just test-e2e`.
fn webdriver_addr() -> String {
    format!("127.0.0.1:{}", env_port("E2E_GECKO_PORT", 4445))
}

fn webdriver_url() -> String {
    format!("http://{}", webdriver_addr())
}

/// The core browser journey: clicking the buttons must add the recipe and
/// its ingredients, persisting them to the database.
#[tokio::test(flavor = "multi_thread")]
async fn add_recipe_shows_up_on_grocery_list() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let http = reqwest::Client::new();

    wait_for_port(&addr.to_string()).await?;
    wait_for_port(&webdriver_addr()).await?;
    let driver = open_headless_firefox().await?;
    let result = run_flow(&driver, &http, &format!("http://{addr}/"), &format!("http://{addr}")).await;
    let _ = driver.quit().await;
    result
}

async fn run_flow(
    driver: &WebDriver,
    http: &reqwest::Client,
    app_url: &str,
    base_url: &str,
) -> anyhow::Result<()> {
    driver
        .goto(app_url)
        .await
        .context("failed to load the web client")?;

    // Tab 1 is the default route; the wasm app must have booted.
    let name_input = driver
        .find(By::Id("recipe-name"))
        .await
        .context("web client did not render the Add Recipe tab")?;
    name_input
        .send_keys("Pancakes")
        .await
        .context("failed to type recipe name")?;
    driver
        .find(By::Id("recipe-ingredients"))
        .await?
        .send_keys("Flour, Milk\nEggs")
        .await?;
    driver.find(By::Id("recipe-submit")).await?.click().await?;

    // The POST must succeed before we switch tabs.
    if let Err(err) = driver
        .find(By::XPath("//*[contains(text(), 'Recipe added!')]"))
        .await
    {
        let src = driver.source().await.unwrap_or_default();
        anyhow::bail!("recipe submission feedback missing ({err}); page source:\n{src}");
    }

    // The recipe click must have persisted the recipe itself to the DB.
    poll_recipes(&http, base_url, "Pancakes", "Flour, Milk\nEggs")
        .await
        .context("recipe was not persisted to the database")?;

    // -- Tab 2: the grocery list must show the parsed ingredients. ----------
    driver
        .find(By::LinkText("Grocery List"))
        .await?
        .click()
        .await?;
    for ingredient in ["Flour", "Milk", "Eggs"] {
        let li = format!("//li[contains(., '{ingredient}')]");
        driver
            .find(By::XPath(&li))
            .await
            .with_context(|| format!("ingredient '{ingredient}' missing from grocery list"))?;
    }

    // Tick "Milk" off and verify the checkbox state was persisted to SQLite.
    let milk = driver
        .find(By::XPath(
            "//li[contains(., 'Milk')]/input[@type='checkbox']",
        ))
        .await?;
    milk.click().await?;
    poll_grocery(&http, base_url, "Milk", Some(true))
        .await
        .context("'bought' state was not persisted to the database")?;

    // Manually add an item and verify it too reaches the database.
    driver
        .find(By::Id("grocery-input"))
        .await?
        .send_keys("Bananas")
        .await?;
    driver.find(By::Id("grocery-add")).await?.click().await?;
    driver
        .find(By::XPath("//li[contains(., 'Bananas')]"))
        .await
        .context("manually added item did not appear in the UI")?;
    poll_grocery(&http, base_url, "Bananas", None)
        .await
        .context("manually added item missing from the database")?;

    Ok(())
}

/// Browser-free API contract test: POSTing a recipe must split its
/// ingredients onto the grocery list and persist everything.
#[tokio::test(flavor = "multi_thread")]
async fn api_round_trip_recipe_to_grocery() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    // Clicking-equivalent: POST the recipe.
    let status = http
        .post(format!("{base}/api/recipes"))
        .json(&NewRecipe {
            name: "Stew".into(),
            ingredients: "Carrots, Onions".into(),
        })
        .send()
        .await?
        .status();
    assert_eq!(status, 201, "POST /api/recipes must return 201");
    poll_recipes(&http, &base, "Stew", "Carrots, Onions").await?;
    poll_grocery(&http, &base, "Carrots", None).await?;
    poll_grocery(&http, &base, "Onions", None).await?;

    // Manual grocery item + bought toggle.
    let item: GroceryItem = http
        .post(format!("{base}/api/grocery"))
        .json(&NewGroceryItem { name: "Potatoes".into() })
        .send()
        .await?
        .json()
        .await?;
    let updated: GroceryItem = http
        .patch(format!("{base}/api/grocery/{}", item.id))
        .json(&shared::GroceryUpdate { bought: true })
        .send()
        .await?
        .json()
        .await?;
    assert!(updated.bought, "PATCH must flip the bought flag");
    poll_grocery(&http, &base, "Potatoes", Some(true)).await?;

    // Invalid payloads are rejected with 4xx, not 5xx.
    let status = http
        .post(format!("{base}/api/recipes"))
        .json(&NewRecipe { name: "  ".into(), ingredients: String::new() })
        .send()
        .await?
        .status();
    assert!(
        status.is_client_error(),
        "empty recipe name must be rejected, got {status}"
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn open_headless_firefox() -> anyhow::Result<WebDriver> {
    let mut caps = Capabilities::new();
    caps.set("browserName", serde_json::json!("firefox"))?;
    caps.set(
        "moz:firefoxOptions",
        serde_json::json!({ "args": ["--headless", "--width=1280", "--height=800"] }),
    )?;
    let driver = WebDriver::new(webdriver_url(), caps).await?;
    // Make every subsequent `find` poll for up to 30s (wasm boot, fetches…).
    driver
        .set_implicit_wait_timeout(Duration::from_secs(30))
        .await?;
    Ok(driver)
}

/// Poll TCP until something is listening (backend or geckodriver).
async fn wait_for_port(addr: &str) -> anyhow::Result<()> {
    for _ in 0..150 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("nothing listening on {addr}")
}

/// Poll `GET /api/grocery` (i.e. the database) until `name` exists and, when
/// given, has the expected `bought` state.
async fn poll_grocery(
    http: &reqwest::Client,
    base: &str,
    name: &str,
    bought: Option<bool>,
) -> anyhow::Result<()> {
    for _ in 0..50 {
        let items: Vec<GroceryItem> = http
            .get(format!("{base}/api/grocery"))
            .send()
            .await?
            .json()
            .await?;
        let found = items
            .iter()
            .find(|i| i.name == name)
            .is_some_and(|i| bought.is_none_or(|b| i.bought == b));
        if found {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("grocery item '{name}' (bought={bought:?}) never appeared in the database")
}

/// Poll `GET /api/recipes` until the given recipe (name + ingredients) has
/// been persisted.
async fn poll_recipes(
    http: &reqwest::Client,
    base: &str,
    name: &str,
    ingredients: &str,
) -> anyhow::Result<()> {
    for _ in 0..50 {
        let recipes: Vec<Recipe> = http
            .get(format!("{base}/api/recipes"))
            .send()
            .await?
            .json()
            .await?;
        if recipes
            .iter()
            .any(|r| r.name == name && r.ingredients == ingredients)
        {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("recipe '{name}' ('{ingredients}') never appeared in the database")
}

/// Locate the web bundle produced by `dx build`. The Justfile exports
/// `BOUEDIG_DIST_DIR`; fall back to the conventional output locations.
fn find_web_bundle() -> anyhow::Result<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("BOUEDIG_DIST_DIR") {
        return Ok(dir.into());
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for candidate in [
        "../target/dx/frontend-web/debug/web/public",
        "../target/dx/frontend-web/release/web/public",
    ] {
        let path = manifest.join(candidate);
        if path.is_dir() {
            return Ok(path);
        }
    }
    anyhow::bail!(
        "web bundle not found - run `just build-web` (or `just test-e2e`) first"
    )
}
