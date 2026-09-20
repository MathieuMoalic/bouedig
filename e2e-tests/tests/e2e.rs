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
use shared::{GroceryItem, Ingredient, InstructionStep, NewGroceryItem, Recipe, RecipeDetail, RecipeInput};
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

    // Create a section via its modal.
    click_scrolled(&driver, "add-section").await?;
    driver
        .find(By::Id("modal-section-name"))
        .await?
        .send_keys("Batter")
        .await?;
    click_scrolled(&driver, "modal-save").await?;
    driver
        .find(By::Id("ingredient-rows"))
        .await?
        .text()
        .await?
        .contains("Batter")
        .then_some(())
        .context("section 'Batter' did not appear after save")?;

    // Add ingredients through the ingredient modal (defaults to the last
    // section = Batter).
    add_ingredient_modal(driver, "200", "g", "Flour", "").await?;
    add_ingredient_modal(driver, "", "", "Milk", "").await?;

    // An invalid quantity must show an inline error and block the save.
    open_ingredient_modal(driver).await?;
    driver.find(By::Id("modal-qty")).await?.send_keys("0").await?;
    driver
        .find(By::Css("#modal-qty.invalid"))
        .await
        .context("invalid quantity does not turn the field red")?;
    driver
        .find(By::Css(".modal-error"))
        .await
        .context("quantity error message not shown while invalid")?;
    click_scrolled(driver, "modal-save").await?;
    driver
        .find(By::Id("modal-name"))
        .await
        .context("save must be blocked while the quantity is invalid")?;
    click_scrolled(driver, "modal-cancel").await?;

    // Instructions: one step per modal, then an instruction section and a
    // step inside it (the section select defaults to the last section).
    add_step_modal(driver, "Mix the batter", None).await?;
    add_step_section_modal(driver, "Cooking").await?;
    add_step_modal(driver, "Cook in a hot pan", Some("Cooking")).await?;

    // Meta fields between instructions and the photo.
    driver
        .find(By::Id("recipe-yield"))
        .await?
        .send_keys("4 pancakes")
        .await?;
    driver
        .find(By::Id("recipe-source"))
        .await?
        .send_keys("Grandma")
        .await?;
    driver
        .find(By::Id("recipe-notes"))
        .await?
        .send_keys("Best eaten warm")
        .await?;

    click_scrolled(&driver, "recipe-submit").await?;

    // -- 3. The detail view shows the structured recipe. --------------------
    wait_for_url_path_prefix(driver, "/recipe/").await?;
    let detail_url = driver.current_url().await?;
    let id: i64 = detail_url
        .path()
        .rsplit('/')
        .next()
        .and_then(|s| s.parse().ok())
        .context("detail url does not contain a recipe id")?;
    if let Err(err) = assert_detail_view(
        driver,
        "Pancakes",
        &["Mix the batter", "Cook in a hot pan"],
    )
    .await
    {
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
    // Tap an ingredient row -> prefilled edit modal -> change its prep.
    let rows_before = ingredient_row_texts(driver).await?;
    anyhow::ensure!(
        rows_before.iter().any(|t| t.contains("200 g Flour")),
        "prefilled ingredient list wrong: {rows_before:?}"
    );
    let flour_row = driver
        .find(By::XPath(
            "//div[contains(@class, 'ingredient-row') and contains(., 'Flour')]",
        ))
        .await?;
    flour_row.click().await?;
    let name_field = driver
        .find(By::Id("modal-name"))
        .await
        .context("ingredient edit modal did not open")?;
    let prefilled_name = name_field.value().await?.unwrap_or_default();
    anyhow::ensure!(
        prefilled_name == "Flour",
        "modal should be prefilled with 'Flour', got '{prefilled_name}'"
    );
    let qty_prefill = driver
        .find(By::Id("modal-qty"))
        .await?
        .value()
        .await?
        .unwrap_or_default();
    anyhow::ensure!(
        qty_prefill == "200",
        "modal qty should be prefilled with '200', got '{qty_prefill}'"
    );
    driver
        .find(By::Id("modal-prep"))
        .await?
        .send_keys("sifted")
        .await?;
    click_scrolled(&driver, "modal-save").await?;
    let rows_after = ingredient_row_texts(driver).await?;
    anyhow::ensure!(
        rows_after
            .iter()
            .any(|t| t.contains("200 g Flour") && t.contains("sifted")),
        "edited prep missing after modal save: {rows_after:?}"
    );

    // Delete the second step through its × button, then save.
    let steps_before = step_texts(driver).await?;
    anyhow::ensure!(steps_before.len() == 2, "expected 2 steps, got {steps_before:?}");
    driver
        .find(By::Css("#step-rows .step-row .row-actions .row-btn.danger"))
        .await?
        .click()
        .await?;
    let steps_after = step_texts(driver).await?;
    anyhow::ensure!(
        steps_after.len() == 1,
        "step was not deleted, still {steps_after:?}"
    );
    // Clear and rename (send_keys appends, so select-all first).
    name_input.clear().await?;
    name_input.send_keys("Pancakes Deluxe").await?;
    click_scrolled(&driver, "recipe-submit").await?;
    wait_for_url_path_prefix(driver, "/recipe/").await?;
    if let Err(err) = assert_detail_view(driver, "Pancakes Deluxe", &["Cook in a hot pan"]).await {
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
        add_ingredient_modal(&driver, "", "", "Cocoa", "").await?;
        // Set the file input directly (WebDriver standard behaviour).
        driver
            .find(By::Id("recipe-photo"))
            .await?
            .send_keys(png_path.to_str().unwrap())
            .await?;
        click_scrolled(&driver, "recipe-submit").await?;

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
            sections: vec!["Base".into()],
            ingredients: vec![
                Ingredient {
                    quantity: Some(300.0),
                    unit: Some("ml".into()),
                    name: "water".into(),
                    prep: None,
                    section: Some("Base".into()),
                },
                Ingredient {
                    quantity: None,
                    unit: None,
                    name: "salt".into(),
                    prep: Some("to taste".into()),
                    section: None,
                },
            ],
            instructions: vec![
                InstructionStep { text: "Boil water".into(), section: None },
                InstructionStep { text: "Add salt".into(), section: None },
            ],
            instruction_sections: vec![],
            notes: "Tastes better the next day.".into(),
            yield_amount: "4 bowls".into(),
            source: "Old family cookbook".into(),
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
    assert_eq!(detail.sections, vec!["Base"]);
    assert_eq!(detail.ingredients.len(), 2);
    assert_eq!(detail.ingredients[1].prep.as_deref(), Some("to taste"));
    assert_eq!(
        detail.instructions,
        vec![
            InstructionStep { text: "Boil water".into(), section: None },
            InstructionStep { text: "Add salt".into(), section: None },
        ]
    );
    assert_eq!(detail.instruction_sections, Vec::<String>::new());
    assert_eq!(detail.notes, "Tastes better the next day.");
    assert_eq!(detail.yield_amount, "4 bowls");
    assert_eq!(detail.source, "Old family cookbook");

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
        "[{\"text\":\"chop\",\"section\":\"Prep\"}]\r\n",
        "--e2eUpDbNd\r\n",
        "Content-Disposition: form-data; name=\"instruction_sections\"\r\n\r\n",
        "[\"Prep\"]\r\n",
        "--e2eUpDbNd\r\n",
        "Content-Disposition: form-data; name=\"notes\"\r\n\r\n",
        "Updated notes\r\n",
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
    assert_eq!(
        updated.instructions,
        vec![InstructionStep { text: "chop".into(), section: Some("Prep".into()) }]
    );
    assert_eq!(updated.instruction_sections, vec!["Prep"]);
    assert_eq!(updated.notes, "Updated notes");
    // Meta fields not present in the PUT are cleared.
    assert_eq!(updated.yield_amount, "");

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
        .json(&RecipeInput {
            name: "  ".into(),
            sections: vec![],
            ingredients: vec![],
            instructions: vec![],
            instruction_sections: vec![],
            notes: String::new(),
            yield_amount: String::new(),
            source: String::new(),
        })
        .send()
        .await?
        .status();
    assert!(
        status.is_client_error(),
        "empty recipe name must be rejected, got {status}"
    );

    Ok(())
}

/// Drag & drop: dragging a step's ≡ handle below the next step reorders the
/// list (mouse action chain; pointer events power the real interaction).
#[tokio::test(flavor = "multi_thread")]
async fn drag_reorders_steps() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    // Create a recipe with two steps directly through the API.
    let status = http
        .post(format!("{base}/api/recipes"))
        .json(&RecipeInput {
            name: "Sortable".into(),
            sections: vec![],
            ingredients: vec![],
            instructions: vec![
                InstructionStep { text: "First step".into(), section: None },
                InstructionStep { text: "Second step".into(), section: None },
            ],
            instruction_sections: vec![],
            notes: String::new(),
            yield_amount: String::new(),
            source: String::new(),
        })
        .send()
        .await?
        .status();
    assert_eq!(status, 201);

    wait_for_port(&webdriver_addr()).await?;
    let driver = open_headless_firefox().await?;
    let result = (|| async {
        let recipes: Vec<Recipe> = http
            .get(format!("{base}/api/recipes"))
            .send()
            .await?
            .json()
            .await?;
        let id = recipes[0].id;
        driver.goto(format!("{base}/edit/{id}")).await?;
        wait_for_url_path(&driver, &format!("/edit/{id}")).await?;

        let before = step_texts(&driver).await?;
        anyhow::ensure!(
            before == vec!["First step", "Second step"],
            "unexpected initial steps: {before:?}"
        );

        // Drag the first step's handle ~1.5 rows down.
        let handle = driver
            .find(By::Id("drag-handle-0"))
            .await
            .context("drag handle for the first step missing")?;
        let height = handle.rect().await?.height;
        driver
            .action_chain()
            .move_to_element_center(&handle)
            .click_and_hold()
            .move_by_offset(0, (height * 1.5) as i64)
            .release()
            .perform()
            .await?;
        tokio::time::sleep(Duration::from_millis(400)).await;

        let after = step_texts(&driver).await?;
        anyhow::ensure!(
            after == vec!["Second step", "First step"],
            "drag did not reorder steps: {after:?}"
        );
        Ok(())
    })()
    .await;
    let _ = driver.quit().await;
    result
}

/// Detail-view scale: multiplies displayed quantities, validates live and
/// resets; nothing is persisted.
#[tokio::test(flavor = "multi_thread")]
async fn scale_multiplies_quantities() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    let status = http
        .post(format!("{base}/api/recipes"))
        .json(&RecipeInput {
            name: "Scalable".into(),
            sections: vec![],
            ingredients: vec![Ingredient {
                quantity: Some(200.0),
                unit: Some("g".into()),
                name: "Flour".into(),
                prep: None,
                section: None,
            }],
            instructions: vec![],
            instruction_sections: vec![],
            notes: String::new(),
            yield_amount: String::new(),
            source: String::new(),
        })
        .send()
        .await?
        .status();
    assert_eq!(status, 201);

    wait_for_port(&webdriver_addr()).await?;
    let driver = open_headless_firefox().await?;
    let result = (|| async {
        let recipes: Vec<Recipe> = http
            .get(format!("{base}/api/recipes"))
            .send()
            .await?
            .json()
            .await?;
        let id = recipes[0].id;
        driver.goto(format!("{base}/recipe/{id}")).await?;
        wait_for_url_path(&driver, &format!("/recipe/{id}")).await?;
        driver.find(By::Id("ingredient-list")).await?;
        let list = driver.find(By::Id("ingredient-list")).await?;
        let text = list.text().await?;
        anyhow::ensure!(text.contains("200 g Flour"), "unscaled text wrong: {text}");

        // Scale 2x -> 400 g.
        driver.find(By::Id("scale-input")).await?.send_keys("2").await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        let text = list.text().await?;
        anyhow::ensure!(
            text.contains("400 g Flour"),
            "scaled quantity missing (expected 400 g): {text}"
        );

        // An out-of-range scale shows the hint and does not scale.
        driver
            .find(By::Id("scale-input"))
            .await?
            .send_keys("000")
            .await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        driver
            .find(By::Css(".scale-hint"))
            .await
            .context("scale hint missing for an out-of-range scale")?;
        let text = list.text().await?;
        anyhow::ensure!(
            text.contains("200 g Flour"),
            "invalid scale must fall back to 1x: {text}"
        );

        // Reset returns to 1x.
        driver.find(By::Id("scale-reset")).await?.click().await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        let text = list.text().await?;
        anyhow::ensure!(
            text.contains("200 g Flour"),
            "reset did not restore 1x: {text}"
        );

        // The stored recipe is untouched by scaling.
        let detail = poll_detail_full(&http, &base, id).await?;
        anyhow::ensure!(
            detail.ingredients[0].quantity == Some(200.0),
            "scale must not be persisted into stored quantities"
        );
        Ok(())
    })()
    .await;
    let _ = driver.quit().await;
    result
}

// ---------------------------------------------------------------------------
// Shared UI assertions & helpers
// ---------------------------------------------------------------------------

/// Scroll a form element into view before clicking it (fixed bottom nav
/// otherwise intercepts clicks near the viewport bottom).
async fn click_scrolled(driver: &WebDriver, id: &str) -> anyhow::Result<()> {
    driver
        .execute(
            format!(
                "var el = document.getElementById('{id}'); if (el) el.scrollIntoView({{block: 'center'}});"
            ),
            Vec::<serde_json::Value>::new(),
        )
        .await?;
    driver.find(By::Id(id)).await?.click().await?;
    Ok(())
}

/// Open the ingredient modal, fill it and save. Empty qty/unit/prep are
/// skipped so the recipe can have unquantified ingredients.
async fn add_ingredient_modal(
    driver: &WebDriver,
    qty: &str,
    unit: &str,
    name: &str,
    prep: &str,
) -> anyhow::Result<()> {
    click_scrolled(driver, "add-ingredient").await?;
    let modal = driver
        .find(By::Id("modal-name"))
        .await
        .context("ingredient modal did not open")?;
    if !qty.is_empty() {
        driver.find(By::Id("modal-qty")).await?.send_keys(qty).await?;
    }
    if !unit.is_empty() {
        driver.find(By::Id("modal-unit")).await?.send_keys(unit).await?;
    }
    modal.send_keys(name).await?;
    if !prep.is_empty() {
        driver.find(By::Id("modal-prep")).await?.send_keys(prep).await?;
    }
    click_scrolled(driver, "modal-save").await?;
    Ok(())
}

/// Open the instruction modal, fill it and save.
async fn add_step_modal(
    driver: &WebDriver,
    text: &str,
    section: Option<&str>,
) -> anyhow::Result<()> {
    click_scrolled(driver, "add-step").await?;
    if let Some(section) = section {
        // The select defaults to the last section; only interact with it when
        // an explicit different section is requested.
        let select_elem = driver.find(By::Id("modal-step-section")).await?;
        let select = thirtyfour::components::SelectElement::new(&select_elem).await?;
        select.select_by_value(section).await?;
    }
    driver
        .find(By::Id("modal-step-text"))
        .await
        .context("instruction modal did not open")?
        .send_keys(text)
        .await?;
    click_scrolled(driver, "modal-save").await?;
    Ok(())
}

/// Create a new instruction section via its modal.
async fn add_step_section_modal(driver: &WebDriver, name: &str) -> anyhow::Result<()> {
    click_scrolled(driver, "add-step-section").await?;
    driver
        .find(By::Id("modal-section-name"))
        .await
        .context("instruction-section modal did not open")?
        .send_keys(name)
        .await?;
    click_scrolled(driver, "modal-save").await?;
    Ok(())
}

/// Open the ingredient modal in "add" mode.
async fn open_ingredient_modal(driver: &WebDriver) -> anyhow::Result<()> {
    click_scrolled(driver, "add-ingredient").await?;
    driver
        .find(By::Id("modal-name"))
        .await
        .context("ingredient modal did not open")?;
    Ok(())
}

/// Texts of the ingredient rows shown in the editor list.
async fn ingredient_row_texts(driver: &WebDriver) -> anyhow::Result<Vec<String>> {
    let rows = driver.find_all(By::Css("#ingredient-rows .ingredient-row")).await?;
    let mut texts = Vec::new();
    for row in rows {
        texts.push(row.text().await?.replace('\n', " "));
    }
    Ok(texts)
}

/// Texts of the instruction steps shown in the editor list.
async fn step_texts(driver: &WebDriver) -> anyhow::Result<Vec<String>> {
    let rows = driver.find_all(By::Css("#step-rows .step-row .step-text")).await?;
    let mut texts = Vec::new();
    for row in rows {
        texts.push(row.text().await?);
    }
    Ok(texts)
}

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
async fn assert_detail_view(
    driver: &WebDriver,
    name: &str,
    expect_instructions: &[&str],
) -> anyhow::Result<()> {
    let h1 = driver.find(By::Css(".detail-name")).await?;
    let shown = h1.text().await?;
    anyhow::ensure!(shown == name, "detail shows '{shown}', expected '{name}'");
    // Header bar: real actions + placeholders.
    driver.find(By::Id("hdr-back")).await?;
    driver.find(By::Id("hdr-edit")).await?;
    driver.find(By::Id("hdr-delete")).await?;
    driver.find(By::Css(".detail-header .hdr-btn.ph")).await?;
    // Grouped ingredient list with bullet items.
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
    anyhow::ensure!(
        text.contains("Batter"),
        "section title missing from detail: '{text}'"
    );
    // Numbered instructions.
    let steps = driver
        .find(By::Id("instruction-list"))
        .await
        .context("instruction list missing")?;
    let text = steps.text().await?;
    anyhow::ensure!(
        text.contains("Cooking"),
        "instruction section title missing from detail: '{text}'"
    );
    for expected in expect_instructions {
        anyhow::ensure!(
            text.contains(expected),
            "instruction '{expected}' missing from detail: '{text}'"
        );
    }
    // Meta sections (filled in by the editor flow).
    driver
        .find(By::Id("detail-yield"))
        .await
        .context("Yield section missing from detail")?;
    driver
        .find(By::Id("detail-source"))
        .await
        .context("Source section missing from detail")?;
    driver
        .find(By::Id("detail-notes"))
        .await
        .context("Notes section missing from detail")?;
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
