//! End-to-end test suite for Bouedig.
//!
//! Drives a real headless Firefox (via geckodriver, started by
//! `just test-e2e`) against the production-style web bundle served by the
//! backend: app shell, recipe detail/edit/delete flows, photo upload and the
//! grouped shopping list. A browser-free test covers the API contract.
//!
//! Ports come from the environment (see `.env`):
//!   * `E2E_BACKEND_PORT` - test backend bind port, 0 = pick a free port
//!   * `E2E_GECKO_PORT`   - geckodriver started by `just test-e2e`

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::Context;
use shared::{GroceryItem, Ingredient, NewGroceryItem, Recipe, RecipeDetail, RecipeInput};
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
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::new("warn,backend=info,sqlx=warn"))
        .with_test_writer()
        .try_init();
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

/// The core browser journey: app shell, structured recipe creation via the
/// + FAB, detail view, functional edit, confirm-delete, and a grocery list
/// that only changes through explicit user actions.
#[tokio::test(flavor = "multi_thread")]
async fn recipe_detail_edit_delete_flow() -> anyhow::Result<()> {
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

    // -- 1. The app shell renders: wallpaper, bottom nav, tabs, title. ------
    assert_shell(driver).await?;

    driver
        .find(By::Id("recipe-grid"))
        .await
        .context("recipes grid did not render")?;

    // -- 2. The + FAB opens the structured add form. ------------------------
    driver.find(By::Id("fab-add-recipe")).await?.click().await?;
    wait_for_url_path(driver, "/add").await?;
    let name_input = driver
        .find(By::Id("recipe-name"))
        .await
        .context("the + FAB did not open the add-recipe form")?;
    name_input.send_keys("Pancakes").await?;

    // Structured ingredient rows.
    driver.find(By::Id("ing-qty-0")).await?.send_keys("200").await?;
    driver.find(By::Id("ing-unit-0")).await?.send_keys("g").await?;
    driver.find(By::Id("ing-name-0")).await?.send_keys("Flour").await?;
    driver
        .find(By::Id("add-ingredient"))
        .await?
        .click()
        .await?;
    driver
        .find(By::Id("ing-name-1"))
        .await?
        .send_keys("Milk")
        .await?;

    // Instructions: one step per line.
    driver
        .find(By::Id("recipe-instructions"))
        .await?
        .send_keys("Mix the batter\nCook in a hot pan")
        .await?;

    driver.find(By::Id("recipe-submit")).await?.click().await?;

    // -- 3. The detail view shows the structured recipe. --------------------
    wait_for_url_path_prefix(driver, "/recipe/").await?;
    let detail_url = driver.current_url().await?;
    let id: i64 = detail_url
        .path()
        .rsplit('/')
        .next()
        .and_then(|s| s.parse().ok())
        .context("detail url does not contain a recipe id")?;
    if let Err(err) = assert_detail_view(driver, "Pancakes").await {
        let src = driver.source().await.unwrap_or_default();
        anyhow::bail!("detail view incomplete after creation ({err}); page source:\n{src}");
    }

    // -- 4. Functional edit via the pencil button. --------------------------
    driver.find(By::Id("hdr-edit")).await?.click().await?;
    wait_for_url_path(driver, &format!("/edit/{id}")).await?;
    let name_input = driver
        .find(By::Id("recipe-name"))
        .await
        .context("edit form did not open")?;
    let prefilled = name_input.value().await?.unwrap_or_default();
    anyhow::ensure!(
        prefilled == "Pancakes",
        "edit form should be prefilled, got '{prefilled}'"
    );
    // Clear and rename (send_keys appends, so select-all first).
    name_input.clear().await?;
    name_input.send_keys("Pancakes Deluxe").await?;
    driver.find(By::Id("recipe-submit")).await?.click().await?;
    wait_for_url_path_prefix(driver, "/recipe/").await?;
    if let Err(err) = assert_detail_view(driver, "Pancakes Deluxe").await {
        let src = driver.source().await.unwrap_or_default();
        anyhow::bail!("detail view incomplete after edit ({err}); page source:\n{src}");
    }
    poll_detail(http, base, id, "Pancakes Deluxe").await?;

    // -- 5. The shopping list is untouched by recipe creation. --------------
    driver
        .find(By::LinkText("Shopping"))
        .await?
        .click()
        .await?;
    wait_for_url_path(driver, "/grocery").await?;
    let src = driver.source().await?;
    anyhow::ensure!(
        src.contains("grocery list is empty"),
        "recipe ingredients must not be auto-added; source:\n{src}"
    );

    // Manual add still works and creates a group.
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
    poll_grocery(&http, base, "Bananas", None).await?;

    // -- 6. Confirm-delete removes the recipe. ------------------------------
    driver
        .find(By::LinkText("Recipes"))
        .await?
        .click()
        .await?;
    wait_for_url_path(driver, "/").await?;
    driver
        .find(By::XPath(
            "//div[contains(@class, 'recipe-card-name') and contains(., 'Pancakes Deluxe')]",
        ))
        .await?
        .click()
        .await?;
    wait_for_url_path_prefix(driver, "/recipe/").await?;
    driver
        .find(By::Id("hdr-delete"))
        .await?
        .click()
        .await?;
    driver
        .find(By::Css(".dialog"))
        .await
        .context("delete confirmation dialog did not appear")?;
    driver.find(By::Id("cancel-delete")).await?.click().await?;
    let src = driver.source().await?;
    anyhow::ensure!(
        src.contains("Pancakes Deluxe"),
        "cancel must not delete the recipe"
    );
    driver.find(By::Id("hdr-delete")).await?.click().await?;
    driver.find(By::Id("confirm-delete")).await?.click().await?;
    wait_for_url_path(driver, "/").await?;
    let src = driver.source().await?;
    anyhow::ensure!(
        !src.contains("Pancakes Deluxe"),
        "deleted recipe still visible in the grid"
    );
    // And it is gone from the database too.
    for _ in 0..50 {
        let recipes: Vec<Recipe> = http
            .get(format!("{base}/api/recipes"))
            .send()
            .await?
            .json()
            .await?;
        if recipes.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let recipes: Vec<Recipe> = http
        .get(format!("{base}/api/recipes"))
        .send()
        .await?
        .json()
        .await?;
    anyhow::ensure!(recipes.is_empty(), "deleted recipe still in the database");

    Ok(())
}

/// Photo upload flow: a photo picked in the form is stored as full-res +
/// compressed thumbnail, served by the backend and shown in the detail view.
#[tokio::test(flavor = "multi_thread")]
async fn photo_upload_reaches_detail_and_database() -> anyhow::Result<()> {
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
            .find(By::Id("ing-name-0"))
            .await?
            .send_keys("Cocoa")
            .await?;
        // Set the file input directly (WebDriver standard behaviour).
        driver
            .find(By::Id("recipe-photo"))
            .await?
            .send_keys(png_path.to_str().unwrap())
            .await?;
        driver.find(By::Id("recipe-submit")).await?.click().await?;

        // The detail view renders the full-resolution photo.
        wait_for_url_path_prefix(&driver, "/recipe/").await?;
        if let Err(err) = driver.find(By::Css(".detail-photo img")).await {
            let src = driver.source().await.unwrap_or_default();
            anyhow::bail!("detail photo missing after submission ({err}); page source:\n{src}");
        }

        // Database: both image variants exist and are served.
        let detail_url = driver.current_url().await?;
        let id: i64 = detail_url
            .path()
            .rsplit('/')
            .next()
            .and_then(|s| s.parse().ok())
            .context("detail url does not contain a recipe id")?;
        let detail = poll_detail_full(&http, &base, id).await?;
        let image = detail.image.context("full-res image url missing")?;
        let thumb = detail.thumb.context("thumbnail url missing")?;
        for (label, url) in [("full-res", &image), ("thumb", &thumb)] {
            let resp = http.get(format!("{base}{url}")).send().await?;
            let status = resp.status();
            let content_type = resp.headers().get("content-type").cloned();
            anyhow::ensure!(
                status.is_success()
                    && content_type.is_some_and(|c| c.to_str().unwrap().starts_with("image/")),
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

/// Browser-free API contract test: structured create/update/delete plus a
/// grocery list that only changes through explicit calls.
#[tokio::test(flavor = "multi_thread")]
async fn api_round_trip_recipe_crud() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    // Create (structured).
    let status = http
        .post(format!("{base}/api/recipes"))
        .json(&RecipeInput {
            name: "Stew".into(),
            ingredients: vec![
                Ingredient {
                    quantity: Some(300.0),
                    unit: Some("ml".into()),
                    name: "water".into(),
                    prep: None,
                },
                Ingredient {
                    quantity: None,
                    unit: None,
                    name: "salt".into(),
                    prep: Some("to taste".into()),
                },
            ],
            instructions: vec!["Boil water".into(), "Add salt".into()],
        })
        .send()
        .await?
        .status();
    assert_eq!(status, 201, "POST /api/recipes must return 201");

    // The shopping list must NOT receive recipe ingredients.
    let grocery: Vec<GroceryItem> = http
        .get(format!("{base}/api/grocery"))
        .send()
        .await?
        .json()
        .await?;
    anyhow::ensure!(
        grocery.is_empty(),
        "ingredients leaked into the grocery list: {grocery:?}"
    );

    // Detail round-trip.
    let recipes: Vec<Recipe> = http
        .get(format!("{base}/api/recipes"))
        .send()
        .await?
        .json()
        .await?;
    let id = recipes[0].id;
    let detail = poll_detail_full(&http, &base, id).await?;
    assert_eq!(detail.name, "Stew");
    assert_eq!(detail.ingredients.len(), 2);
    assert_eq!(detail.ingredients[1].prep.as_deref(), Some("to taste"));
    assert_eq!(detail.instructions, vec!["Boil water", "Add salt"]);

    // Update (multipart PUT).
    let boundary = "e2eUpDbNd";
    let payload = concat!(
        "--e2eUpDbNd\r\n",
        "Content-Disposition: form-data; name=\"name\"\r\n\r\n",
        "Better Stew\r\n",
        "--e2eUpDbNd\r\n",
        "Content-Disposition: form-data; name=\"ingredients\"\r\n\r\n",
        "[{\"quantity\":2,\"unit\":\"g\",\"name\":\"carrot\",\"prep\":\"grated\"}]\r\n",
        "--e2eUpDbNd\r\n",
        "Content-Disposition: form-data; name=\"instructions\"\r\n\r\n",
        "[\"chop\"]\r\n",
        "--e2eUpDbNd--\r\n",
    );
    let updated: RecipeDetail = http
        .put(format!("{base}/api/recipes/{id}"))
        .header("content-type", format!("multipart/form-data; boundary={boundary}"))
        .body(payload)
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(updated.name, "Better Stew");
    assert_eq!(updated.ingredients.len(), 1);
    assert_eq!(updated.ingredients[0].name, "carrot");
    assert_eq!(updated.instructions, vec!["chop"]);

    // Delete.
    let status = http
        .delete(format!("{base}/api/recipes/{id}"))
        .send()
        .await?
        .status();
    assert_eq!(status, 204);
    let status = http
        .get(format!("{base}/api/recipes/{id}"))
        .send()
        .await?
        .status();
    assert_eq!(status, 404);

    // Grocery manual add + bought toggle still behave.
    let item: GroceryItem = http
        .post(format!("{base}/api/grocery"))
        .json(&NewGroceryItem { name: "Potatoes".into(), category: None })
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(item.category, shared::DEFAULT_CATEGORY);
    let updated_item: GroceryItem = http
        .patch(format!("{base}/api/grocery/{}", item.id))
        .json(&shared::GroceryUpdate { bought: true })
        .send()
        .await?
        .json()
        .await?;
    assert!(updated_item.bought, "PATCH must flip the bought flag");
    poll_grocery(&http, &base, "Potatoes", Some(true)).await?;

    // Invalid payloads are rejected with 4xx, not 5xx.
    let status = http
        .post(format!("{base}/api/recipes"))
        .json(&RecipeInput { name: "  ".into(), ingredients: vec![], instructions: vec![] })
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
// Shared UI assertions & helpers
// ---------------------------------------------------------------------------

async fn assert_shell(driver: &WebDriver) -> anyhow::Result<()> {
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
    Ok(())
}

/// Assert the currently open detail page shows the given recipe name with
/// its structured ingredients/instructions and the full header bar.
async fn assert_detail_view(driver: &WebDriver, name: &str) -> anyhow::Result<()> {
    let h1 = driver.find(By::Css(".detail-name")).await?;
    let shown = h1.text().await?;
    anyhow::ensure!(shown == name, "detail shows '{shown}', expected '{name}'");
    // Header bar: real actions + placeholders.
    driver.find(By::Id("hdr-back")).await?;
    driver.find(By::Id("hdr-edit")).await?;
    driver.find(By::Id("hdr-delete")).await?;
    driver.find(By::Css(".detail-header .hdr-btn.ph")).await?;
    // Ingredient list with bullet items.
    let list = driver
        .find(By::Id("ingredient-list"))
        .await
        .context("ingredient list missing")?;
    let text = list.text().await?;
    anyhow::ensure!(
        text.contains("200 g Flour"),
        "ingredient line missing from detail: '{text}'"
    );
    anyhow::ensure!(text.contains("Milk"), "ingredient 'Milk' missing: '{text}'");
    // Numbered instructions.
    let steps = driver
        .find(By::Id("instruction-list"))
        .await
        .context("instruction list missing")?;
    let text = steps.text().await?;
    anyhow::ensure!(
        text.contains("Mix the batter") && text.contains("Cook in a hot pan"),
        "instructions missing from detail: '{text}'"
    );
    Ok(())
}

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
    wait_for_url_path_if(driver, &|p| p == path).await
}

/// Wait until the current URL path starts with `prefix` (e.g. /recipe/).
async fn wait_for_url_path_prefix(driver: &WebDriver, prefix: &str) -> anyhow::Result<()> {
    wait_for_url_path_if(driver, &|p| p.starts_with(prefix)).await
}

async fn wait_for_url_path_if(
    driver: &WebDriver,
    matches: &impl Fn(&str) -> bool,
) -> anyhow::Result<()> {
    for _ in 0..50 {
        let url = driver.current_url().await?;
        if matches(url.path()) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let url = driver.current_url().await?;
    anyhow::bail!("navigation never happened (at {})", url)
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

/// Poll `GET /api/recipes/{id}` until the recipe carries the expected name.
async fn poll_detail(
    http: &reqwest::Client,
    base: &str,
    id: i64,
    name: &str,
) -> anyhow::Result<()> {
    for _ in 0..50 {
        if let Ok(detail) = http
            .get(format!("{base}/api/recipes/{id}"))
            .send()
            .await
        {
            if detail.status().is_success() {
                let detail: RecipeDetail = detail.json().await?;
                if detail.name == name {
                    return Ok(());
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("recipe {id} never became '{name}' in the database")
}

/// Like `poll_detail` but returns the detail (with image fields).
async fn poll_detail_full(
    http: &reqwest::Client,
    base: &str,
    id: i64,
) -> anyhow::Result<RecipeDetail> {
    for _ in 0..50 {
        let detail: RecipeDetail = http
            .get(format!("{base}/api/recipes/{id}"))
            .send()
            .await?
            .json()
            .await?;
        return Ok(detail);
    }
    unreachable!()
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
