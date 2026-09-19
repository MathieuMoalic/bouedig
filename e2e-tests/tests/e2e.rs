//! End-to-end test suite for Bouedig.
//!
//! Drives a real headless Firefox (via geckodriver, started by
//! `just test-e2e` on port 4445) against the production-style web bundle
//! served by the backend, verifying UI -> backend -> SQLite -> UI round trips.

use std::time::Duration;

use anyhow::Context;
use shared::GroceryItem;
use thirtyfour::{By, Capabilities, WebDriver};

/// Address the test backend binds to.
const BACKEND_ADDR: &str = "127.0.0.1:3100";
const APP_URL: &str = "http://127.0.0.1:3100/";
/// Geckodriver started by `just test-e2e`.
const WEBDRIVER_URL: &str = "http://127.0.0.1:4445";

#[tokio::test(flavor = "multi_thread")]
async fn add_recipe_shows_up_on_grocery_list() -> anyhow::Result<()> {
    // -- Backend: isolated temp database + the built web bundle. ------------
    let db_dir = tempfile::tempdir()?;
    let config = backend::Config {
        addr: BACKEND_ADDR.parse().unwrap(),
        db_url: format!("sqlite://{}/bouedig-test.db?mode=rwc", db_dir.path().display()),
        base_path: None,
        static_dir: Some(find_web_bundle()?),
    };
    tokio::spawn(backend::run(config));
    wait_for_port(BACKEND_ADDR).await?;
    let http = reqwest::Client::new();

    // -- WebDriver: headless Firefox. ---------------------------------------
    wait_for_port("127.0.0.1:4445").await?;
    let driver = open_headless_firefox().await?;
    let result = run_flow(&driver, &http).await;
    let _ = driver.quit().await;
    result
}

/// The core user journey: add a recipe, see its ingredients on the grocery
/// list, tick one off, add an item manually.
async fn run_flow(driver: &WebDriver, http: &reqwest::Client) -> anyhow::Result<()> {
    driver
        .goto(APP_URL)
        .await
        .context("failed to load the web client")?;

    // Tab 1 is the default route; the wasm app must have booted.
    let name_input = driver
        .wait()
        .for_element(By::Id("recipe-name"))
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
    driver
        .wait()
        .for_element(By::XPath("//*[contains(text(), 'Recipe added!')]"))
        .await
        .context("recipe submission did not succeed")?;

    // -- Tab 2: the grocery list must show the parsed ingredients. ----------
    driver
        .find(By::LinkText("Grocery List"))
        .await?
        .click()
        .await?;
    for ingredient in ["Flour", "Milk", "Eggs"] {
        let li = format!("//li[contains(., '{ingredient}')]");
        driver
            .wait()
            .for_element(By::XPath(&li))
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
    poll_grocery(http, "Milk", Some(true))
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
        .wait()
        .for_element(By::XPath("//li[contains(., 'Bananas')]"))
        .await
        .context("manually added item did not appear in the UI")?;
    poll_grocery(http, "Bananas", None)
        .await
        .context("manually added item missing from the database")?;

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

async fn open_headless_firefox() -> anyhow::Result<WebDriver> {
    let mut caps = Capabilities::new();
    caps.insert("browserName", serde_json::json!("firefox"));
    caps.insert(
        "moz:firefoxOptions",
        serde_json::json!({ "args": ["--headless", "--width=1280", "--height=800"] }),
    );
    let driver = WebDriver::new(WEBDRIVER_URL, caps).await?;
    driver.config().poll_interval = Duration::from_millis(250);
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
    name: &str,
    bought: Option<bool>,
) -> anyhow::Result<()> {
    for _ in 0..50 {
        let items: Vec<GroceryItem> = http
            .get(format!("http://{BACKEND_ADDR}/api/grocery"))
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

/// Locate the web bundle produced by `dx build`. The Justfile exports
/// `BOUEDIG_DIST_DIR`; fall back to the conventional output locations.
fn find_web_bundle() -> anyhow::Result<std::path::PathBuf> {
    if let Ok(dir) = std::env::var("BOUEDIG_DIST_DIR") {
        return Ok(dir.into());
    }
    let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    for candidate in ["../frontend-web/dist", "../dist", "dist"] {
        let path = manifest.join(candidate);
        if path.is_dir() {
            return Ok(path);
        }
    }
    anyhow::bail!(
        "web bundle not found - run `just build-web` (or `just test-e2e`) first"
    )
}
