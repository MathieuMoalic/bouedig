//! End-to-end test suite for Bouedig.
//!
//! Drives a real headless Firefox (via geckodriver, started by
//! `just test-e2e`) against the production-style web bundle served by the
//! backend, verifying UI -> backend -> SQLite -> UI round trips, including
//! the new photo-card app shell and grouped grocery list. A second,
//! browser-free test exercises the API contract directly.
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

/// Start an isolated backend (temp SQLite, temp image dir, built web bundle)
/// and return its bound address.
async fn spawn_test_backend() -> anyhow::Result<SocketAddr> {
    let db_dir = tempfile::tempdir()?;
    let data_dir = tempfile::tempdir()?;
    let config = backend::Config {
        addr: SocketAddr::from(([127, 0, 0, 1], env_port("E2E_BACKEND_PORT", 0))),
        db_url: format!("sqlite://{}/bouedig-test.db?mode=rwc", db_dir.path().display()),
        base_path: None,
        static_dir: Some(find_web_bundle()?),
        data_dir: data_dir.path().to_path_buf(),
    };
    let addr = backend::spawn_server(config).await?;
    // Keep the tempdirs alive for the rest of the process.
    std::mem::forget(db_dir);
    std::mem::forget(data_dir);
    Ok(addr)
}

/// Geckodriver host:port, started by `just test-e2e`.
fn webdriver_addr() -> String {
    format!("127.0.0.1:{}", env_port("E2E_GECKO_PORT", 4445))
}

fn webdriver_url() -> String {
    format!("http://{}", webdriver_addr())
}

/// The core browser journey: the new app shell renders, the + FAB opens the
/// add form, submitting persists the recipe (visible as a card in the grid),
/// and the grouped shopping list reflects everything in the database.
#[tokio::test(flavor = "multi_thread")]
async fn add_recipe_shows_up_on_grocery_list() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    wait_for_port(&addr.to_string()).await?;
    wait_for_port(&webdriver_addr()).await?;
    let driver = open_headless_firefox().await?;
    let result = run_flow(&driver, &http, &base).await;
    let _ = driver.quit().await;
    result
}

async fn run_flow(driver: &WebDriver, http: &reqwest::Client, base: &str) -> anyhow::Result<()> {
    driver
        .goto(format!("{base}/"))
        .await
        .context("failed to load the web client")?;

    // -- 1. The app shell renders: wallpaper, bottom nav, all four tabs. ----
    driver
        .find(By::Id("bottom-nav"))
        .await
        .context("app shell (bottom navigation) did not render")?;
    driver
        .find(By::Css(".wallpaper"))
        .await
        .context("app shell (wallpaper) did not render")?;
    for tab in ["Recipes", "Meal plan", "Shopping", "Settings"] {
        driver
            .find(By::LinkText(tab))
            .await
            .with_context(|| format!("nav tab '{tab}' missing"))?;
    }

    // The browser tab is named and shows the favicon.
    let title = driver.title().await?;
    anyhow::ensure!(
        title == "Bouedig",
        "tab title should be 'Bouedig', got '{title}'"
    );
    driver
        .find(By::Css("link[rel='icon']"))
        .await
        .context("favicon <link> missing from the document head")?;

    // -- 2. Tab clicks change the active view/route. ------------------------
    driver.find(By::LinkText("Meal plan")).await?.click().await?;
    wait_for_url_path(driver, "/meal-plan").await?;
    driver
        .find(By::Css(".placeholder-card"))
        .await
        .context("meal plan tab did not switch the view")?;

    driver
        .find(By::LinkText("Recipes"))
        .await?
        .click()
        .await?;
    wait_for_url_path(driver, "/").await?;
    driver
        .find(By::Id("recipe-grid"))
        .await
        .context("recipes grid did not render")?;

    // -- 3. The + FAB opens the add-recipe form. ----------------------------
    driver.find(By::Id("fab-add-recipe")).await?.click().await?;
    driver
        .find(By::Id("recipe-name"))
        .await
        .context("the + FAB did not open the add-recipe form")?;
    wait_for_url_path(driver, "/add").await?;

    driver
        .find(By::Id("recipe-name"))
        .await?
        .send_keys("Pancakes")
        .await
        .context("failed to type recipe name")?;
    driver
        .find(By::Id("recipe-ingredients"))
        .await?
        .send_keys("Flour, Milk\nEggs")
        .await?;
    driver.find(By::Id("recipe-submit")).await?.click().await?;

    // Submitting routes back to the grid, where the new card appears.
    wait_for_url_path(driver, "/").await?;
    if let Err(err) = driver
        .find(By::XPath(
            "//div[contains(@class, 'recipe-card-name') and contains(., 'Pancakes')]",
        ))
        .await
    {
        let src = driver.source().await.unwrap_or_default();
        anyhow::bail!("recipe card missing after submission ({err}); page source:\n{src}");
    }

    // The recipe click must have persisted the recipe itself to the DB.
    poll_recipes(&http, base, "Pancakes", "Flour, Milk\nEggs")
        .await
        .context("recipe was not persisted to the database")?;

    // -- 4. Shopping tab: grouped list shows the parsed ingredients. --------
    driver
        .find(By::LinkText("Shopping"))
        .await?
        .click()
        .await?;
    wait_for_url_path(driver, "/grocery").await?;
    driver
        .find(By::XPath(
            "//button[contains(@class, 'grocery-group') and contains(., 'Groceries')]",
        ))
        .await
        .context("default grocery group header did not render")?;
    for ingredient in ["Flour", "Milk", "Eggs"] {
        let li = format!("//li[contains(., '{ingredient}')]");
        driver
            .find(By::XPath(&li))
            .await
            .with_context(|| format!("ingredient '{ingredient}' missing from grocery list"))?;
    }

    // Group collapse toggles the items away and back.
    driver
        .find(By::XPath(
            "//button[contains(@class, 'grocery-group') and contains(., 'Groceries')]",
        ))
        .await?
        .click()
        .await?;
    let src = driver.source().await?;
    anyhow::ensure!(
        !src.contains("Flour"),
        "grocery group did not collapse (items still in the DOM)"
    );
    driver
        .find(By::XPath(
            "//button[contains(@class, 'grocery-group') and contains(., 'Groceries')]",
        ))
        .await?
        .click()
        .await?;
    driver
        .find(By::XPath("//li[contains(., 'Flour')]"))
        .await
        .context("grocery group did not re-expand")?;

    // Tick "Milk" off and verify the checkbox state was persisted to SQLite.
    let milk = driver
        .find(By::XPath(
            "//li[contains(., 'Milk')]/input[@type='checkbox']",
        ))
        .await?;
    milk.click().await?;
    poll_grocery(&http, base, "Milk", Some(true))
        .await
        .context("'bought' state was not persisted to the database")?;

    // Manually add an item and verify it too reaches the database.
    driver
        .find(By::Id("grocery-input"))
        .await?
        .send_keys("Bananas")
        .await?;
    driver
        .find(By::Id("grocery-category"))
        .await?
        .send_keys("Fresh")
        .await?;
    driver.find(By::Id("grocery-add")).await?.click().await?;
    driver
        .find(By::XPath("//li[contains(., 'Bananas')]"))
        .await
        .context("manually added item did not appear in the UI")?;
    poll_grocery(&http, base, "Bananas", None)
        .await
        .context("manually added item missing from the database")?;

    Ok(())
}

/// Photo upload flow: a photo picked in the form is stored as full-res +
/// compressed thumbnail and served by the backend; the grid card shows it.
#[tokio::test(flavor = "multi_thread")]
async fn photo_upload_reaches_grid_and_database() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    // A small valid PNG on disk, selectable by the browser.
    let png_path = std::env::temp_dir().join("bouedig-e2e-photo.png");
    std::fs::write(&png_path, tiny_png()).context("failed to write test photo")?;

    wait_for_port(&addr.to_string()).await?;
    wait_for_port(&webdriver_addr()).await?;
    let driver = open_headless_firefox().await?;
    let result = (|| async {
        driver.goto(format!("{base}/")).await?;
        driver.find(By::Id("fab-add-recipe")).await?.click().await?;
        driver
            .find(By::Id("recipe-name"))
            .await?
            .send_keys("Photo Cake")
            .await?;
        driver
            .find(By::Id("recipe-ingredients"))
            .await?
            .send_keys("Cocoa, Sugar")
            .await?;
        // Set the file input directly (WebDriver standard behaviour).
        driver
            .find(By::Id("recipe-photo"))
            .await?
            .send_keys(png_path.to_str().unwrap())
            .await?;
        driver.find(By::Id("recipe-submit")).await?.click().await?;

        // The grid card renders the compressed thumbnail.
        wait_for_url_path(&driver, "/").await?;
        if let Err(err) = driver.find(By::Css("#recipe-grid .recipe-card img")).await {
            let src = driver.source().await.unwrap_or_default();
            anyhow::bail!("recipe card thumbnail missing after submission ({err}); page source:\n{src}");
        }

        // Database: both image variants exist and the thumbnail is served.
        let recipe = poll_recipes_full(&http, &base, "Photo Cake")
            .await
            .context("photo recipe missing from the database")?;
        let image = recipe.image.context("full-res image url missing")?;
        let thumb = recipe.thumb.context("thumbnail url missing")?;
        for (label, url) in [("full-res", &image), ("thumb", &thumb)] {
            let resp = http.get(format!("{base}{url}")).send().await?;
            let status = resp.status();
            let content_type = resp.headers().get("content-type").cloned();
            anyhow::ensure!(
                status.is_success() && content_type.is_some_and(|c| c.to_str().unwrap().starts_with("image/")),
                "{label} image {url} not served correctly ({status})"
            );
        }
        anyhow::ensure!(image != thumb, "thumb must be a separate file");
        Ok(())
    })()
    .await;
    let _ = driver.quit().await;
    result
}

/// Browser-free API contract test: POSTing a recipe must split its
/// ingredients onto the grouped grocery list and persist everything.
#[tokio::test(flavor = "multi_thread")]
async fn api_round_trip_recipe_to_grocery() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

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

    // Manual grocery item + bought toggle, with and without a category.
    let item: GroceryItem = http
        .post(format!("{base}/api/grocery"))
        .json(&NewGroceryItem { name: "Potatoes".into(), category: None })
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(item.category, shared::DEFAULT_CATEGORY);
    let updated: GroceryItem = http
        .patch(format!("{base}/api/grocery/{}", item.id))
        .json(&shared::GroceryUpdate { bought: true })
        .send()
        .await?
        .json()
        .await?;
    assert!(updated.bought, "PATCH must flip the bought flag");
    poll_grocery(&http, &base, "Potatoes", Some(true)).await?;

    let item: GroceryItem = http
        .post(format!("{base}/api/grocery"))
        .json(&NewGroceryItem {
            name: "Wine".into(),
            category: Some("Online Alcohol".into()),
        })
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(item.category, "Online Alcohol");
    poll_grocery(&http, &base, "Wine", None).await?;

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

/// Wait until the current URL path equals `path` (router navigation).
async fn wait_for_url_path(driver: &WebDriver, path: &str) -> anyhow::Result<()> {
    for _ in 0..50 {
        let url = driver.current_url().await?;
        if url.path() == path {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let url = driver.current_url().await?;
    anyhow::bail!("navigation to '{path}' never happened (at {})", url)
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

/// Like `poll_recipes` but returns the matched recipe (with image fields).
async fn poll_recipes_full(
    http: &reqwest::Client,
    base: &str,
    name: &str,
) -> anyhow::Result<Recipe> {
    for _ in 0..50 {
        let recipes: Vec<Recipe> = http
            .get(format!("{base}/api/recipes"))
            .send()
            .await?
            .json()
            .await?;
        if let Some(recipe) = recipes.iter().find(|r| r.name == name) {
            return Ok(recipe.clone());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("recipe '{name}' never appeared in the database")
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

/// A 1x1 red PNG, hardcoded (no image crate needed in the test suite).
fn tiny_png() -> Vec<u8> {
    const B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
    fn b64_decode(input: &str) -> Vec<u8> {
        const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = Vec::new();
        let mut buf = 0u32;
        let mut bits = 0u32;
        for c in input.bytes() {
            if c == b'=' {
                break;
            }
            let v = TABLE.iter().position(|t| *t == c).unwrap() as u32;
            buf = (buf << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
        out
    }
    b64_decode(B64)
}
