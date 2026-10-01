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
/// and return its bound address. Auth is off — every existing flow test
/// relies on that; the auth test uses [`spawn_test_backend_with_password`].
async fn spawn_test_backend() -> anyhow::Result<SocketAddr> {
    spawn_test_backend_with_password(None).await
}

/// Same, with the household password configured (auth on).
async fn spawn_test_backend_with_password(password: Option<&str>) -> anyhow::Result<SocketAddr> {
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
        // The browser import test fetches a loopback fixture page.
        import_allow_private: true,
        // No OpenRouter key in e2e: unclassified items simply stay in Other.
        openrouter_key: None,
        classifier_model: None,
        classifier_endpoint: None,
        password: password.map(str::to_string),
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

    // -- 2. The + FAB opens a menu; "Add manually" opens the add form. ------
    driver.find(By::Id("fab-add-recipe")).await?.click().await?;
    driver
        .find(By::Id("fab-menu-manual"))
        .await
        .context("+ menu did not open")?
        .click()
        .await?;
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

    // Manual add: Enter submits; the add row has no group field anymore.
    let input = driver.find(By::Id("grocery-input")).await?;
    input.send_keys("Bananas").await?;
    input.send_keys("\u{E007}").await?;
    driver
        .find(By::XPath("//li[contains(., 'Bananas')]"))
        .await
        .context("manually added item did not appear in the UI")?;
    poll_grocery(&http, base, "Bananas", None).await?;

    // Suggestions: typing a prefix of an on-list item offers it in the
    // dropdown; picking it fills the input and Enter adds a second line.
    let input = driver.find(By::Id("grocery-input")).await?;
    input.send_keys("Bana").await?;
    let mut suggestion = None;
    for _ in 0..25 {
        if let Ok(el) = driver.find(By::Css(".suggestions .suggestion")).await {
            suggestion = Some(el);
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let suggestion =
        suggestion.context("no suggestion dropdown for a typed prefix")?;
    let text = suggestion.text().await?;
    anyhow::ensure!(
        text.to_lowercase().contains("banana"),
        "unexpected suggestion text: {text}"
    );
    suggestion.click().await?;
    let picked = input
        .prop("value")
        .await?
        .context("input lost while picking a suggestion")?;
    anyhow::ensure!(
        picked.eq_ignore_ascii_case("Bananas"),
        "picking a suggestion must fill the input, got {picked:?}"
    );
    input.send_keys("\u{E007}").await?;
    // The pick added a second "Bananas" line (separate lines by design).
    let mut bananas = 0usize;
    for _ in 0..50 {
        let items: Vec<GroceryItem> = http
            .get(format!("{base}/api/grocery"))
            .send()
            .await?
            .json()
            .await?;
        bananas = items.iter().filter(|i| i.name == "Bananas").count();
        if bananas >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::ensure!(
        bananas >= 2,
        "picking the suggestion + Enter did not add a second Bananas line"
    );

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
        driver.find(By::Id("fab-menu-manual")).await?.click().await?;
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

    // Grocery manual add still works; the bought toggle deletes the item.
    let item: GroceryItem = http
        .post(format!("{base}/api/grocery"))
        .json(&NewGroceryItem { name: "Potatoes".into(), category: None })
        .send()
        .await?
        .json()
        .await?;
    assert_eq!(item.category, shared::DEFAULT_CATEGORY);
    let status = http
        .patch(format!("{base}/api/grocery/{}", item.id))
        .json(&shared::GroceryUpdate { bought: true })
        .send()
        .await?
        .status();
    assert_eq!(status, 204, "toggling to bought must delete the item");
    for _ in 0..50 {
        let grocery: Vec<GroceryItem> = http
            .get(format!("{base}/api/grocery"))
            .send()
            .await?
            .json()
            .await?;
        if grocery.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let grocery: Vec<GroceryItem> = http
        .get(format!("{base}/api/grocery"))
        .send()
        .await?
        .json()
        .await?;
    anyhow::ensure!(
        grocery.is_empty(),
        "toggled item must be gone from the database: {grocery:?}"
    );

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

/// Recipe → shopping list: the header cart button opens a bottom sheet with
/// one unchecked checkbox per ingredient, the All toggle selects everything,
/// unchecking one excludes just that line, Add pushes the selected lines as
/// separate grocery items with their base quantities (scale ignored), and
/// Cancel leaves the list untouched.
#[tokio::test(flavor = "multi_thread")]
async fn recipe_add_to_shopping_list_flow() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    let status = http
        .post(format!("{base}/api/recipes"))
        .json(&RecipeInput {
            name: "Sheet Cake".into(),
            sections: vec![],
            ingredients: vec![
                Ingredient {
                    quantity: Some(200.0),
                    unit: Some("g".into()),
                    name: "Flour".into(),
                    prep: None,
                    section: None,
                },
                Ingredient {
                    quantity: Some(2.0),
                    unit: Some("tbsp".into()),
                    name: "Soy sauce".into(),
                    prep: Some("thinly sliced".into()),
                    section: None,
                },
                Ingredient {
                    quantity: None,
                    unit: None,
                    name: "Salt".into(),
                    prep: None,
                    section: None,
                },
            ],
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

        // Set a 2x scale first: the sheet must still offer base quantities.
        driver.find(By::Id("scale-input")).await?.send_keys("2").await?;
        tokio::time::sleep(Duration::from_millis(300)).await;

        // Open the sheet.
        driver.find(By::Id("hdr-cart")).await?.click().await?;
        let sheet = driver.find(By::Css(".sheet")).await.context("sheet missing")?;
        let rows = driver.find_all(By::Css(".sheet .sheet-row")).await?;
        anyhow::ensure!(rows.len() == 3, "expected 3 sheet rows, got {}", rows.len());
        let texts = sheet.text().await?;
        for expected in ["200 g Flour", "2 tbsp Soy sauce", "Salt"] {
            anyhow::ensure!(texts.contains(expected), "sheet missing '{expected}': {texts}");
        }
        anyhow::ensure!(
            !texts.contains("400 g Flour"),
            "sheet must show base quantities, not scaled: {texts}"
        );
        // Prep text is cooking information — it must not reach the list.
        anyhow::ensure!(
            !texts.contains("thinly sliced"),
            "sheet must not include prep text: {texts}"
        );
        for row in &rows {
            let checkbox = row.find(By::Css("input[type=checkbox]")).await?;
            anyhow::ensure!(
                !checkbox.is_selected().await?,
                "sheet lines must start unchecked"
            );
        }

        // Add is disabled while nothing is selected.
        let add = driver.find(By::Id("sheet-add")).await?;
        anyhow::ensure!(!add.is_enabled().await?, "Add must be disabled with no selection");

        // All selects everything, unchecking the flour excludes only it.
        driver.find(By::Id("sheet-all")).await?.click().await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        let rows = driver.find_all(By::Css(".sheet .sheet-row")).await?;
        rows[0].find(By::Css("input[type=checkbox]")).await?.click().await?;
        driver.find(By::Id("sheet-add")).await?.click().await?;
        wait_for_gone(&driver, ".sheet").await?;

        // Exactly the two selected lines landed on the list, in order.
        poll_grocery(&http, &base, "2 tbsp Soy sauce", None).await?;
        poll_grocery(&http, &base, "Salt", None).await?;
        let items: Vec<GroceryItem> = http
            .get(format!("{base}/api/grocery"))
            .send()
            .await?
            .json()
            .await?;
        let names: Vec<&str> = items.iter().map(|i| i.name.as_str()).collect();
        anyhow::ensure!(
            names == ["2 tbsp Soy sauce", "Salt"],
            "unexpected grocery contents: {names:?}"
        );
        anyhow::ensure!(
            driver.find(By::Css(".added-note")).await?.text().await?.contains("Added 2 items"),
            "confirmation note missing"
        );

        // Cancel leaves the list untouched, and the next open starts fresh.
        driver.find(By::Id("hdr-cart")).await?.click().await?;
        let rows = driver.find_all(By::Css(".sheet .sheet-row")).await?;
        anyhow::ensure!(rows.len() == 3, "sheet must reopen with all rows");
        for row in &rows {
            anyhow::ensure!(
                !row.find(By::Css("input[type=checkbox]")).await?.is_selected().await?,
                "reopened sheet must start unchecked again"
            );
        }
        driver.find(By::Id("sheet-cancel")).await?.click().await?;
        wait_for_gone(&driver, ".sheet").await?;
        let items: Vec<GroceryItem> = http
            .get(format!("{base}/api/grocery"))
            .send()
            .await?
            .json()
            .await?;
        anyhow::ensure!(items.len() == 2, "Cancel must not change the list");
        Ok(())
    })()
    .await;
    let _ = driver.quit().await;
    result
}

/// Grocery provenance + editing: adding a planned recipe's ingredients
/// stamps the provenance recipe on each line, the edit sheet shows the
/// recipe with its plan day ("… in 4 days"), renaming via the sheet updates
/// the list and the database, and checking the row's checkbox marks it
/// bought (removed).
#[tokio::test(flavor = "multi_thread")]
async fn grocery_edit_provenance_and_bought_flow() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    // A recipe planned 4 days out, with two ingredients to push.
    let status = http
        .post(format!("{base}/api/recipes"))
        .json(&RecipeInput {
            name: "Provenance Loaf".into(),
            sections: vec![],
            ingredients: vec![
                Ingredient {
                    quantity: Some(120.0),
                    unit: Some("ml".into()),
                    name: "Lentils".into(),
                    prep: None,
                    section: None,
                },
                Ingredient {
                    quantity: Some(1.0),
                    unit: None,
                    name: "Onion".into(),
                    prep: None,
                    section: None,
                },
            ],
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
    let recipes: Vec<Recipe> = http
        .get(format!("{base}/api/recipes"))
        .send()
        .await?
        .json()
        .await?;
    let recipe_id = recipes[0].id;

    let planned = (chrono::Local::now().date_naive() + chrono::Duration::days(4))
        .format("%Y-%m-%d");
    let status = http
        .post(format!("{base}/api/meal-plan"))
        .json(&serde_json::json!({ "date": planned.to_string(), "recipe_id": recipe_id }))
        .send()
        .await?
        .status();
    assert_eq!(status, 201);

    wait_for_port(&webdriver_addr()).await?;
    let driver = open_headless_firefox().await?;
    let result = (|| async {
        // Push the ingredients from the detail page's cart sheet.
        driver.goto(format!("{base}/recipe/{recipe_id}")).await?;
        wait_for_url_path(&driver, &format!("/recipe/{recipe_id}")).await?;
        driver.find(By::Id("ingredient-list")).await?;
        driver.find(By::Id("hdr-cart")).await?.click().await?;
        driver.find(By::Id("sheet-all")).await?.click().await?;
        driver.find(By::Id("sheet-add")).await?.click().await?;
        wait_for_gone(&driver, ".sheet").await?;

        driver.goto(format!("{base}/grocery")).await?;
        wait_for_url_path(&driver, "/grocery").await?;
        driver
            .find(By::XPath("//li[contains(@class, 'grocery-item') and contains(., '120 ml Lentils')]"))
            .await
            .context("recipe ingredient did not reach the grocery list")?;

        // Tap the row: the edit sheet opens with the provenance line.
        driver
            .find(By::XPath("//li[contains(@class, 'grocery-item') and contains(., '120 ml Lentils')]"))
            .await?
            .click()
            .await?;
        driver.find(By::Id("sheet-item-name")).await
            .context("edit sheet did not open")?;
        let source = driver.find(By::Id("sheet-item-source")).await?.text().await?;
        anyhow::ensure!(
            source == "Provenance Loaf in 4 days",
            "provenance line wrong: '{source}'"
        );

        // Rename and move to a named group. The group control must be the
        // emoji button grid (it went through datalist and dropdown before)
        // offering every preset category.
        let name_input = driver.find(By::Id("sheet-item-name")).await?;
        name_input.clear().await?;
        name_input.send_keys("Red lentils").await?;
        let buttons = driver
            .find_all(By::Css(".sheet-cats .cat-btn"))
            .await?;
        anyhow::ensure!(
            buttons.len() >= 14,
            "expected the preset categories as buttons, got {}",
            buttons.len()
        );
        // The current group starts selected.
        driver
            .find(By::Css(".sheet-cats .cat-btn.selected .cat-name"))
            .await?
            .text()
            .await?;
        // Tap the Pantry button.
        driver.find(By::Id("cat-Pantry")).await?.click().await?;
        driver.find(By::Id("sheet-item-save")).await?.click().await?;
        wait_for_gone(&driver, ".sheet").await?;

        let texts = grocery_item_texts(&driver).await?;
        anyhow::ensure!(
            texts.iter().any(|t| t.contains("Red lentils")),
            "renamed item missing from the list: {texts:?}"
        );
        let items: Vec<serde_json::Value> = http
            .get(format!("{base}/api/grocery"))
            .send()
            .await?
            .json()
            .await?;
        let lentils = items
            .iter()
            .find(|i| i["name"] == "Red lentils")
            .context("rename never reached the database")?;
        anyhow::ensure!(
            lentils["category"] == "Pantry",
            "group change never reached the database: {items:?}"
        );

        // Checking the box marks the item bought: the row disappears.
        driver
            .find(By::XPath(
                "//li[contains(@class, 'grocery-item') and contains(., 'Red lentils')]//input[@type='checkbox']",
            ))
            .await?
            .click()
            .await?;
        poll_grocery_gone(&http, &base, "Red lentils").await?;
        let items: Vec<serde_json::Value> = http
            .get(format!("{base}/api/grocery"))
            .send()
            .await?
            .json()
            .await?;
        anyhow::ensure!(items.len() == 1, "only the lentils line should be gone: {items:?}");
        Ok(())
    })()
    .await;
    let _ = driver.quit().await;
    result
}

/// Recipes tab: live fuzzy search filters the grid, the sort menu orders it
/// (A–Z, then Random twice with fresh orders and a persisted choice), and
/// the detail header actions work — the meal-plan day chooser schedules the
/// recipe and Share surfaces the link.
#[tokio::test(flavor = "multi_thread")]
async fn recipes_search_sort_and_detail_actions_flow() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    for (name, ingredient) in [
        ("Alpha Pancakes", "soy milk"),
        ("Beta Curry", "rice"),
        ("Gamma Soup", "soup greens"),
        ("Delta Loaf", "flour"),
        ("Epsilon Stew", "lentils"),
    ] {
        let payload = format!(
            r#"{{"name":"{name}","sections":[],"ingredients":[{{"quantity":1.0,"unit":"g","name":"{ingredient}","prep":null,"section":null}}],"instructions":[],"instruction_sections":[],"notes":"","yield":"","source":""}}"#
        );
        let status = http
            .post(format!("{base}/api/recipes"))
            .body(payload)
            .header("content-type", "application/json")
            .send()
            .await?
            .status();
        assert_eq!(status, 201);
    }

    wait_for_port(&webdriver_addr()).await?;
    let driver = open_headless_firefox().await?;
    let result = (|| async {
        driver.goto(format!("{base}/")).await?;
        wait_for_url_path(&driver, "/").await?;
        driver.find(By::Id("recipe-grid")).await?;

        let card_names = || async {
            let names = driver
                .find_all(By::Css(".recipe-card-name"))
                .await?;
            let mut texts = Vec::new();
            for name in &names {
                texts.push(name.text().await?);
            }
            Ok::<Vec<String>, anyhow::Error>(texts)
        };

        // -- Search: a typo'd query fuzzy-matches "Soup". --------------------
        driver.find(By::Id("fab-search")).await?.click().await?;
        let input = driver.find(By::Id("recipe-search")).await
            .context("search bar did not open")?;
        input.send_keys("soop").await?;
        tokio::time::sleep(Duration::from_millis(900)).await;
        let texts = card_names().await?;
        anyhow::ensure!(
            texts == ["Gamma Soup"],
            "fuzzy search should leave only Gamma Soup: {texts:?}"
        );

        // Closing search restores the full grid.
        driver.find(By::Id("search-close")).await?.click().await?;
        tokio::time::sleep(Duration::from_millis(400)).await;
        anyhow::ensure!(card_names().await?.len() == 5, "closing search must restore the grid");

        // -- Sort: A–Z, then Random twice (fresh order, persisted). ----------
        driver.find(By::Id("fab-sort")).await?.click().await?;
        driver
            .find(By::Id("sort-name_asc"))
            .await
            .context("sort menu did not open")?
            .click()
            .await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        anyhow::ensure!(
            card_names().await?.first().context("empty grid")? == "Alpha Pancakes",
            "A–Z sort must put Alpha Pancakes first"
        );

        driver.find(By::Id("fab-sort")).await?.click().await?;
        driver.find(By::Id("sort-random")).await?.click().await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let first_random = card_names().await?;
        driver.find(By::Id("fab-sort")).await?.click().await?;
        driver.find(By::Id("sort-random")).await?.click().await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let second_random = card_names().await?;
        anyhow::ensure!(
            first_random != second_random,
            "two Random presses must reshuffle: {first_random:?} vs {second_random:?}"
        );
        let sort_setting: serde_json::Value = http
            .get(format!("{base}/api/settings/recipes_sort"))
            .send()
            .await?
            .json()
            .await?;
        anyhow::ensure!(
            sort_setting["value"] == "random",
            "sort choice must persist: {sort_setting:?}"
        );

        // -- Detail: day chooser adds the recipe to tomorrow. ----------------
        // Another recipe is already planned for today: the chooser must show
        // it (blaz-style thumbnails), and picking tomorrow schedules Alpha.
        let today = chrono::Local::now().date_naive().format("%Y-%m-%d").to_string();
        let recipes: Vec<serde_json::Value> = http
            .get(format!("{base}/api/recipes"))
            .send()
            .await?
            .json()
            .await?;
        let gamma = recipes
            .iter()
            .find(|r| r["name"] == "Gamma Soup")
            .context("Gamma Soup missing")?;
        let status = http
            .post(format!("{base}/api/meal-plan"))
            .json(&serde_json::json!({ "date": today, "recipe_id": gamma["id"] }))
            .send()
            .await?
            .status();
        assert_eq!(status, 201);

        driver
            .find(By::XPath(
                "//div[contains(@class, 'recipe-card-name') and contains(., 'Alpha Pancakes')]",
            ))
            .await?
            .click()
            .await?;
        wait_for_url_path_prefix(&driver, "/recipe/").await?;
        driver.find(By::Id("hdr-mealplan")).await?.click().await?;
        let sheet = driver.find(By::Css(".sheet")).await
            .context("day chooser did not open")?;
        let sheet_text = sheet.text().await?;
        anyhow::ensure!(
            sheet_text.contains("Assign \u{201c}Alpha Pancakes\u{201d} to"),
            "chooser must name the recipe: {sheet_text}"
        );
        let day_buttons = driver.find_all(By::Css(".sheet .day-btn")).await?;
        anyhow::ensure!(day_buttons.len() == 14, "expected 14 day buttons");
        anyhow::ensure!(
            day_buttons[0].text().await?.contains("Gamma Soup"),
            "today's row must show the planned recipe"
        );
        anyhow::ensure!(
            day_buttons[1].text().await?.contains("Nothing planned"),
            "tomorrow's row should start empty"
        );
        day_buttons[1].click().await?; // Tomorrow
        tokio::time::sleep(Duration::from_millis(800)).await;
        let tomorrow = (chrono::Local::now().date_naive() + chrono::Duration::days(1))
            .format("%Y-%m-%d")
            .to_string();
        let entries: Vec<serde_json::Value> = http
            .get(format!("{base}/api/meal-plan"))
            .send()
            .await?
            .json()
            .await?;
        anyhow::ensure!(
            entries.len() == 2,
            "expected the alpha entry alongside today's: {entries:?}"
        );
        anyhow::ensure!(
            entries
                .iter()
                .any(|e| e["date"] == tomorrow
                    && e["recipe"]["name"] == "Alpha Pancakes"),
            "meal-plan entry for tomorrow missing: {entries:?}"
        );

        // -- Share: surfaces the link note. ----------------------------------
        driver.find(By::Id("hdr-share")).await?.click().await?;
        let note = driver
            .find(By::Css(".added-note"))
            .await
            .context("share note missing")?
            .text()
            .await?;
        anyhow::ensure!(
            note.contains("copied") || note.contains("http"),
            "share note wrong: '{note}'"
        );
        Ok(())
    })()
    .await;
    let _ = driver.quit().await;
    result
}

/// Auth: recipe browsing stays public while the private tabs show the login
/// gate; a wrong password errors, the right one reveals the tab, and the
/// detail-page actions only exist for the authenticated.
#[tokio::test(flavor = "multi_thread")]
async fn auth_login_gate_flow() -> anyhow::Result<()> {
    let addr = spawn_test_backend_with_password(Some("fondue")).await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    // Anonymous API: grocery is gated, writes are gated…
    let status = http.get(format!("{base}/api/grocery")).send().await?.status();
    assert_eq!(status, 401);
    let status = http
        .post(format!("{base}/api/recipes"))
        .body(r#"{"name":"Gate Loaf","sections":[],"ingredients":[],"instructions":[],"instruction_sections":[],"notes":"","yield":"","source":""}"#)
        .header("content-type", "application/json")
        .send()
        .await?
        .status();
    assert_eq!(status, 401);

    // …but a logged-in cookie client can seed a recipe for browsing.
    let authed = reqwest::Client::builder().cookie_store(true).build()?;
    let status = authed
        .post(format!("{base}/api/login"))
        .json(&serde_json::json!({ "password": "fondue" }))
        .send()
        .await?
        .status();
    assert_eq!(status, 204);
    let status = authed
        .post(format!("{base}/api/recipes"))
        .json(&serde_json::json!({
            "name": "Gate Loaf",
            "sections": [],
            "ingredients": [{"quantity": 1.0, "unit": "g", "name": "flour", "prep": null, "section": null}],
            "instructions": [],
            "instruction_sections": [],
            "notes": "",
            "yield": "",
            "source": ""
        }))
        .send()
        .await?
        .status();
    assert_eq!(status, 201);

    wait_for_port(&webdriver_addr()).await?;
    let driver = open_headless_firefox().await?;
    let result = (|| async {
        // The recipes grid loads without any login.
        driver.goto(format!("{base}/")).await?;
        wait_for_url_path(&driver, "/").await?;
        driver
            .find(By::Css(".recipe-card-name"))
            .await
            .context("public grid did not render")?;

        // The Shopping tab is the login gate.
        driver.goto(format!("{base}/grocery")).await?;
        wait_for_url_path(&driver, "/grocery").await?;
        driver
            .find(By::Id("login-password"))
            .await
            .context("login gate missing on the shopping tab")?;

        // Wrong password errors.
        driver
            .find(By::Id("login-password"))
            .await?
            .send_keys("wrong")
            .await?;
        driver.find(By::Id("login-submit")).await?.click().await?;
        tokio::time::sleep(Duration::from_millis(600)).await;
        let error = driver
            .find(By::Css(".login-card .status-error"))
            .await?
            .text()
            .await?;
        anyhow::ensure!(error.contains("Wrong password"), "login error wrong: '{error}'");

        // The right password reveals the list.
        let input = driver.find(By::Id("login-password")).await?;
        input.clear().await?;
        input.send_keys("fondue").await?;
        driver.find(By::Id("login-submit")).await?.click().await?;
        tokio::time::sleep(Duration::from_millis(800)).await;
        driver
            .find(By::Id("grocery-input"))
            .await
            .context("grocery content did not appear after login")?;

        // The detail page now shows the authenticated actions.
        driver.goto(format!("{base}/recipe/1")).await?;
        wait_for_url_path_prefix(&driver, "/recipe/").await?;
        driver.find(By::Id("ingredient-list")).await?;
        driver
            .find(By::Id("hdr-cart"))
            .await
            .context("detail actions missing after login")?;
        Ok(())
    })()
    .await;
    let _ = driver.quit().await;
    result
}

/// Poll until no element matches `selector` (e.g. a dialog has closed).
async fn wait_for_gone(driver: &WebDriver, selector: &str) -> anyhow::Result<()> {
    for _ in 0..50 {
        if driver.find_all(By::Css(selector)).await?.is_empty() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("'{selector}' never disappeared")
}

/// Shopping-list UX: the suggestions dropdown only opens for typed text while
/// the input is focused, "appel" fuzzy-matches the past entry "apple", the ×
/// button removes an item, and a removed item never comes back when another
/// one is added afterwards.
#[tokio::test(flavor = "multi_thread")]
async fn grocery_suggestions_removal_and_no_resurrection() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    wait_for_port(&addr.to_string()).await?;
    wait_for_port(&webdriver_addr()).await?;
    let driver = open_headless_firefox().await?;
    let result = (|| async {
        driver.goto(format!("{base}/grocery")).await?;
        wait_for_url_path(&driver, "/grocery").await?;
        driver
            .find(By::Id("grocery-input"))
            .await
            .context("grocery input missing")?;

        // Empty input: the dropdown stays closed even while focused.
        driver.find(By::Id("grocery-input")).await?.click().await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        anyhow::ensure!(
            driver
                .find_all(By::Css(".suggestions .suggestion"))
                .await?
                .is_empty(),
            "dropdown must stay closed without typed text"
        );

        // Add "apple" so there is a past entry to suggest. The Add click
        // clears the input (controlled input — send_keys alone would append).
        driver.find(By::Id("grocery-input")).await?.send_keys("apple").await?;
        driver.find(By::Id("grocery-add")).await?.click().await?;
        driver
            .find(By::XPath("//li[contains(., 'apple')]"))
            .await
            .context("added item did not appear in the UI")?;
        poll_grocery(&http, &base, "apple", None).await?;

        // Typing a fuzzy typo opens the dropdown with the close match.
        let input = driver.find(By::Id("grocery-input")).await?;
        input.clear().await?;
        input.send_keys("appel").await?;
        let suggestion = driver
            .find(By::Css(".suggestions .suggestion"))
            .await
            .context("fuzzy suggestion for 'appel' did not appear");
        let suggestion = match suggestion {
            Ok(s) => s,
            Err(err) => {
                let src = driver.source().await.unwrap_or_default();
                anyhow::bail!("{err}; page source:\n{src}");
            }
        };
        suggestion.click().await?;
        tokio::time::sleep(Duration::from_millis(300)).await;
        let value = driver
            .find(By::Id("grocery-input"))
            .await?
            .value()
            .await?
            .unwrap_or_default();
        anyhow::ensure!(
            value == "apple",
            "suggestion click should fill 'apple', got '{value}'"
        );
        // Accepting the suggestion and adding creates a second "apple" row
        // (and clears the input again).
        driver.find(By::Id("grocery-add")).await?.click().await?;
        poll_grocery_count(&http, &base, "apple", 2).await?;

        // A second, different item in the same group. The input is empty
        // after Add, so send_keys types from scratch.
        driver.find(By::Id("grocery-input")).await?.send_keys("Milk").await?;
        driver.find(By::Id("grocery-add")).await?.click().await?;
        poll_grocery(&http, &base, "Milk", None).await?;
        let texts = grocery_item_texts(&driver).await?;
        anyhow::ensure!(
            texts.iter().any(|t| t.contains("apple")) && texts.iter().any(|t| t.contains("Milk")),
            "expected 'apple' and 'Milk' rows, got {texts:?}"
        );

        // Checking the box (bought) removes every "apple" row from the UI
        // and the database.
        for _ in 0..10 {
            let apple_rows = driver
                .find_all(By::XPath(
                    "//li[contains(@class, 'grocery-item') and contains(., 'apple')]",
                ))
                .await?;
            if apple_rows.is_empty() {
                break;
            }
            apple_rows[0]
                .find(By::Css("input[type=checkbox]"))
                .await
                .context("bought checkbox missing on the item row")?
                .click()
                .await?;
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        anyhow::ensure!(
            !grocery_item_texts(&driver).await?.iter().any(|t| t.contains("apple")),
            "removed item still visible in the UI"
        );
        poll_grocery_gone(&http, &base, "apple").await?;

        // Adding another item must NOT resurrect "apple".
        let bread_input = driver.find(By::Id("grocery-input")).await?;
        bread_input.clear().await?;
        bread_input.send_keys("Bread").await?;
        driver.find(By::Id("grocery-add")).await?.click().await?;
        poll_grocery(&http, &base, "Bread", None).await?;
        tokio::time::sleep(Duration::from_millis(800)).await;
        let texts = grocery_item_texts(&driver).await?;
        anyhow::ensure!(
            !texts.iter().any(|t| t.contains("apple")),
            "'apple' came back after adding a new item: {texts:?}"
        );
        let grocery: Vec<GroceryItem> = http
            .get(format!("{base}/api/grocery"))
            .send()
            .await?
            .json()
            .await?;
        anyhow::ensure!(
            !grocery.iter().any(|i| i.name == "apple"),
            "'apple' back in the database: {grocery:?}"
        );
        Ok(())
    })()
    .await;
    let _ = driver.quit().await;
    result
}

/// Import-from-URL: the browser flow fetches a local recipe page through the
/// backend importer, shows the preview in the editor, and saving goes through
/// the normal API. (The backend in this suite allows loopback targets so the
/// fixture page below can be fetched.)
#[tokio::test(flavor = "multi_thread")]
async fn import_from_url_flow() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    // A tiny recipe page (JSON-LD) served on loopback by the test itself.
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let fixture_addr = listener.local_addr()?;
    let fixture = r#"
<!DOCTYPE html>
<html><head><title>Breton Galette — Fixture</title>
<script type="application/ld+json">
{"@type":"Recipe","name":"Breton Galette Complète",
 "description":"A complete buckwheat galette.",
 "recipeYield":"2 galettes",
 "recipeIngredient":["2 buckwheat galettes","2 eggs","60 g gruyère, grated"],
 "recipeInstructions":[
   {"@type":"HowToStep","text":"Warm the galettes."},
   {"@type":"HowToStep","text":"Crack an egg onto each."},
   {"@type":"HowToStep","text":"Season.<br><strong>Chef's Notes:</strong> Rest before serving."},
   {"@type":"HowToStep","text":"Add cheese, fold and serve."}]}
</script></head>
<body><h1>Breton Galette Complète</h1></body></html>
"#;
    std::thread::spawn(move || {
        use std::io::{Read, Write};
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let mut buffer = [0u8; 4096];
            let _ = stream.read(&mut buffer);
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                fixture.len(),
                fixture
            );
            let _ = stream.write_all(response.as_bytes());
        }
    });

    // API-level: the endpoint returns a preview; nothing is persisted.
    let status = http
        .post(format!("{base}/api/recipes/import"))
        .json(&serde_json::json!({ "url": format!("http://{fixture_addr}/galette") }))
        .send()
        .await?
        .status();
    assert_eq!(status, 200, "import endpoint must return a preview");
    let recipes: Vec<Recipe> = http
        .get(format!("{base}/api/recipes"))
        .send()
        .await?
        .json()
        .await?;
    anyhow::ensure!(
        recipes.is_empty(),
        "import must not create rows: {recipes:?}"
    );

    // Browser flow: FAB → /import → URL → preview in the editor → save.
    wait_for_port(&webdriver_addr()).await?;
    let driver = open_headless_firefox().await?;
    let result = (|| async {
        driver.goto(format!("{base}/")).await?;
        driver.find(By::Id("fab-add-recipe")).await?.click().await?;
        driver
            .find(By::Id("fab-menu-import-url"))
            .await
            .context("+ menu did not open")?
            .click()
            .await?;
        wait_for_url_path(&driver, "/import").await?;
        driver
            .find(By::Id("import-url"))
            .await?
            .send_keys(format!("http://{fixture_addr}/galette"))
            .await?;
        driver.find(By::Id("import-fetch")).await?.click().await?;

        // The preview summary shows method + confidence and the editor is
        // prefilled with the imported data.
        let summary = driver
            .find(By::Id("import-summary"))
            .await
            .context("import summary did not appear")?;
        let summary_text = summary.text().await?;
        anyhow::ensure!(
            summary_text.contains("json_ld"),
            "summary should name the extraction method: {summary_text}"
        );

        // The prefilled editor carries the imported name and ingredients.
        let name_value = driver
            .find(By::Id("recipe-name"))
            .await?
            .value()
            .await?
            .unwrap_or_default();
        anyhow::ensure!(
            name_value == "Breton Galette Complète",
            "editor should be prefilled with the imported name, got '{name_value}'"
        );
        let rows = ingredient_row_texts(&driver).await?;
        anyhow::ensure!(
            rows.iter().any(|t| t.contains("gruyère")),
            "imported ingredients missing from the editor: {rows:?}"
        );

        // Saving goes through the normal create endpoint.
        click_scrolled(&driver, "recipe-submit").await?;
        wait_for_url_path_prefix(&driver, "/recipe/").await?;
        let detail_url = driver.current_url().await?;
        let id: i64 = detail_url
            .path()
            .rsplit('/')
            .next()
            .and_then(|s| s.parse().ok())
            .context("detail url does not contain a recipe id")?;

        let name_heading = driver.find(By::Css(".detail-name")).await?;
        let shown = name_heading.text().await?;
        anyhow::ensure!(
            shown == "Breton Galette Complète",
            "detail shows '{shown}', expected 'Breton Galette Complète'"
        );
        driver.find(By::Id("hdr-edit")).await?;
        driver.find(By::Id("hdr-delete")).await?;
        let ingredients = driver
            .find(By::Id("ingredient-list"))
            .await?
            .text()
            .await?;
        for expected in ["2 buckwheat galettes", "2 eggs", "60 g gruyère, grated"] {
            anyhow::ensure!(
                ingredients.contains(expected),
                "imported ingredient '{expected}' missing from detail: {ingredients}"
            );
        }
        let instructions = driver
            .find(By::Id("instruction-list"))
            .await?
            .text()
            .await?;
        anyhow::ensure!(
            instructions.contains("Crack an egg onto each."),
            "imported instruction missing from detail: {instructions}"
        );
        // The tagged fixture step must arrive as clean prose: no HTML tags
        // and the decoded apostrophe from "Chef's".
        anyhow::ensure!(
            instructions.contains("Season. Chef's Notes: Rest before serving."),
            "tagged instruction not sanitized: {instructions}"
        );
        anyhow::ensure!(
            !instructions.contains('<'),
            "HTML tags leaked into detail instructions: {instructions}"
        );
        let source_text = driver.find(By::Id("detail-source")).await?.text().await?;
        anyhow::ensure!(
            source_text.contains("galette"),
            "source URL not shown on the detail page: {source_text}"
        );

        let detail = poll_detail_full(&http, &base, id).await?;
        anyhow::ensure!(
            detail.source == format!("http://{fixture_addr}/galette"),
            "imported source URL must be preserved, got '{}'",
            detail.source
        );
        anyhow::ensure!(
            detail.ingredients.len() == 3 && detail.yield_amount == "2 galettes",
            "imported structure incomplete: {detail:?}"
        );
        Ok(())
    })()
    .await;
    let _ = driver.quit().await;
    result
}

/// Meal plan: add a recipe to a day through the picker, see the card, then
/// remove it again.
#[tokio::test(flavor = "multi_thread")]
async fn meal_plan_add_and_remove_flow() -> anyhow::Result<()> {
    let addr = spawn_test_backend().await?;
    let base = format!("http://{addr}");
    let http = reqwest::Client::new();

    // Seed two recipes directly through the API.
    for name in ["Plan Soup", "Plan Pasta"] {
        let status = http
            .post(format!("{base}/api/recipes"))
            .json(&RecipeInput {
                name: name.into(),
                sections: vec![],
                ingredients: vec![],
                instructions: vec![InstructionStep { text: "Cook.".into(), section: None }],
                instruction_sections: vec![],
                notes: String::new(),
                yield_amount: String::new(),
                source: String::new(),
            })
            .send()
            .await?
            .status();
        assert_eq!(status, 201);
    }

    wait_for_port(&addr.to_string()).await?;
    wait_for_port(&webdriver_addr()).await?;
    let driver = open_headless_firefox().await?;
    let result = (|| async {
        driver.goto(format!("{base}/meal-plan")).await?;
        wait_for_url_path(&driver, "/meal-plan").await?;

        // The rolling range renders many days (a week back, two months
        // ahead) — every day is plannable. The wasm client needs a moment
        // to boot, so poll for the first render.
        let mut day_count: i64 = 0;
        for _ in 0..50 {
            day_count = driver
                .execute(
                    "return document.querySelectorAll('.plan-day').length;",
                    Vec::<serde_json::Value>::new(),
                )
                .await?
                .json()
                .as_i64()
                .context("day count not a number")?;
            if day_count > 0 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        // The lazy window starts at 25 sections (3 back, today, 21 ahead)
        // and grows a week at a time as the reader approaches an end.
        anyhow::ensure!(
            day_count >= 20 && day_count <= 40,
            "unexpected initial window: {day_count} sections"
        );

        // Infinite scroll: driving the content scroller to its bottom must
        // extend the range automatically (no buttons).
        driver
            .execute(
                "var c = document.querySelector('.content'); c.scrollTop = c.scrollHeight;",
                Vec::<serde_json::Value>::new(),
            )
            .await?;
        tokio::time::sleep(Duration::from_millis(600)).await;
        let extended = driver
            .execute(
                "return document.querySelectorAll('.plan-day').length;",
                Vec::<serde_json::Value>::new(),
            )
            .await?
            .json().as_i64().context("day count not a number")
            .context("extended day count missing")?;
        anyhow::ensure!(
            extended > day_count,
            "scrolling to the bottom must extend the range: {day_count} -> {extended}"
        );

        // Every rendered day must be unique — duplicated sections were the
        // symptom of broken negative date math in the old range code.
        let unique_labels = driver
            .execute(
                "return Array.from(document.querySelectorAll('.plan-day-label')).filter(l => l.textContent !== 'Today' && l.textContent !== 'Tomorrow').length;",
                Vec::<serde_json::Value>::new(),
            )
            .await?
            .json()
            .as_i64()
            .context("label count missing")?;
        let unique_days = driver
            .execute(
                "var texts = Array.from(document.querySelectorAll('.plan-day-label')).filter(l => l.textContent !== 'Today' && l.textContent !== 'Tomorrow').map(l => l.textContent); return new Set(texts).size;",
                Vec::<serde_json::Value>::new(),
            )
            .await?
            .json()
            .as_i64()
            .context("unique label count missing")?;
        anyhow::ensure!(
            unique_days == unique_labels,
            "duplicate day sections rendered: {unique_days} unique of {unique_labels}"
        );

        // Today's section: open its picker via the labeled section.
        driver
            .find(By::XPath(
                "//div[contains(@class, 'plan-day')][.//span[@class='plan-day-label' and text()='Today']]//button[@class='plan-add']",
            ))
            .await?
            .click()
            .await?;
        driver
            .find(By::Id("plan-search"))
            .await
            .context("picker dialog did not open")?;

        // Search filters the picker rows; pick the soup.
        driver
            .find(By::Id("plan-search"))
            .await?
            .send_keys("Plan Soup")
            .await?;
        let row = driver
            .find(By::XPath(
                "//button[contains(@class, 'plan-picker-row') and contains(., 'Plan Soup')]",
            ))
            .await?;
        row.click().await?;
        tokio::time::sleep(Duration::from_millis(800)).await;

        // The card appears in today's section.
        let card = driver
            .find(By::XPath(
                "//div[contains(@class, 'plan-card') and contains(., 'Plan Soup')]",
            ))
            .await
            .context("planned card did not appear")?;
        let _ = card;

        // The database has the entry for today.
        let entries: Vec<serde_json::Value> = http
            .get(format!("{base}/api/meal-plan"))
            .send()
            .await?
            .json()
            .await?;
        anyhow::ensure!(
            entries.len() == 1,
            "expected 1 meal plan entry, got {entries:?}"
        );

        // Remove it via the card's × button.
        driver
            .find(By::Css(".plan-card .plan-remove"))
            .await?
            .click()
            .await?;
        tokio::time::sleep(Duration::from_millis(800)).await;
        let entries: Vec<serde_json::Value> = http
            .get(format!("{base}/api/meal-plan"))
            .send()
            .await?
            .json()
            .await?;
        anyhow::ensure!(
            entries.is_empty(),
            "meal plan entry must be gone: {entries:?}"
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
    // Header bar: all actions wired, no placeholders left.
    driver.find(By::Id("hdr-back")).await?;
    driver.find(By::Id("hdr-share")).await?;
    driver.find(By::Id("hdr-mealplan")).await?;
    driver.find(By::Id("hdr-cart")).await?;
    driver.find(By::Id("hdr-edit")).await?;
    driver.find(By::Id("hdr-delete")).await?;
    let placeholders = driver
        .find_all(By::Css(".detail-header .hdr-btn.ph"))
        .await?;
    anyhow::ensure!(
        placeholders.is_empty(),
        "detail header still has {} placeholder buttons",
        placeholders.len()
    );
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

/// Poll `GET /api/grocery` until `name` appears at least `expected` times.
async fn poll_grocery_count(
    http: &reqwest::Client,
    base: &str,
    name: &str,
    expected: usize,
) -> anyhow::Result<()> {
    for _ in 0..50 {
        let items: Vec<GroceryItem> = http
            .get(format!("{base}/api/grocery"))
            .send()
            .await?
            .json()
            .await?;
        if items.iter().filter(|i| i.name == name).count() >= expected {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("grocery item '{name}' never appeared {expected} times")
}

/// Poll `GET /api/grocery` until `name` is gone from the database.
async fn poll_grocery_gone(http: &reqwest::Client, base: &str, name: &str) -> anyhow::Result<()> {
    for _ in 0..50 {
        let items: Vec<GroceryItem> = http
            .get(format!("{base}/api/grocery"))
            .send()
            .await?
            .json()
            .await?;
        if !items.iter().any(|i| i.name == name) {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    anyhow::bail!("grocery item '{name}' never disappeared from the database")
}

/// Texts of the grocery rows currently rendered in the shopping tab.
async fn grocery_item_texts(driver: &WebDriver) -> anyhow::Result<Vec<String>> {
    let rows = driver
        .find_all(By::Css("#grocery-list li.grocery-item"))
        .await?;
    let mut texts = Vec::new();
    for row in rows {
        texts.push(row.text().await?.replace('\n', " "));
    }
    Ok(texts)
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
