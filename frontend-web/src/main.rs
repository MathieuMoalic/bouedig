//! Bouedig web client (Dioxus + WASM).

use std::collections::HashSet;

use dioxus::prelude::*;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsCast;
use shared::{
    GroceryItem, Ingredient, InstructionStep, MealPlanEntry, NewGroceryBatch, NewGroceryItem,
    Recipe, RecipeDetail as RecipeDetailModel, RecipeInput,
};

const WALLPAPER: Asset = asset!("/assets/background.avif");
const FAVICON: Asset = asset!("/assets/icon.png");

fn main() {
    // Set up logging and panic reporting first, so that anything that goes
    // wrong afterwards (including panics) is visible in the browser console.
    console_error_panic_hook::set_once();
    tracing_wasm::set_as_global_default();
    tracing::info!("Bouedig web client starting");
    dioxus::launch(App);
}

#[derive(Clone, Routable, Debug, PartialEq)]
enum Route {
    #[layout(Layout)]
    #[route("/")]
    Recipes {},
    #[route("/add")]
    AddRecipe {},
    #[route("/import")]
    ImportRecipe {},
    #[route("/recipe/:id")]
    RecipeDetail { id: i64 },
    #[route("/edit/:id")]
    EditRecipe { id: i64 },
    #[route("/grocery")]
    Grocery {},
    #[route("/meal-plan")]
    MealPlan {},
    #[route("/settings")]
    Settings {},
}

#[component]
fn App() -> Element {
    rsx! {
        Router::<Route> {}
    }
}

// ---------------------------------------------------------------------------
// App shell: wallpaper + content + bottom navigation
// ---------------------------------------------------------------------------

#[component]
fn Layout() -> Element {
    rsx! {
        style { {include_str!("../assets/style.css")} }
        document::Title { "Bouedig" }
        document::Link { rel: "icon", r#type: "image/png", href: FAVICON }
        div { class: "app",
            div { class: "wallpaper", style: "background-image: url('{WALLPAPER}')" }
            main { class: "content",
                id: "content-scroller",
                Outlet::<Route> {}
            }
            BottomNav {}
        }
    }
}

#[component]
fn BottomNav() -> Element {
    let route: Route = use_route();
    let tabs: [(Route, &str, Icons); 4] = [
        (Route::Recipes {}, "Recipes", Icons::Recipes),
        (Route::MealPlan {}, "Meal plan", Icons::MealPlan),
        (Route::Grocery {}, "Shopping", Icons::Shopping),
        (Route::Settings {}, "Settings", Icons::Settings),
    ];

    rsx! {
        nav { id: "bottom-nav", class: "bottom-nav",
            for (target, label, icon) in tabs {
                NavTab { target: target.clone(), label: label, icon: icon, active: route == target }
            }
        }
    }
}

#[component]
fn NavTab(target: Route, label: String, icon: Icons, active: bool) -> Element {
    rsx! {
        Link {
            to: target,
            class: if active { "nav-tab active" } else { "nav-tab" },
            TabIcon { icon: icon }
            span { class: "tab-label", "{label}" }
        }
    }
}

#[component]
fn TabIcon(icon: Icons) -> Element {
    icon.render()
}

/// Inline SVG icons (stroke-based, 24x24 viewBox).
#[derive(Clone, Copy, PartialEq)]
enum Icons {
    Recipes,
    MealPlan,
    Shopping,
    Settings,
}

impl Icons {
    fn render(self) -> Element {
        rsx! {
            match self {
                Icons::Recipes => rsx! {
                    svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.8", "stroke-linecap": "round",
                        path { d: "M7 3v6.5M10 3v6.5M8.5 3v18M8.5 9.5a1.5 1.5 0 0 1-3 0" }
                        path { d: "M15.5 3c2 1.5 3 3.5 3 6v12M15.5 3v18" }
                    }
                },
                Icons::MealPlan => rsx! {
                    svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.8", "stroke-linecap": "round",
                        rect { x: "3", y: "5", width: "18", height: "16", rx: "2" }
                        path { d: "M3 10h18M8 3v4M16 3v4M7.5 14h3M13.5 14h3M7.5 17.5h3" }
                    }
                },
                Icons::Shopping => rsx! {
                    svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.8", "stroke-linecap": "round", "stroke-linejoin": "round",
                        path { d: "M2.5 4h2.6l2.5 11.5h11l2.4-8.5H6.2" }
                        circle { cx: "9", cy: "20", r: "1.4" }
                        circle { cx: "17", cy: "20", r: "1.4" }
                    }
                },
                Icons::Settings => rsx! {
                    svg { class: "icon", view_box: "0 0 24 24", fill: "currentColor",
                        path { d: "M19.14 12.94c.04-.3.06-.61.06-.94 0-.32-.02-.64-.07-.94l2.03-1.58c.18-.14.23-.41.12-.61l-1.92-3.32c-.12-.22-.37-.29-.59-.22l-2.39.96c-.5-.38-1.03-.7-1.62-.94l-.36-2.54c-.04-.24-.24-.41-.48-.41h-3.84c-.24 0-.43.17-.47.41l-.36 2.54c-.59.24-1.13.57-1.62.94l-2.39-.96c-.22-.08-.47 0-.59.22L2.74 8.87c-.12.21-.08.47.12.61l2.03 1.58c-.05.3-.09.63-.09.94s.02.64.07.94l-2.03 1.58c-.18.14-.23.41-.12.61l1.92 3.32c.12.22.37.29.59.22l2.39-.96c.5.38 1.03.7 1.62.94l.36 2.54c.05.24.24.41.48.41h3.84c.24 0 .44-.17.47-.41l.36-2.54c.59-.24 1.13-.56 1.62-.94l2.39.96c.22.08.47 0 .59-.22l1.92-3.32c.12-.22.07-.47-.12-.61l-2.03-1.58zM12 15.6c-1.98 0-3.6-1.62-3.6-3.6s1.62-3.6 3.6-3.6 3.6 1.62 3.6 3.6-1.62 3.6-3.6 3.6z" }
                    }
                },
            }
        }
    }
}

// --- small icons used across pages -----------------------------------------

#[component]
fn IconPlus() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round",
            path { d: "M12 5v14M5 12h14" }
        }
    }
}

#[component]
fn IconSearch() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round",
            circle { cx: "11", cy: "11", r: "6.5" }
            path { d: "M16 16l5 5" }
        }
    }
}

/// Globe icon for the "import recipe from URL" action.
#[component]
fn IconGlobe() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round",
            circle { cx: "12", cy: "12", r: "8.5" }
            path { d: "M3.5 12h17M12 3.5c2.6 2.3 4 5.2 4 8.5s-1.4 6.2-4 8.5c-2.6-2.3-4-5.2-4-8.5s1.4-6.2 4-8.5z" }
        }
    }
}

#[component]
fn IconList() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round",
            path { d: "M4 7h16M4 12h16M4 17h16" }
        }
    }
}

#[component]
fn IconMenu() -> Element {
    rsx! {
        svg { class: "icon small", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round",
            path { d: "M4 7h16M4 12h16M4 17h10" }
        }
    }
}

#[component]
fn IconChevron(down: bool) -> Element {
    rsx! {
        svg { class: "icon small", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round",
            path { d: if down { "M6 9l6 6 6-6" } else { "M9 6l6 6-6 6" } }
        }
    }
}

#[component]
fn IconCalendar() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.8", "stroke-linecap": "round",
            rect { x: "3", y: "5", width: "18", height: "16", rx: "2" }
            path { d: "M3 10h18M8 3v4M16 3v4" }
        }
    }
}

#[component]
fn IconBack() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round", "stroke-linejoin": "round",
            path { d: "M19 12H5M11 18l-6-6 6-6" }
        }
    }
}

#[component]
fn IconTimer() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.8", "stroke-linecap": "round",
            circle { cx: "12", cy: "13", r: "7.5" }
            path { d: "M12 9.5V13l2.5 2M9.5 2.5h5M12 2.5V5" }
        }
    }
}

#[component]
fn IconShare() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.8", "stroke-linecap": "round",
            circle { cx: "6", cy: "12", r: "2.6" }
            circle { cx: "18", cy: "6", r: "2.6" }
            circle { cx: "18", cy: "18", r: "2.6" }
            path { d: "M8.3 10.8l7.4-3.6M8.3 13.2l7.4 3.6" }
        }
    }
}

#[component]
fn IconCart() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.8", "stroke-linecap": "round", "stroke-linejoin": "round",
            path { d: "M2.5 4h2.6l2.5 11.5h11l2.4-8.5H6.2" }
            circle { cx: "9", cy: "20", r: "1.4" }
            circle { cx: "17", cy: "20", r: "1.4" }
        }
    }
}

#[component]
fn IconPencil() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.8", "stroke-linecap": "round", "stroke-linejoin": "round",
            path { d: "M4 20h4l11-11-4-4L4 16v4zM13.5 5.5l4 4" }
        }
    }
}

#[component]
fn IconTrash() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.8", "stroke-linecap": "round",
            path { d: "M4 7h16M9 7V4h6v3M6.5 7l1 13h9l1-13M10 11v6M14 11v6" }
        }
    }
}

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

/// The browser origin, e.g. `http://localhost:8788` in dev (behind the dx
/// proxy) or the production origin behind the reverse proxy. reqwest on
/// wasm cannot build relative URLs, so we absolutise every path.
fn api_base() -> String {
    web_sys::window()
        .expect("no window")
        .location()
        .origin()
        .expect("no origin")
}

async fn api_get<T: DeserializeOwned>(path: &str) -> anyhow::Result<T> {
    let url = format!("{}{}", api_base(), path);
    tracing::info!("GET {url}");
    let resp = reqwest::get(&url)
        .await
        .map_err(|err| {
            tracing::error!("GET {url} request failed: {err:#}");
            err
        })?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        tracing::error!("GET {url} failed: {status} ({body})");
        anyhow::bail!("GET {url} failed: {status} ({body})");
    }
    // Result parsing is logged: a malformed payload must never be silent.
    match serde_json::from_str(&body) {
        Ok(parsed) => {
            tracing::debug!("GET {url} ok ({status})");
            Ok(parsed)
        }
        Err(err) => {
            tracing::error!("GET {url}: failed to parse response body: {err} (body: {body})");
            Err(err.into())
        }
    }
}

/// Build a `multipart/form-data` body by hand (reqwest's multipart module is
/// not available on wasm). Returns the `Content-Type` header value and body.
fn build_multipart(
    name: &str,
    sections_json: &str,
    ingredients_json: &str,
    instructions_json: &str,
    instruction_sections_json: &str,
    notes: &str,
    yield_amount: &str,
    source: &str,
    image: Option<(&str, &[u8])>,
    image_url: Option<&str>,
) -> (String, Vec<u8>) {
    // `SystemTime::now` is not implemented on wasm, use the JS clock.
    let boundary = format!("bouedig{}", js_sys::Date::now() as u64);
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"sections\"\r\n\r\n{sections_json}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"ingredients\"\r\n\r\n{ingredients_json}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"instructions\"\r\n\r\n{instructions_json}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"instruction_sections\"\r\n\r\n{instruction_sections_json}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"notes\"\r\n\r\n{notes}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"yield\"\r\n\r\n{yield_amount}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"source\"\r\n\r\n{source}\r\n").as_bytes());
    if let Some(url) = image_url {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"image_url\"\r\n\r\n{url}\r\n").as_bytes());
    }
    if let Some((filename, bytes)) = image {
        let mime = match filename.rsplit('.').next().unwrap_or_default() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            _ => "application/octet-stream",
        };
        body.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"{filename}\"\r\nContent-Type: {mime}\r\n\r\n").as_bytes(),
        );
        body.extend_from_slice(bytes);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

// ---------------------------------------------------------------------------
// Tab: Recipes (photo grid)
// ---------------------------------------------------------------------------

#[component]
fn Recipes() -> Element {
    let mut recipes = use_signal(Vec::<Recipe>::new);
    let mut error = use_signal(|| String::new());
    let mut loaded = use_signal(|| false);
    let navigator = use_navigator();

    use_effect(move || {
        if !loaded() {
            loaded.set(true);
            spawn(async move {
                match api_get::<Vec<Recipe>>("/api/recipes").await {
                    Ok(list) => {
                        tracing::debug!("recipes refreshed: {} items", list.len());
                        recipes.set(list);
                        error.set(String::new());
                    }
                    Err(err) => {
                        tracing::error!("recipes refresh failed: {err:#}");
                        error.set(err.to_string());
                    }
                }
            });
        }
    });

    let list = recipes.read().clone();

    rsx! {
        div { class: "page",
            if list.is_empty() {
                p { class: "empty", "No recipes yet. Tap + to add your first one." }
            }
            div { id: "recipe-grid", class: "recipe-grid",
                for recipe in list {
                    RecipeCard { key: "{recipe.id}", recipe: recipe }
                }
            }
            div { class: "fab-stack",
                button { class: "fab small", title: "Search", IconSearch {} }
                button {
                    id: "fab-import-recipe",
                    class: "fab small",
                    title: "Import from URL",
                    onclick: move |_| {
                        tracing::info!("import FAB clicked, opening the import form");
                        navigator.push(Route::ImportRecipe {});
                    },
                    IconGlobe {}
                }
                button {
                    id: "fab-add-recipe",
                    class: "fab",
                    title: "Add recipe",
                    onclick: move |_| {
                        tracing::info!("+ FAB clicked, opening the add-recipe form");
                        navigator.push(Route::AddRecipe {});
                    },
                    IconPlus {}
                }
            }
        }
    }
}

#[component]
fn RecipeCard(recipe: Recipe) -> Element {
    let navigator = use_navigator();
    let initials: String = recipe
        .name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    rsx! {
        div {
            class: "recipe-card clickable",
            id: "recipe-card-{recipe.id}",
            role: "button",
            tabindex: "0",
            onclick: move |_| {
                tracing::info!("recipe card {} clicked, opening detail", recipe.id);
                navigator.push(Route::RecipeDetail { id: recipe.id });
            },
            div { class: "recipe-card-media",
                if let Some(thumb) = &recipe.thumb {
                    img { src: "{thumb}", loading: "lazy", alt: "{recipe.name}" }
                } else {
                    div { class: "recipe-placeholder", "{initials}" }
                }
                button { class: "card-fab", title: "Add to meal plan", onclick: move |e: MouseEvent| e.stop_propagation(), IconCalendar {} }
            }
            div { class: "recipe-card-name", "{recipe.name}" }
        }
    }
}

// ---------------------------------------------------------------------------
// Tab: Add / Edit recipe (shared structured form)
// ---------------------------------------------------------------------------

/// A named ingredient section (e.g. "Crêpes", "Filling").
#[derive(Clone, PartialEq)]
struct SectionRow {
    id: u64,
    name: String,
}

/// One ingredient in the editor list; fields are only changed through the
/// ingredient modal, so rows are plain display data.
#[derive(Clone, PartialEq)]
struct IngredientRow {
    id: u64,
    section_id: Option<u64>,
    name: String,
    quantity: String,
    unit: String,
    prep: String,
}

#[derive(Clone, PartialEq)]
struct StepRow {
    id: u64,
    section_id: Option<u64>,
    text: String,
}

/// Which modal is open (if any). Field values live inside the variant.
#[derive(Clone, PartialEq)]
enum Modal {
    Ingredient {
        section_id: Option<u64>,
        editing_row: Option<u64>,
        qty: String,
        unit: String,
        name: String,
        prep: String,
    },
    Section {
        editing_section: Option<u64>,
        name: String,
        /// true = an instruction section, false = an ingredient section.
        is_step: bool,
    },
    Step {
        editing_step: Option<u64>,
        section_id: Option<u64>,
        text: String,
    },
}

#[derive(Clone, Copy, PartialEq)]
enum DragKind {
    Ingredient,
    Section,
    Step,
    StepSection,
}

#[derive(Clone, PartialEq)]
struct DragState {
    kind: DragKind,
    id: u64,
    start_y: f64,
    row_height: f64,
    applied: f64,
}

/// Move an item one slot down/up among its siblings (same section).
fn swap_ingredient(rows: &mut [IngredientRow], id: u64, down: bool) -> bool {
    let Some(index) = rows.iter().position(|r| r.id == id) else {
        return false;
    };
    let Some(section_id) = rows.get(index).map(|r| r.section_id) else {
        return false;
    };
    let mut neighbor = index as i64 + if down { 1 } else { -1 };
    // Only swap within the same section.
    while (0..rows.len() as i64).contains(&neighbor)
        && rows[neighbor as usize].section_id != section_id
    {
        neighbor += if down { 1 } else { -1 };
    }
    if !(0..rows.len() as i64).contains(&neighbor) {
        return false;
    }
    rows.swap(index, neighbor as usize);
    true
}

fn swap_section(sections: &mut [SectionRow], id: u64, down: bool) -> bool {
    let Some(index) = sections.iter().position(|s| s.id == id) else {
        return false;
    };
    let neighbor = index as i64 + if down { 1 } else { -1 };
    if !(0..sections.len() as i64).contains(&neighbor) {
        return false;
    }
    sections.swap(index, neighbor as usize);
    true
}

fn swap_step(steps: &mut [StepRow], id: u64, down: bool) -> bool {
    let Some(index) = steps.iter().position(|s| s.id == id) else {
        return false;
    };
    let neighbor = index as i64 + if down { 1 } else { -1 };
    if !(0..steps.len() as i64).contains(&neighbor) {
        return false;
    }
    steps.swap(index, neighbor as usize);
    true
}

/// Format a quantity without trailing zeros (200.0 -> "200", 1.5 -> "1.5").
fn fmt_qty(q: f64) -> String {
    let s = format!("{q}");
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    }
}

#[component]
fn AddRecipe() -> Element {
    rsx! {
        RecipeForm { editing: None }
    }
}

#[component]
fn EditRecipe(id: i64) -> Element {
    rsx! {
        RecipeForm { editing: Some(id) }
    }
}

// ---------------------------------------------------------------------------
// Page: Import recipe from a URL
// ---------------------------------------------------------------------------

/// Mirror of the backend's `RecipePreview` (kept local: importer-specific
/// API types do not belong in `shared` until another client needs them).
#[derive(Debug, Clone, serde::Deserialize)]
struct ImportPreview {
    recipe: RecipeInput,
    method: String,
    confidence: f32,
    warnings: Vec<String>,
    /// Absolute image URL from the source page; downloaded by the backend
    /// only when the recipe is saved.
    #[serde(default)]
    image_url: Option<String>,
}

/// Where a warning can take the user in the form below.
#[derive(Clone, Debug, PartialEq)]
enum JumpTarget {
    /// The ingredient rows whose text contains each name.
    IngredientRows(Vec<String>),
    /// A form element by id.
    Field(&'static str),
}

/// The ingredient names a warning quotes ("…parsed: \"salt\", \"pepper\"").
fn quoted_names(warning: &str) -> Vec<String> {
    warning.split('"').skip(1).step_by(2).map(str::to_string).collect()
}

/// Map a warning to the form element(s) that let the user fix it.
/// `None` = informational only (nothing to jump to).
fn jump_target_for(warning: &str, ingredients: &[Ingredient]) -> Option<JumpTarget> {
    if warning.contains("could not be parsed") {
        // The warning names the lines it could not parse; those are the rows
        // to highlight. Fall back to every quantity-less row if extraction
        // ever comes up empty.
        let mut names = quoted_names(warning);
        if names.is_empty() {
            names = ingredients
                .iter()
                .filter(|i| i.quantity.is_none())
                .map(|i| i.name.clone())
                .collect();
        }
        return (!names.is_empty()).then_some(JumpTarget::IngredientRows(names));
    }
    if warning.contains("no yield") {
        return Some(JumpTarget::Field("recipe-yield"));
    }
    if warning.contains("no instructions") {
        return Some(JumpTarget::Field("step-rows"));
    }
    if warning.contains("no ingredients") {
        return Some(JumpTarget::Field("ingredient-rows"));
    }
    if warning.contains("no name") {
        return Some(JumpTarget::Field("recipe-name"));
    }
    None
}

/// Scroll to and flash whatever the warning points at. The flash is a CSS
/// animation (no timer needed); rewriting the class attribute after a reflow
/// makes repeated clicks restart it.
fn do_jump(target: &JumpTarget) {
    let Some(document) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let flash = |element: &web_sys::Element| {
        let existing = element.get_attribute("class").unwrap_or_default();
        let stripped = existing
            .split_whitespace()
            .filter(|class| *class != "row-flash")
            .collect::<Vec<_>>()
            .join(" ");
        element
            .set_attribute("class", &stripped)
            .expect("class attribute");
        if let Ok(html) = element.clone().dyn_into::<web_sys::HtmlElement>() {
            // Reading a layout property forces the reflow.
            let _ = html.offset_width();
        }
        let updated = if stripped.is_empty() {
            "row-flash".to_string()
        } else {
            format!("{stripped} row-flash")
        };
        element
            .set_attribute("class", &updated)
            .expect("class attribute");
    };
    match target {
        JumpTarget::Field(id) => {
            if let Some(element) = document.get_element_by_id(id) {
                flash(&element);
                element.scroll_into_view();
            }
        }
        JumpTarget::IngredientRows(names) => {
            let Ok(rows) = document.query_selector_all(".ingredient-row") else {
                return;
            };
            let mut first: Option<web_sys::Element> = None;
            for i in 0..rows.length() {
                let Some(Ok(row)) =
                    rows.get(i).map(|node| node.dyn_into::<web_sys::Element>())
                else {
                    continue;
                };
                let text = row.text_content().unwrap_or_default();
                if names.iter().any(|name| text.contains(name)) {
                    flash(&row);
                    first.get_or_insert(row);
                }
            }
            if let Some(row) = first {
                row.scroll_into_view();
            }
        }
    }
}

/// URL → fetch preview → review/edit in the existing recipe form → save.
/// The imported recipe is never saved without an explicit user action.
#[component]
fn ImportRecipe() -> Element {
    let mut url = use_signal(String::new);
    let mut loading = use_signal(|| false);
    let mut error = use_signal(|| String::new());
    let mut preview = use_signal(|| None::<ImportPreview>);
    let mut show_warnings = use_signal(|| true);

    let mut import = move |_| {
        let target = url.read().trim().to_string();
        if target.is_empty() || *loading.read() {
            return;
        }
        loading.set(true);
        error.set(String::new());
        let target = target.clone();
        spawn(async move {
            let client = reqwest::Client::new();
            let endpoint = format!("{}/api/recipes/import", api_base());
            tracing::info!("importing recipe from {target}");
            let result = client
                .post(endpoint)
                .json(&serde_json::json!({ "url": target }))
                .send()
                .await;
            match result {
                Ok(resp) if resp.status().is_success() => match resp.json::<ImportPreview>().await {
                    Ok(p) => {
                        tracing::info!(
                            "import preview: {} (method {}, confidence {:.2})",
                            p.recipe.name, p.method, p.confidence
                        );
                        show_warnings.set(true);
                        preview.set(Some(p));
                    }
                    Err(err) => {
                        tracing::error!("import response unreadable: {err:#}");
                        error.set("The server returned an unreadable preview.".into());
                    }
                },
                Ok(resp) => {
                    let status = resp.status();
                    let message = resp.text().await.unwrap_or_default();
                    tracing::error!("import failed: {status} {message}");
                    let message = serde_json::from_str::<serde_json::Value>(&message)
                        .ok()
                        .and_then(|v| v["error"].as_str().map(str::to_string))
                        .unwrap_or(message);
                    error.set(message);
                }
                Err(err) => {
                    tracing::error!("import request failed: {err:#}");
                    error.set("Could not reach the server.".into());
                }
            }
            loading.set(false);
        });
    };

    let preview_snapshot = preview.read().clone();
    let confidence_pct = preview_snapshot
        .as_ref()
        .map(|p| (p.confidence * 100.0) as u64)
        .unwrap_or(0);
    match preview_snapshot {
        None => rsx! {
            div { class: "page",
                div { class: "card import-card",
                    h1 { "Import recipe" }
                    p { class: "muted",
                        "Paste the web address of a recipe page. Bouedig reads the \
                         page, extracts the recipe and lets you review it before saving."
                    }
                    div { class: "import-row",
                        input {
                            id: "import-url",
                            r#type: "url",
                            placeholder: "https://…",
                            value: "{url}",
                            autocomplete: "off",
                            oninput: move |e: FormEvent| url.set(e.value()),
                            onkeydown: move |e: KeyboardEvent| {
                                if e.key() == Key::Enter {
                                    import(());
                                }
                            },
                        }
                        button {
                            id: "import-fetch",
                            class: "btn-primary",
                            r#type: "button",
                            disabled: *loading.read(),
                            onclick: move |_| import(()),
                            if *loading.read() { "Fetching…" } else { "Import" }
                        }
                    }
                    if !error.read().is_empty() {
                        p { class: "status-error", "{error}" }
                    }
                    if *loading.read() {
                        p { class: "muted", "Fetching the recipe page…" }
                    }
                }
            }
        },
        Some(p) => {
            let warning_items: Vec<(&String, Option<JumpTarget>)> = p
                .warnings
                .iter()
                .map(|warning| (warning, jump_target_for(warning, &p.recipe.ingredients)))
                .collect();
            rsx! {
            div { class: "page",
                div { class: "card import-summary", id: "import-summary",
                    if let Some(image) = &p.image_url {
                        img { class: "import-photo", src: "{image}", alt: "{p.recipe.name}", referrerpolicy: "no-referrer" }
                    }
                    div { class: "import-summary-head",
                        span { class: "import-badge", "{p.method}" }
                        span { class: "muted", "confidence {confidence_pct}%" }
                    }
                    if show_warnings() && !p.warnings.is_empty() {
                        div { class: "import-warnings",
                            button {
                                id: "import-warnings-dismiss",
                                class: "warnings-dismiss",
                                r#type: "button",
                                title: "Dismiss",
                                onclick: move |_| show_warnings.set(false),
                                IconX {}
                            }
                            p { "The extraction was incomplete — tap a warning to jump to it:" }
                            ul { class: "warning-list",
                                for (warning, target) in warning_items {
                                    li {
                                        if let Some(target) = target {
                                            button {
                                                class: "warning-jump",
                                                r#type: "button",
                                                title: "Show it in the form",
                                                onclick: move |_| do_jump(&target),
                                                span { class: "warning-text", "{warning}" }
                                                span { class: "warning-arrow", "↓" }
                                            }
                                        } else {
                                            span { class: "warning-text", "{warning}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    button {
                        id: "import-restart",
                        class: "btn-ghost",
                        r#type: "button",
                        onclick: move |_| {
                            preview.set(None);
                            url.set(String::new());
                        },
                        "Import a different URL"
                    }
                }
                // Review/edit in the existing editor, then save explicitly.
                RecipeFormFields {
                    initial: import_preview_to_detail(&p),
                    initial_image_url: p.image_url.clone(),
                    editing_id: None,
                }
            }
            }
        },
    }
}

/// Adapt the imported `RecipeInput` into the editor's `initial` value (the
/// editor consumes the detail shape; ids/photos are meaningless here).
fn import_preview_to_detail(preview: &ImportPreview) -> RecipeDetailModel {
    let recipe = &preview.recipe;
    RecipeDetailModel {
        id: 0,
        name: recipe.name.clone(),
        sections: recipe.sections.clone(),
        ingredients: recipe.ingredients.clone(),
        instructions: recipe.instructions.clone(),
        instruction_sections: recipe.instruction_sections.clone(),
        notes: recipe.notes.clone(),
        yield_amount: recipe.yield_amount.clone(),
        source: recipe.source.clone(),
        image: None,
        thumb: None,
    }
}

#[component]
fn RecipeForm(editing: Option<i64>) -> Element {
    let mut detail = use_signal(|| None::<RecipeDetailModel>);
    let mut load_error = use_signal(|| String::new());

    // Load the existing recipe when editing; create mode renders immediately.
    use_effect(move || {
        let Some(id) = editing else { return };
        spawn(async move {
            match api_get::<RecipeDetailModel>(&format!("/api/recipes/{id}")).await {
                Ok(d) => detail.set(Some(d)),
                Err(err) => {
                    tracing::error!("failed to load recipe {id} for editing: {err:#}");
                    load_error.set(err.to_string());
                }
            }
        });
    });

    let loaded = detail.read().clone();
    match (editing, loaded) {
        (None, _) => rsx! {
            RecipeFormFields { initial: RecipeDetailModel::default(), initial_image_url: None, editing_id: None }
        },
        (Some(_), Some(initial)) => rsx! {
            RecipeFormFields { initial: initial, initial_image_url: None, editing_id: editing }
        },
        (Some(_), None) => rsx! {
            if load_error.read().is_empty() {
                p { class: "empty", "Loading…" }
            } else {
                p { class: "status-error", "{load_error}" }
            }
        },
    }
}

// ---------------------------------------------------------------------------
// Modal + drag helpers
// ---------------------------------------------------------------------------

/// Measure a row's rendered height by its element id (wasm only).
fn measure_row_height(element_id: &str) -> f64 {
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(element_id))
            .map(|el| el.get_bounding_client_rect().height() as f64)
            .filter(|h| *h > 0.0)
            .unwrap_or(44.0)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        let _ = element_id;
        44.0
    }
}

/// Amount column of an ingredient row, e.g. `180 g` or `–` when missing.
fn amount_text(row: &IngredientRow) -> String {
    let qty = row.quantity.trim();
    let unit = row.unit.trim();
    if qty.is_empty() && unit.is_empty() {
        return "\u{2013}".to_string();
    }
    let mut text = String::new();
    text.push_str(qty);
    if !qty.is_empty() && !unit.is_empty() {
        text.push(' ');
    }
    text.push_str(unit);
    text
}

#[component]
fn IconX() -> Element {
    rsx! {
        svg { class: "icon small", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round",
            path { d: "M6 6l12 12M18 6l-6 6-6-6M6 6l12 12M18 6L6 18" }
        }
    }
}

#[component]
fn IconHandle() -> Element {
    rsx! {
        svg { class: "icon small", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "2", "stroke-linecap": "round",
            path { d: "M4 9h16M4 15h16" }
        }
    }
}

#[component]
fn RecipeFormFields(
    initial: RecipeDetailModel,
    initial_image_url: Option<String>,
    editing_id: Option<i64>,
) -> Element {
    // One shared id counter so element ids never collide across lists.
    let sec_count = initial.sections.len() as u64;
    let ing_count = initial.ingredients.len() as u64;
    let step_count = initial.instructions.len() as u64;

    let mut name = use_signal(|| initial.name.clone());
    let mut sections = use_signal(|| {
        initial
            .sections
            .iter()
            .enumerate()
            .map(|(i, s)| SectionRow { id: i as u64, name: s.clone() })
            .collect::<Vec<_>>()
    });
    let mut rows = use_signal(|| {
        let sec_base = sec_count;
        initial
            .ingredients
            .iter()
            .enumerate()
            .map(|(i, ing)| IngredientRow {
                id: sec_base + i as u64,
                section_id: ing.section.as_deref().and_then(|s| {
                    initial
                        .sections
                        .iter()
                        .position(|sec| sec == s)
                        .map(|p| p as u64)
                }),
                name: ing.name.clone(),
                quantity: ing.quantity.map(fmt_qty).unwrap_or_default(),
                unit: ing.unit.clone().unwrap_or_default(),
                prep: ing.prep.clone().unwrap_or_default(),
            })
            .collect::<Vec<_>>()
    });
    let mut steps = use_signal(|| {
        let step_base = sec_count + ing_count;
        let istep_base = step_base + step_count;
        initial
            .instructions
            .iter()
            .enumerate()
            .map(|(i, step)| StepRow {
                id: step_base + i as u64,
                section_id: step.section.as_deref().and_then(|s| {
                    initial
                        .instruction_sections
                        .iter()
                        .position(|sec| sec == s)
                        .map(|p| istep_base + p as u64)
                }),
                text: step.text.clone(),
            })
            .collect::<Vec<_>>()
    });
    let mut step_sections = use_signal(|| {
        let istep_base = sec_count + ing_count + step_count;
        initial
            .instruction_sections
            .iter()
            .enumerate()
            .map(|(i, s)| SectionRow { id: istep_base + i as u64, name: s.clone() })
            .collect::<Vec<_>>()
    });
    let mut next_id =
        use_signal(|| sec_count + ing_count + step_count + initial.instruction_sections.len() as u64);
    let mut yield_amount = use_signal(|| initial.yield_amount.clone());
    let mut source = use_signal(|| initial.source.clone());
    let mut notes = use_signal(|| initial.notes.clone());
    let mut modal = use_signal(|| None::<Modal>);
    let mut drag = use_signal(|| None::<DragState>);
    let mut suppress_click = use_signal(|| false);
    let mut photo = use_signal(|| None::<(String, Vec<u8>)>);
    let mut import_image_url = use_signal(|| initial_image_url);
    let mut status = use_signal(|| String::new());
    let mut status_error = use_signal(|| false);
    let navigator = use_navigator();

    let existing_thumb = initial.thumb.clone();
    let editing = editing_id.is_some();

    // Snapshots for this render.
    let sections_snapshot = sections.read().clone();
    let rows_snapshot = rows.read().clone();
    let steps_snapshot = steps.read().clone();
    let step_sections_snapshot = step_sections.read().clone();
    let drag_snapshot = drag.read().clone();
    let modal_snapshot = modal.read().clone();
    let name_value = name.read().clone();
    let yield_value = yield_amount.read().clone();
    let source_value = source.read().clone();
    let notes_value = notes.read().clone();

    // Live quantity validation for the open ingredient modal: empty is
    // allowed ("–"); otherwise it must parse as a positive, finite number.
    let qty_valid = match &modal_snapshot {
        Some(Modal::Ingredient { qty, .. }) => {
            let qty = qty.trim();
            qty.is_empty()
                || qty
                    .parse::<f64>()
                    .ok()
                    .is_some_and(|q| q.is_finite() && q > 0.0)
        }
        _ => true,
    };

    // Instruction steps grouped for display: unsectioned steps first, then
    // every instruction section in order.
    let mut step_groups: Vec<(Option<SectionRow>, Vec<StepRow>)> = Vec::new();
    let plain_steps: Vec<StepRow> = steps_snapshot
        .iter()
        .filter(|s| s.section_id.is_none())
        .cloned()
        .collect();
    if !plain_steps.is_empty() {
        step_groups.push((None, plain_steps));
    }
    for section in &step_sections_snapshot {
        step_groups.push((
            Some(section.clone()),
            steps_snapshot
                .iter()
                .filter(|s| s.section_id == Some(section.id))
                .cloned()
                .collect(),
        ));
    }
    // Continuous step numbering across all groups.
    let mut step_number = 0usize;
    let numbered_step_groups: Vec<(Option<SectionRow>, Vec<(usize, StepRow)>)> = step_groups
        .into_iter()
        .map(|(section, group)| {
            let numbered = group
                .into_iter()
                .map(|step| {
                    step_number += 1;
                    (step_number, step)
                })
                .collect();
            (section, numbered)
        })
        .collect();

    // Ingredients grouped for display: unsectioned rows first, then every
    // section in order (empty sections still show their header).
    let mut groups: Vec<(Option<SectionRow>, Vec<(IngredientRow, String)>)> = Vec::new();
    let plain: Vec<(IngredientRow, String)> = rows_snapshot
        .iter()
        .filter(|r| r.section_id.is_none())
        .cloned()
        .map(|row| {
            let amount = amount_text(&row);
            (row, amount)
        })
        .collect();
    if !plain.is_empty() {
        groups.push((None, plain));
    }
    for section in &sections_snapshot {
        groups.push((
            Some(section.clone()),
            rows_snapshot
                .iter()
                .filter(|r| r.section_id == Some(section.id))
                .cloned()
                .map(|row| {
                    let amount = amount_text(&row);
                    (row, amount)
                })
                .collect(),
        ));
    }

    rsx! {
        div {
            class: "page",
            onpointermove: move |e: PointerEvent| {
                let Some(mut d) = drag.read().clone() else { return };
                let dy = e.client_coordinates().y - d.start_y;
                let threshold = d.row_height * 0.55;
                let mut moved = false;
                while dy - d.applied > threshold {
                    let ok = match d.kind {
                        DragKind::Ingredient => rows.with_mut(|r| swap_ingredient(r, d.id, true)),
                        DragKind::Section => sections.with_mut(|s| swap_section(s, d.id, true)),
                        DragKind::Step => steps.with_mut(|s| swap_step(s, d.id, true)),
                        DragKind::StepSection => {
                            step_sections.with_mut(|s| swap_section(s, d.id, true))
                        }
                    };
                    if !ok { break; }
                    d.applied += d.row_height;
                    moved = true;
                }
                while d.applied - dy > threshold {
                    let ok = match d.kind {
                        DragKind::Ingredient => rows.with_mut(|r| swap_ingredient(r, d.id, false)),
                        DragKind::Section => sections.with_mut(|s| swap_section(s, d.id, false)),
                        DragKind::Step => steps.with_mut(|s| swap_step(s, d.id, false)),
                        DragKind::StepSection => {
                            step_sections.with_mut(|s| swap_section(s, d.id, false))
                        }
                    };
                    if !ok { break; }
                    d.applied -= d.row_height;
                    moved = true;
                }
                if moved {
                    drag.set(Some(d));
                }
            },
            onpointerup: move |_| {
                if drag.read().is_some() {
                    suppress_click.set(true);
                    drag.set(None);
                }
            },
            onpointerleave: move |_| {
                if drag.read().is_some() {
                    suppress_click.set(true);
                    drag.set(None);
                }
            },
            h1 { if editing { "Edit Recipe" } else { "Add a Recipe" } }
            form {
                class: "card",
                onsubmit: move |e: FormEvent| {
                    e.prevent_default();
                    status_error.set(false);
                    let sections_now = sections.read().clone();
                    let rows_now = rows.read().clone();
                    let steps_now = steps.read().clone();
                    let step_sections_now = step_sections.read().clone();
                    let ingredients: Vec<Ingredient> = rows_now
                        .iter()
                        .filter(|r| !r.name.trim().is_empty())
                        .map(|r| Ingredient {
                            quantity: r.quantity.trim().parse::<f64>().ok(),
                            unit: (!r.unit.trim().is_empty()).then(|| r.unit.trim().to_string()),
                            name: r.name.trim().to_string(),
                            prep: (!r.prep.trim().is_empty()).then(|| r.prep.trim().to_string()),
                            section: r.section_id.and_then(|sid| {
                                sections_now
                                    .iter()
                                    .find(|s| s.id == sid)
                                    .map(|s| s.name.trim().to_string())
                            }),
                        })
                        .collect();
                    let instructions: Vec<InstructionStep> = steps_now
                        .iter()
                        .filter(|s| !s.text.trim().is_empty())
                        .map(|s| InstructionStep {
                            text: s.text.trim().to_string(),
                            section: s.section_id.and_then(|sid| {
                                step_sections_now
                                    .iter()
                                    .find(|sec| sec.id == sid)
                                    .map(|sec| sec.name.trim().to_string())
                            }),
                        })
                        .collect();
                    let input = RecipeInput {
                        name: name.read().trim().to_string(),
                        sections: sections_now
                            .iter()
                            .map(|s| s.name.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect(),
                        ingredients: ingredients.clone(),
                        instructions: instructions.clone(),
                        instruction_sections: step_sections_now
                            .iter()
                            .map(|s| s.name.trim().to_string())
                            .filter(|s| !s.is_empty())
                            .collect(),
                        notes: notes.read().trim().to_string(),
                        yield_amount: yield_amount.read().trim().to_string(),
                        source: source.read().trim().to_string(),
                    };
                    if input.name.trim().is_empty() {
                        status.set("Please enter a recipe name.".into());
                        status_error.set(true);
                        return;
                    }
                    let image = photo.read().clone();
                    spawn(async move {
                        let client = reqwest::Client::new();
                        let base = api_base();
                        tracing::info!(
                            "Recipe form submit (edit={:?}, name={:?}, ingredients={}, photo={})",
                            editing_id, input.name, input.ingredients.len(), image.is_some()
                        );
                        let ingredients_json = serde_json::to_string(&input.ingredients).unwrap_or_default();
                        let sections_json = serde_json::to_string(&input.sections).unwrap_or_default();
                        let instructions_json = serde_json::to_string(&input.instructions).unwrap_or_default();
                        let instruction_sections_json = serde_json::to_string(&input.instruction_sections).unwrap_or_default();
                        let notes_json = input.notes.clone();
                        let yield_json = input.yield_amount.clone();
                        let source_json = input.source.clone();

                        let import_url = import_image_url.read().clone();
                        let import_url_ref = import_url.as_deref();
                        let result = match (editing_id, image) {
                            // Plain create: multipart (not JSON) whenever an
                            // imported image URL must ride along.
                            (None, None) if import_url.is_none() => client
                                .post(format!("{base}/api/recipes"))
                                .json(&input)
                                .send()
                                .await
                                .map(|r| (r, None)),
                            (None, None) => {
                                let (ct, body) = build_multipart(
                                    &input.name,
                                    &sections_json,
                                    &ingredients_json,
                                    &instructions_json,
                                    &instruction_sections_json,
                                    &notes_json,
                                    &yield_json,
                                    &source_json,
                                    None,
                                    import_url.as_deref(),
                                );
                                client
                                    .post(format!("{base}/api/recipes/photo"))
                                    .header("Content-Type", ct)
                                    .body(body)
                                    .send()
                                    .await
                                    .map(|r| (r, None))
                            }
                            (None, Some((filename, bytes))) => {
                                let (ct, body) = build_multipart(
                                    &input.name,
                                    &sections_json,
                                    &ingredients_json,
                                    &instructions_json,
                                    &instruction_sections_json,
                                    &notes_json,
                                    &yield_json,
                                    &source_json,
                                    Some((&filename, &bytes)),
                                    None,
                                );
                                client
                                    .post(format!("{base}/api/recipes/photo"))
                                    .header("Content-Type", ct)
                                    .body(body)
                                    .send()
                                    .await
                                    .map(|r| (r, None))
                            }
                            (Some(id), image) => {
                                let image_ref = image
                                    .as_ref()
                                    .map(|(f, b)| (f.as_str(), b.as_slice()));
                                let (ct, body) = build_multipart(
                                    &input.name,
                                    &sections_json,
                                    &ingredients_json,
                                    &instructions_json,
                                    &instruction_sections_json,
                                    &notes_json,
                                    &yield_json,
                                    &source_json,
                                    image_ref,
                                    if image_ref.is_some() {
                                        None
                                    } else {
                                        import_url_ref
                                    },
                                );
                                client
                                    .put(format!("{base}/api/recipes/{id}"))
                                    .header("Content-Type", ct)
                                    .body(body)
                                    .send()
                                    .await
                                    .map(|r| (r, editing_id))
                            }
                        };
                        match result {
                            Ok((r, redirect)) if r.status().is_success() => {
                                tracing::info!("recipe saved ({})", r.status());
                                let saved_id = match redirect {
                                    Some(id) => Some(id),
                                    None => r.json::<Recipe>().await.ok().map(|recipe| recipe.id),
                                };
                                match saved_id {
                                    Some(id) => navigator.replace(Route::RecipeDetail { id }),
                                    None => navigator.replace(Route::Recipes {}),
                                };
                            }
                            Ok((r, _)) => {
                                let msg = format!("Server error: {}", r.status());
                                tracing::error!("recipe save failed: {msg}");
                                status.set(msg);
                                status_error.set(true);
                            }
                            Err(err) => {
                                tracing::error!("recipe save request failed: {err:#}");
                                status.set(format!("Request failed: {err}"));
                                status_error.set(true);
                            }
                        }
                    });
                },
                label { "Recipe Name" }
                input {
                    id: "recipe-name",
                    r#type: "text",
                    value: "{name_value}",
                    placeholder: "e.g. Pancakes",
                    oninput: move |e: FormEvent| {
                        let v = e.value();
                        name.set(v);
                    },
                }

                div { class: "list-tools",
                    label { "Ingredients" }
                }
                div { id: "ingredient-rows", class: "ingredient-rows",
                    for (section, group) in groups {
                        if let Some(section) = section {
                            div { class: "section-header", id: "section-header-{section.id}",
                                span { class: "section-name", "{section.name}" }
                                div { class: "row-actions",
                                    button {
                                        id: "section-edit-{section.id}",
                                        class: "row-btn",
                                        title: "Rename section",
                                        r#type: "button",
                                        onclick: move |e: MouseEvent| {
                                            e.stop_propagation();
                                            modal.set(Some(Modal::Section {
                                                editing_section: Some(section.id),
                                                name: section.name.clone(),
                                                is_step: false,
                                            }));
                                        },
                                        IconPencil {}
                                    }
                                    button {
                                        id: "section-delete-{section.id}",
                                        class: "row-btn danger",
                                        title: "Delete section",
                                        r#type: "button",
                                        onclick: move |e: MouseEvent| {
                                            e.stop_propagation();
                                            tracing::info!("deleting section {} and its ingredients", section.id);
                                            sections.with_mut(|s| s.retain(|x| x.id != section.id));
                                            rows.with_mut(|r| r.retain(|x| x.section_id != Some(section.id)));
                                        },
                                        IconX {}
                                    }
                                    button {
                                        id: "drag-handle-{section.id}",
                                        class: "row-btn drag-handle",
                                        title: "Drag to reorder",
                                        r#type: "button",
                                        onpointerdown: move |e: PointerEvent| {
                                            e.stop_propagation();
                                            drag.set(Some(DragState {
                                                kind: DragKind::Section,
                                                id: section.id,
                                                start_y: e.client_coordinates().y,
                                                row_height: measure_row_height(&format!("section-header-{}", section.id)),
                                                applied: 0.0,
                                            }));
                                        },
                                        IconHandle {}
                                    }
                                }
                            }
                        }
                        for (row, amount) in group {
                            div {
                                class: if drag_snapshot.as_ref().is_some_and(|d| d.id == row.id && d.kind == DragKind::Ingredient) { "ingredient-row dragging" } else { "ingredient-row" },
                                id: "ing-row-{row.id}",
                                onclick: move |_| {
                                    if suppress_click() {
                                        suppress_click.set(false);
                                        return;
                                    }
                                    modal.set(Some(Modal::Ingredient {
                                        section_id: row.section_id,
                                        editing_row: Some(row.id),
                                        qty: row.quantity.clone(),
                                        unit: row.unit.clone(),
                                        name: row.name.clone(),
                                        prep: row.prep.clone(),
                                    }));
                                },
                                span { class: "ing-amount", "{amount}" }
                                div { class: "ing-main",
                                    span { class: "ing-name", "{row.name}" }
                                    if !row.prep.trim().is_empty() {
                                        span { class: "ing-prep-text", "{row.prep}" }
                                    }
                                }
                                div { class: "row-actions", onclick: move |e: MouseEvent| e.stop_propagation(),
                                    button {
                                        id: "ing-delete-{row.id}",
                                        class: "row-btn danger",
                                        title: "Remove ingredient",
                                        r#type: "button",
                                        onclick: move |e: MouseEvent| {
                                            e.stop_propagation();
                                            let id = row.id;
                                            tracing::debug!("deleting ingredient row {id}");
                                            rows.with_mut(|r| r.retain(|x| x.id != id));
                                        },
                                        IconX {}
                                    }
                                    button {
                                        id: "drag-handle-{row.id}",
                                        class: "row-btn drag-handle",
                                        title: "Drag to reorder",
                                        r#type: "button",
                                        onpointerdown: move |e: PointerEvent| {
                                            e.stop_propagation();
                                            let id = row.id;
                                            drag.set(Some(DragState {
                                                kind: DragKind::Ingredient,
                                                id,
                                                start_y: e.client_coordinates().y,
                                                row_height: measure_row_height(&format!("ing-row-{id}")),
                                                applied: 0.0,
                                            }));
                                        },
                                        IconHandle {}
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "list-actions",
                    button {
                        id: "add-ingredient",
                        class: "list-add",
                        r#type: "button",
                        onclick: move |_| {
                            let default_section = sections.read().last().map(|s| s.id);
                            tracing::debug!("opening ingredient modal (add)");
                            modal.set(Some(Modal::Ingredient {
                                section_id: default_section,
                                editing_row: None,
                                qty: String::new(),
                                unit: String::new(),
                                name: String::new(),
                                prep: String::new(),
                            }));
                        },
                        IconPlus {}
                        span { "Add ingredient" }
                    }
                    button {
                        id: "add-section",
                        class: "list-add",
                        r#type: "button",
                        onclick: move |_| {
                            tracing::debug!("opening section modal (add)");
                            modal.set(Some(Modal::Section { editing_section: None, name: String::new(), is_step: false }));
                        },
                        IconList {}
                        span { "Add section" }
                    }
                }

                label { "Instructions" }
                div { id: "step-rows", class: "step-rows",
                    for (section, group) in numbered_step_groups.clone() {
                        if let Some(section) = section {
                            div { class: "section-header", id: "step-section-header-{section.id}",
                                span { class: "section-name", "{section.name}" }
                                div { class: "row-actions",
                                    button {
                                        id: "step-section-edit-{section.id}",
                                        class: "row-btn",
                                        title: "Rename section",
                                        r#type: "button",
                                        onclick: move |e: MouseEvent| {
                                            e.stop_propagation();
                                            modal.set(Some(Modal::Section {
                                                editing_section: Some(section.id),
                                                name: section.name.clone(),
                                                is_step: true,
                                            }));
                                        },
                                        IconPencil {}
                                    }
                                    button {
                                        id: "step-section-delete-{section.id}",
                                        class: "row-btn danger",
                                        title: "Delete section",
                                        r#type: "button",
                                        onclick: move |e: MouseEvent| {
                                            e.stop_propagation();
                                            tracing::info!("deleting instruction section {} and its steps", section.id);
                                            step_sections.with_mut(|s| s.retain(|x| x.id != section.id));
                                            steps.with_mut(|s| s.retain(|x| x.section_id != Some(section.id)));
                                        },
                                        IconX {}
                                    }
                                    button {
                                        id: "drag-handle-{section.id}",
                                        class: "row-btn drag-handle",
                                        title: "Drag to reorder",
                                        r#type: "button",
                                        onpointerdown: move |e: PointerEvent| {
                                            e.stop_propagation();
                                            drag.set(Some(DragState {
                                                kind: DragKind::StepSection,
                                                id: section.id,
                                                start_y: e.client_coordinates().y,
                                                row_height: measure_row_height(&format!("step-section-header-{}", section.id)),
                                                applied: 0.0,
                                            }));
                                        },
                                        IconHandle {}
                                    }
                                }
                            }
                        }
                        for (number, step) in group.into_iter() {
                            div {
                                class: if drag_snapshot.as_ref().is_some_and(|d| d.id == step.id && d.kind == DragKind::Step) { "step-row dragging" } else { "step-row" },
                                id: "step-row-{step.id}",
                                onclick: move |_| {
                                    if suppress_click() {
                                        suppress_click.set(false);
                                        return;
                                    }
                                    modal.set(Some(Modal::Step {
                                        editing_step: Some(step.id),
                                        section_id: step.section_id,
                                        text: step.text.clone(),
                                    }));
                                },
                                span { class: "step-badge", "{number}" }
                                span { class: "step-text", "{step.text}" }
                                div { class: "row-actions", onclick: move |e: MouseEvent| e.stop_propagation(),
                                    button {
                                        id: "step-delete-{step.id}",
                                        class: "row-btn danger",
                                        title: "Delete step",
                                        r#type: "button",
                                        onclick: move |e: MouseEvent| {
                                            e.stop_propagation();
                                            let id = step.id;
                                            tracing::debug!("deleting step {id}");
                                            steps.with_mut(|s| s.retain(|x| x.id != id));
                                        },
                                        IconX {}
                                    }
                                    button {
                                        id: "drag-handle-{step.id}",
                                        class: "row-btn drag-handle",
                                        title: "Drag to reorder",
                                        r#type: "button",
                                        onpointerdown: move |e: PointerEvent| {
                                            e.stop_propagation();
                                            let id = step.id;
                                            drag.set(Some(DragState {
                                                kind: DragKind::Step,
                                                id,
                                                start_y: e.client_coordinates().y,
                                                row_height: measure_row_height(&format!("step-row-{id}")),
                                                applied: 0.0,
                                            }));
                                        },
                                        IconHandle {}
                                    }
                                }
                            }
                        }
                    }
                }
                div { class: "list-actions",
                    button {
                        id: "add-step",
                        class: "list-add",
                        r#type: "button",
                        onclick: move |_| {
                            tracing::debug!("opening step modal (add)");
                            let default_section = step_sections.read().last().map(|s| s.id);
                            modal.set(Some(Modal::Step {
                                editing_step: None,
                                section_id: default_section,
                                text: String::new(),
                            }));
                        },
                        IconPlus {}
                        span { "Add step" }
                    }
                    button {
                        id: "add-step-section",
                        class: "list-add",
                        r#type: "button",
                        onclick: move |_| {
                            tracing::debug!("opening instruction-section modal (add)");
                            modal.set(Some(Modal::Section {
                                editing_section: None,
                                name: String::new(),
                                is_step: true,
                            }));
                        },
                        IconList {}
                        span { "Add section" }
                    }
                }

                label { "Yield" }
                input {
                    id: "recipe-yield",
                    r#type: "text",
                    value: "{yield_value}",
                    placeholder: "e.g. 12 cookies",
                    oninput: move |e: FormEvent| {
                        let v = e.value();
                        yield_amount.set(v);
                    },
                }
                label { "Source" }
                input {
                    id: "recipe-source",
                    r#type: "text",
                    value: "{source_value}",
                    placeholder: "e.g. Grandma's cookbook",
                    oninput: move |e: FormEvent| {
                        let v = e.value();
                        source.set(v);
                    },
                }
                label { "Notes" }
                textarea {
                    id: "recipe-notes",
                    value: "{notes_value}",
                    placeholder: "Anything worth remembering about this recipe…",
                    oninput: move |e: FormEvent| {
                        let v = e.value();
                        notes.set(v);
                    },
                }

                label { "Photo" }
                if let Some(thumb) = &existing_thumb {
                    if editing {
                        img { class: "photo-preview", src: "{thumb}", alt: "current photo" }
                    }
                }
                input {
                    id: "recipe-photo",
                    r#type: "file",
                    accept: "image/jpeg,image/png,image/webp",
                    onchange: move |e: FormEvent| {
                        let files = e.files();
                        spawn(async move {
                            if let Some(file) = files.into_iter().next() {
                                tracing::info!("photo selected: {} ({} bytes)", file.name(), file.size());
                                match file.read_bytes().await {
                                    Ok(bytes) => photo.set(Some((file.name(), bytes.to_vec()))),
                                    Err(err) => {
                                        tracing::error!("failed to read photo: {err:#}");
                                    }
                                }
                            }
                        });
                    },
                }
                if let Some((fname, _)) = photo.read().clone() {
                    p { class: "hint", "Selected: {fname}" }
                }

                button { id: "recipe-submit", r#type: "submit",
                    if editing { "Save Changes" } else { "Add Recipe" }
                }
            }
            p { id: "recipe-status", class: if status_error() { "error" } else { "" }, "{status}" }

            if let Some(m) = modal_snapshot {
                div {
                    class: "dialog-backdrop",
                    onclick: move |_| modal.set(None),
                    div { class: "dialog", role: "dialog", onclick: move |e: MouseEvent| e.stop_propagation(),
                        match m {
                            Modal::Ingredient { section_id, editing_row, qty, unit, name, prep } => rsx! {
                                h2 { class: "dialog-title",
                                    if editing_row.is_some() { "Edit ingredient" } else { "Add ingredient" }
                                }
                                div { class: "modal-row",
                                    input {
                                        id: "modal-qty",
                                        class: if qty_valid { "" } else { "invalid" },
                                        r#type: "text",
                                        placeholder: "Qty",
                                        value: "{qty}",
                                        oninput: move |e: FormEvent| {
                                            let v = e.value();
                                            modal.with_mut(|m| {
                                                if let Some(Modal::Ingredient { qty, .. }) = m { *qty = v; }
                                            });
                                        },
                                    }
                                    input {
                                        id: "modal-unit",
                                        r#type: "text",
                                        placeholder: "Unit",
                                        value: "{unit}",
                                        oninput: move |e: FormEvent| {
                                            let v = e.value();
                                            modal.with_mut(|m| {
                                                if let Some(Modal::Ingredient { unit, .. }) = m { *unit = v; }
                                            });
                                        },
                                    }
                                }
                                input {
                                    id: "modal-name",
                                    r#type: "text",
                                    placeholder: "Name *",
                                    value: "{name}",
                                    oninput: move |e: FormEvent| {
                                        let v = e.value();
                                        modal.with_mut(|m| {
                                            if let Some(Modal::Ingredient { name, .. }) = m { *name = v; }
                                        });
                                    },
                                }
                                if !qty_valid {
                                    p { class: "modal-error",
                                        "Quantity must be a positive number (or empty)."
                                    }
                                }
                                input {
                                    id: "modal-prep",
                                    r#type: "text",
                                    placeholder: "Prep (optional)",
                                    value: "{prep}",
                                    oninput: move |e: FormEvent| {
                                        let v = e.value();
                                        modal.with_mut(|m| {
                                            if let Some(Modal::Ingredient { prep, .. }) = m { *prep = v; }
                                        });
                                    },
                                }
                                label { class: "modal-label", "Section" }
                                select {
                                    id: "modal-section",
                                    onchange: move |e: FormEvent| {
                                        let v = e.value();
                                        modal.with_mut(|m| {
                                            if let Some(Modal::Ingredient { section_id, .. }) = m {
                                                *section_id = v.parse().ok();
                                            }
                                        });
                                    },
                                    option { value: "", selected: if section_id.is_none() { "true" } else { "false" }, "No section" }
                                    for s in &sections_snapshot {
                                        option {
                                            value: "{s.id}",
                                            selected: if section_id == Some(s.id) { "true" } else { "false" },
                                            "{s.name}"
                                        }
                                    }
                                }
                                div { class: "dialog-actions",
                                    button {
                                        id: "modal-cancel",
                                        class: "dialog-btn",
                                        onclick: move |_| modal.set(None),
                                        "Cancel"
                                    }
                                    button {
                                        id: "modal-save",
                                        class: "dialog-btn primary",
                                        onclick: move |_| {
                                            if name.trim().is_empty() || !qty_valid {
                                                return;
                                            }
                                            let row = IngredientRow {
                                                id: editing_row.unwrap_or_default(),
                                                section_id: section_id,
                                                name: name.trim().to_string(),
                                                quantity: qty.trim().to_string(),
                                                unit: unit.trim().to_string(),
                                                prep: prep.trim().to_string(),
                                            };
                                            match editing_row {
                                                Some(id) => rows.with_mut(|r| {
                                                    if let Some(existing) = r.iter_mut().find(|x| x.id == id) {
                                                        *existing = row;
                                                    }
                                                }),
                                                None => {
                                                    let id = *next_id.read();
                                                    rows.with_mut(|r| r.push(IngredientRow { id, ..row }));
                                                    next_id.set(id + 1);
                                                }
                                            }
                                            tracing::debug!("ingredient modal saved (row={editing_row:?})");
                                            modal.set(None);
                                        },
                                        "Save"
                                    }
                                }
                            },
                            Modal::Section { editing_section, name, is_step } => rsx! {
                                h2 { class: "dialog-title",
                                    if editing_section.is_some() { "Rename section" } else { "Add section" }
                                }
                                input {
                                    id: "modal-section-name",
                                    r#type: "text",
                                    placeholder: "Section name",
                                    value: "{name}",
                                    oninput: move |e: FormEvent| {
                                        let v = e.value();
                                        modal.with_mut(|m| {
                                            if let Some(Modal::Section { name, .. }) = m { *name = v; }
                                        });
                                    },
                                }
                                div { class: "dialog-actions",
                                    button { id: "modal-cancel", class: "dialog-btn", onclick: move |_| modal.set(None), "Cancel" }
                                    button {
                                        id: "modal-save",
                                        class: "dialog-btn primary",
                                        onclick: move |_| {
                                            let trimmed = name.trim().to_string();
                                            if trimmed.is_empty() {
                                                return;
                                            }
                                            if is_step {
                                                match editing_section {
                                                    Some(id) => step_sections.with_mut(|s| {
                                                        if let Some(section) = s.iter_mut().find(|x| x.id == id) {
                                                            section.name = trimmed;
                                                        }
                                                    }),
                                                    None => {
                                                        let id = *next_id.read();
                                                        step_sections.with_mut(|s| s.push(SectionRow { id, name: trimmed }));
                                                        next_id.set(id + 1);
                                                    }
                                                }
                                            } else {
                                                match editing_section {
                                                    Some(id) => sections.with_mut(|s| {
                                                        if let Some(section) = s.iter_mut().find(|x| x.id == id) {
                                                            section.name = trimmed;
                                                        }
                                                    }),
                                                    None => {
                                                        let id = *next_id.read();
                                                        sections.with_mut(|s| s.push(SectionRow { id, name: trimmed }));
                                                        next_id.set(id + 1);
                                                    }
                                                }
                                            }
                                            modal.set(None);
                                        },
                                        "Save"
                                    }
                                }
                            },
                            Modal::Step { editing_step, section_id, text } => rsx! {
                                h2 { class: "dialog-title",
                                    if editing_step.is_some() { "Edit instruction" } else { "Add instruction" }
                                }
                                textarea {
                                    id: "modal-step-text",
                                    placeholder: "Instruction",
                                    value: "{text}",
                                    oninput: move |e: FormEvent| {
                                        let v = e.value();
                                        modal.with_mut(|m| {
                                            if let Some(Modal::Step { text, .. }) = m { *text = v; }
                                        });
                                    },
                                }
                                label { class: "modal-label", "Section" }
                                select {
                                    id: "modal-step-section",
                                    onchange: move |e: FormEvent| {
                                        let v = e.value();
                                        modal.with_mut(|m| {
                                            if let Some(Modal::Step { section_id, .. }) = m {
                                                *section_id = v.parse().ok();
                                            }
                                        });
                                    },
                                    option { value: "", selected: if section_id.is_none() { "true" } else { "false" }, "No section" }
                                    for s in &step_sections_snapshot {
                                        option {
                                            value: "{s.id}",
                                            selected: if section_id == Some(s.id) { "true" } else { "false" },
                                            "{s.name}"
                                        }
                                    }
                                }
                                div { class: "dialog-actions",
                                    button { id: "modal-cancel", class: "dialog-btn", onclick: move |_| modal.set(None), "Cancel" }
                                    button {
                                        id: "modal-save",
                                        class: "dialog-btn primary",
                                        onclick: move |_| {
                                            let trimmed = text.trim().to_string();
                                            if trimmed.is_empty() {
                                                return;
                                            }
                                            match editing_step {
                                                Some(id) => steps.with_mut(|s| {
                                                    if let Some(step) = s.iter_mut().find(|x| x.id == id) {
                                                        step.text = trimmed;
                                                        step.section_id = section_id;
                                                    }
                                                }),
                                                None => {
                                                    let id = *next_id.read();
                                                    steps.with_mut(|s| s.push(StepRow {
                                                        id,
                                                        section_id,
                                                        text: trimmed,
                                                    }));
                                                    next_id.set(id + 1);
                                                }
                                            }
                                            modal.set(None);
                                        },
                                        "Save"
                                    }
                                }
                            },
                        }
                    }
                }
            }
        }
    }
}
// Tab: Recipe detail (blaz-style)
// ---------------------------------------------------------------------------

#[component]
fn RecipeDetail(id: i64) -> Element {
    let mut detail = use_signal(|| None::<RecipeDetailModel>);
    let mut error = use_signal(|| String::new());
    let mut confirm_delete = use_signal(|| false);
    let mut scale_text = use_signal(|| String::from("1"));
    let mut cart_sheet = use_signal(|| false);
    let mut added_note = use_signal(|| String::new());
    let navigator = use_navigator();

    use_effect(move || {
        spawn(async move {
            match api_get::<RecipeDetailModel>(&format!("/api/recipes/{id}")).await {
                Ok(d) => {
                    tracing::debug!("recipe {id} loaded: {} ingredients", d.ingredients.len());
                    detail.set(Some(d));
                }
                Err(err) => {
                    tracing::error!("recipe {id} failed to load: {err:#}");
                    error.set(err.to_string());
                }
            }
        });
    });

    let loaded = detail.read().clone();
    let image_url = loaded.as_ref().and_then(|d| d.image.clone());
    let scale_value = scale_text.read().clone();
    let scale = scale_value.trim().parse::<f64>().ok();
    let scale_valid = scale.is_some_and(|s| s.is_finite() && s > 0.0 && s <= 1000.0);
    let effective_scale = if scale_valid { scale.unwrap() } else { 1.0 };

    // Detail view ingredient grouping: unsectioned items first, then one
    // group per section (in order).
    let detail_groups: Vec<(Option<String>, Vec<Ingredient>)> = loaded
        .as_ref()
        .map(|d| {
            let mut groups: Vec<(Option<String>, Vec<Ingredient>)> = Vec::new();
            let plain: Vec<Ingredient> = d
                .ingredients
                .iter()
                .filter(|i| i.section.is_none())
                .cloned()
                .collect();
            if !plain.is_empty() {
                groups.push((None, plain));
            }
            for section in &d.sections {
                groups.push((
                    Some(section.clone()),
                    d.ingredients
                        .iter()
                        .filter(|i| i.section.as_deref() == Some(section.as_str()))
                        .cloned()
                        .collect(),
                ));
            }
            groups
        })
        .unwrap_or_default();

    let has_ingredients = loaded
        .as_ref()
        .is_some_and(|d| !d.ingredients.is_empty());

    // Shopping-list sheet lines: base quantities (scale ignored on purpose).
    let cart_lines: Vec<String> = loaded
        .as_ref()
        .map(|d| {
            d.ingredients
                .iter()
                .map(|ingredient| ingredient_line(ingredient, 1.0))
                .collect()
        })
        .unwrap_or_default();

    // Detail view instruction grouping: unsectioned steps first, then one
    // group per instruction section (in order), with continuous numbering.
    let instruction_groups: Vec<(Option<String>, Vec<(usize, InstructionStep)>)> = loaded
        .as_ref()
        .map(|d| {
            let mut groups: Vec<(Option<String>, Vec<InstructionStep>)> = Vec::new();
            let plain: Vec<InstructionStep> = d
                .instructions
                .iter()
                .filter(|s| s.section.is_none())
                .cloned()
                .collect();
            if !plain.is_empty() {
                groups.push((None, plain));
            }
            for section in &d.instruction_sections {
                groups.push((
                    Some(section.clone()),
                    d.instructions
                        .iter()
                        .filter(|s| s.section.as_deref() == Some(section.as_str()))
                        .cloned()
                        .collect(),
                ));
            }
            let mut number = 0usize;
            groups
                .into_iter()
                .map(|(section, group)| {
                    let numbered = group
                        .into_iter()
                        .map(|step| {
                            number += 1;
                            (number, step)
                        })
                        .collect::<Vec<_>>();
                    (section, numbered)
                })
                .collect()
        })
        .unwrap_or_default();

    rsx! {
        div { class: "page detail-page",
            div { class: "detail-header",
                button {
                    id: "hdr-back",
                    class: "hdr-btn",
                    title: "Back",
                    onclick: move |_| { navigator.push(Route::Recipes {}); },
                    IconBack {}
                }
                div { class: "hdr-spacer" }
                button { class: "hdr-btn ph", title: "Timer", IconTimer {} }
                button { class: "hdr-btn ph", title: "Share", IconShare {} }
                button { class: "hdr-btn ph", title: "Add to meal plan", IconCalendar {} }
                button {
                    id: "hdr-cart",
                    class: if has_ingredients { "hdr-btn" } else { "hdr-btn ph" },
                    title: "Add to shopping list",
                    onclick: move |_| {
                        if has_ingredients {
                            added_note.set(String::new());
                            cart_sheet.set(true);
                        }
                    },
                    IconCart {}
                }
                button {
                    id: "hdr-edit",
                    class: "hdr-btn",
                    title: "Edit",
                    onclick: move |_| { navigator.push(Route::EditRecipe { id }); },
                    IconPencil {}
                }
                button {
                    id: "hdr-delete",
                    class: "hdr-btn danger",
                    title: "Delete",
                    onclick: move |_| {
                        tracing::info!("delete requested for recipe {id}");
                        confirm_delete.set(true);
                    },
                    IconTrash {}
                }
            }

            if !added_note.read().is_empty() {
                p { class: "added-note", "{added_note}" }
            }

            match loaded.as_ref() {
                Some(d) => rsx! {
                    h1 { class: "detail-name", "{d.name}" }
                    if let Some(url) = &image_url {
                        div { class: "detail-photo",
                            img { src: "{url}", alt: "{d.name}" }
                        }
                    }
                    div { class: "card",
                        div { class: "scale-row",
                            label { "Scale" }
                            input {
                                id: "scale-input",
                                class: if scale_valid { "" } else { "invalid" },
                                r#type: "text",
                                value: "{scale_value}",
                                oninput: move |e: FormEvent| {
                                    let v = e.value();
                                    scale_text.set(v);
                                },
                            }
                            if scale_valid {
                                span { class: "scale-factor", "{fmt_qty(effective_scale)}x" }
                            } else {
                                span { class: "scale-hint", "0 < scale ≤ 1000" }
                            }
                            button {
                                id: "scale-reset",
                                class: "scale-reset",
                                onclick: move |_| scale_text.set(String::from("1")),
                                "Reset"
                            }
                        }
                        h2 { "Ingredients" }
                        if d.ingredients.is_empty() && d.sections.is_empty() {
                            p { class: "empty", "No ingredients yet." }
                        } else {
                            // Grouped view: unsectioned items first, then each
                            // section under a teal header (like blaz).
                            div { id: "ingredient-list", class: "ingredient-groups",
                                for (section, group) in &detail_groups {
                                    if let Some(name) = section {
                                        div { class: "detail-section-title", "{name}" }
                                    }
                                    ul { class: "ingredient-list",
                                        for ingredient in group {
                                            li { class: "ingredient-item", "{ingredient_line(ingredient, effective_scale)}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    div { class: "card",
                        h2 { "Instructions" }
                        if d.instructions.is_empty() && d.instruction_sections.is_empty() {
                            p { class: "empty", "No instructions yet." }
                        } else {
                            // Grouped view with continuous numbering across
                            // sections.
                            div { id: "instruction-list", class: "ingredient-groups",
                                for (section, group) in &instruction_groups {
                                    if let Some(name) = section {
                                        div { class: "detail-section-title", "{name}" }
                                    }
                                    ol { class: "instruction-list",
                                        for (number, step) in group {
                                            li { key: "{number}", class: "instruction-item",
                                                span { class: "step-badge", "{number}" }
                                                span { class: "step-text", "{step.text}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !d.yield_amount.trim().is_empty() {
                        div { class: "card meta-card", id: "detail-yield",
                            h2 { "Yield" }
                            p { "{d.yield_amount}" }
                        }
                    }
                    if !d.source.trim().is_empty() {
                        div { class: "card meta-card", id: "detail-source",
                            h2 { "Source" }
                            p { "{d.source}" }
                        }
                    }
                    if !d.notes.trim().is_empty() {
                        div { class: "card meta-card", id: "detail-notes",
                            h2 { "Notes" }
                            p { "{d.notes}" }
                        }
                    }
                },
                None => rsx! {
                    if error.read().is_empty() {
                        p { class: "empty", "Loading…" }
                    } else {
                        p { class: "status-error", "{error}" }
                    }
                },
            }

            if confirm_delete() {
                DeleteDialog {
                    name: loaded.as_ref().map(|d| d.name.clone()).unwrap_or_default(),
                    on_cancel: move |_| confirm_delete.set(false),
                    on_confirm: move |_| {
                        spawn(async move {
                            let client = reqwest::Client::new();
                            let url = format!("{}/api/recipes/{id}", api_base());
                            tracing::info!("deleting recipe {id}: DELETE {url}");
                            match client.delete(&url).send().await {
                                Ok(r) if r.status().is_success() => {
                                    tracing::info!("recipe {id} deleted");
                                    navigator.replace(Route::Recipes {});
                                }
                                Ok(r) => {
                                    tracing::error!("delete failed: {}", r.status());
                                    confirm_delete.set(false);
                                    error.set(format!("Delete failed: {}", r.status()));
                                }
                                Err(err) => {
                                    tracing::error!("delete request failed: {err:#}");
                                    confirm_delete.set(false);
                                    error.set(format!("Delete failed: {err}"));
                                }
                            }
                        });
                    },
                }
            }

            if cart_sheet() && !cart_lines.is_empty() {
                CartSheet {
                    recipe_name: loaded
                        .as_ref()
                        .map(|d| d.name.clone())
                        .unwrap_or_default(),
                    lines: cart_lines.clone(),
                    on_add: move |selected: Vec<String>| {
                        cart_sheet.set(false);
                        let count = selected.len();
                        if count == 0 {
                            return;
                        }
                        let payload = NewGroceryBatch {
                            items: selected
                                .into_iter()
                                .map(|name| NewGroceryItem { name, category: None })
                                .collect(),
                        };
                        spawn(async move {
                            let client = reqwest::Client::new();
                            let url = format!("{}/api/grocery/batch", api_base());
                            match client.post(&url).json(&payload).send().await {
                                Ok(r) if r.status().is_success() => {
                                    tracing::info!("added {count} items to shopping list");
                                    added_note.set(format!(
                                        "Added {count} item{} to shopping list",
                                        if count == 1 { "" } else { "s" }
                                    ));
                                }
                                Ok(r) => {
                                    tracing::error!("grocery batch failed: {}", r.status());
                                    error.set(format!(
                                        "Could not add to shopping list: {}",
                                        r.status()
                                    ));
                                }
                                Err(err) => {
                                    tracing::error!("grocery batch request failed: {err:#}");
                                    error.set(format!("Could not add to shopping list: {err}"));
                                }
                            }
                        });
                    },
                    on_cancel: move |_| cart_sheet.set(false),
                }
            }
        }
    }
}

/// One ingredient line, e.g. `180 g buckwheat flour, finely chopped`,
/// with the quantity multiplied by the detail-view scale factor.
fn ingredient_line(ingredient: &Ingredient, scale: f64) -> String {
    let mut line = String::new();
    if let Some(q) = ingredient.quantity {
        line.push_str(&fmt_qty(q * scale));
        line.push(' ');
    }
    if let Some(unit) = &ingredient.unit {
        line.push_str(unit);
        line.push(' ');
    }
    line.push_str(&ingredient.name);
    if let Some(prep) = &ingredient.prep {
        line.push_str(", ");
        line.push_str(prep);
    }
    line
}

#[component]
fn DeleteDialog(
    name: String,
    on_cancel: EventHandler<MouseEvent>,
    on_confirm: EventHandler<MouseEvent>,
) -> Element {
    rsx! {
        div { class: "dialog-backdrop",
            div { class: "dialog", role: "dialog",
                p { class: "dialog-text", "Delete \u{201c}{name}\u{201d}?" }
                div { class: "dialog-actions",
                    button { id: "cancel-delete", class: "dialog-btn", onclick: on_cancel, "Cancel" }
                    button { id: "confirm-delete", class: "dialog-btn danger", onclick: on_confirm, "Delete" }
                }
            }
        }
    }
}

/// Bottom sheet for pushing recipe ingredients to the shopping list (blaz
/// style): one checkbox per line, all unchecked at first, an All toggle to
/// select everything at once, and Cancel/Add.
#[component]
fn CartSheet(
    recipe_name: String,
    lines: Vec<String>,
    on_add: EventHandler<Vec<String>>,
    on_cancel: EventHandler<()>,
) -> Element {
    let mut selected = use_signal(Vec::<bool>::new);
    // Seed per open: the sheet unmounts when closed, so the first render of
    // each open resets to all-unchecked.
    if selected.read().len() != lines.len() {
        selected.set(vec![false; lines.len()]);
    }
    let selected_snapshot = selected.read().clone();
    let all_checked = !selected_snapshot.is_empty() && selected_snapshot.iter().all(|&b| b);
    let any_checked = selected_snapshot.iter().any(|&b| b);
    let line_count = lines.len();

    rsx! {
        div { class: "sheet-backdrop",
            onclick: move |_| on_cancel.call(()),
            div { class: "sheet", role: "dialog",
                onclick: move |e: MouseEvent| e.stop_propagation(),
                h2 { class: "sheet-title", "Add to shopping list" }
                p { class: "sheet-subtitle", "{recipe_name}" }
                label { class: "sheet-all",
                    input {
                        id: "sheet-all",
                        r#type: "checkbox",
                        checked: all_checked,
                        onchange: move |_| selected.set(vec![!all_checked; line_count]),
                    }
                    "All"
                }
                div { class: "sheet-list",
                    for (index, line) in lines.iter().enumerate() {
                        label { class: "sheet-row", key: "{index}",
                            input {
                                r#type: "checkbox",
                                class: "sheet-check",
                                checked: selected_snapshot[index],
                                onchange: move |_| selected.with_mut(|v| v[index] = !v[index]),
                            }
                            span { class: "sheet-line", "{line}" }
                        }
                    }
                }
                div { class: "sheet-actions",
                    button { id: "sheet-cancel", class: "dialog-btn", onclick: move |_| on_cancel.call(()), "Cancel" }
                    button {
                        id: "sheet-add",
                        class: "dialog-btn primary",
                        disabled: !any_checked,
                        onclick: move |_| {
                            let chosen: Vec<String> = lines
                                .iter()
                                .enumerate()
                                .filter(|(index, _)| selected_snapshot[*index])
                                .map(|(_, line)| line.clone())
                                .collect();
                            on_add.call(chosen);
                        },
                        "Add"
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tab: Shopping (grouped grocery list)
// ---------------------------------------------------------------------------

/// Compute Levenshtein distance between two strings, comparing per character
/// (so multi-byte input like "é" counts as one character, not two bytes).
fn levenshtein_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut v0: Vec<usize> = (0..=b.len()).collect();
    let mut v1 = vec![0; b.len() + 1];
    for (i, ca) in a.chars().enumerate() {
        v1[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            v1[j + 1] = std::cmp::min(std::cmp::min(v1[j] + 1, v0[j + 1] + 1), v0[j] + cost);
        }
        std::mem::swap(&mut v0, &mut v1);
    }
    v0[b.len()]
}

/// Rank past item names by how closely they match the typed text using
/// Levenshtein distance (max 3 edits), closest match first. Empty input
/// yields no suggestions — the dropdown only opens for typed text.
fn rank_suggestions(input: &str, past: &[String]) -> Vec<(String, usize)> {
    let needle = input.trim().to_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let mut scored: Vec<(String, usize)> = past
        .iter()
        .filter_map(|name| {
            let hay = name.to_lowercase();
            // Prefix matches rank first even when the name is long.
            let dist = if hay.starts_with(&needle) {
                0
            } else {
                levenshtein_distance(&needle, &hay)
            };
            (dist <= 3).then_some((name.clone(), dist))
        })
        .collect();
    scored.sort_by(|a, b| a.1.cmp(&b.1));
    scored
}

#[component]
fn Grocery() -> Element {
    let mut items = use_signal(Vec::<GroceryItem>::new);
    let mut new_item = use_signal(String::new);
    let mut new_category = use_signal(String::new);
    let mut error = use_signal(|| String::new());
    let mut focused = use_signal(|| false);
    let collapsed = use_signal(|| HashSet::<String>::new());
    let mut loaded = use_signal(|| false);

    use_effect(move || {
        if !loaded() {
            loaded.set(true);
            spawn(async move {
                refresh(items, error).await;
            });
        }
    });

    let list = items.read().clone();
    // Unique categories in list order.
    let mut groups: Vec<(String, Vec<GroceryItem>)> = Vec::new();
    for item in &list {
        match groups.iter_mut().find(|(c, _)| *c == item.category) {
            Some((_, v)) => v.push(item.clone()),
            None => groups.push((item.category.clone(), vec![item.clone()])),
        }
    }

    // Past entries = unique names of items that have ever been added.
    let mut past_names: Vec<String> = Vec::new();
    for item in &list {
        if !past_names.contains(&item.name) {
            past_names.push(item.name.clone());
        }
    }

    let suggestions = rank_suggestions(&new_item.read(), &past_names);

    rsx! {
        div { class: "page",
            div { class: "add-row",
                div { class: "input-wrap",
                    input {
                        id: "grocery-input",
                        r#type: "text",
                        value: "{new_item}",
                        placeholder: "Add an item manually…",
                        oninput: move |e: FormEvent| new_item.set(e.value()),
                        onfocus: move |_| focused.set(true),
                        onblur: move |_| focused.set(false),
                        autocomplete: "off",
                    }
                    if focused() && !new_item.read().trim().is_empty() && !suggestions.is_empty() {
                        div { class: "suggestions",
                            for (name, _) in suggestions {
                                button {
                                    class: "suggestion",
                                    r#type: "button",
                                    // mousedown, not click: the input's blur
                                    // (fired on mousedown) unmounts this
                                    // dropdown before a click could land.
                                    onmousedown: move |_| {
                                        new_item.set(name.clone());
                                    },
                                    span { class: "suggestion-text", "{name}" }
                                }
                            }
                        }
                    }
                }
                input {
                    id: "grocery-category",
                    r#type: "text",
                    value: "{new_category}",
                    placeholder: "Group…",
                    list: "category-options",
                    oninput: move |e: FormEvent| new_category.set(e.value()),
                }
                datalist { id: "category-options",
                    for (category, _) in groups.clone() {
                        option { value: "{category}" }
                    }
                }
                button {
                    id: "grocery-add",
                    onclick: move |_| {
                        let item = NewGroceryItem {
                            name: new_item.read().clone(),
                            category: {
                                let c = new_category.read().trim().to_string();
                                (!c.is_empty()).then_some(c)
                            },
                        };
                        if item.name.trim().is_empty() {
                            return;
                        }
                        new_item.set(String::new());
                        spawn(async move {
                            let client = reqwest::Client::new();
                            let url = format!("{}/api/grocery", api_base());
                            tracing::info!("Grocery Add button: POST {url} (name={:?}, category={:?})", item.name, item.category);
                            match client.post(url).json(&item).send().await {
                                Ok(r) if r.status().is_success() => match r.json::<GroceryItem>().await {
                                    // Append the created item to the local
                                    // state instead of re-fetching: the list
                                    // keeps whatever the user just removed.
                                    Ok(created) => items.with_mut(|v| v.push(created)),
                                    Err(err) => {
                                        tracing::error!("POST /api/grocery returned an unreadable body: {err:#}");
                                        refresh(items, error).await;
                                    }
                                },
                                Ok(r) => {
                                    tracing::error!("POST /api/grocery failed: {}", r.status());
                                    error.set("Failed to add item.".into());
                                }
                                Err(err) => {
                                    tracing::error!("POST /api/grocery request failed: {err:#}");
                                    error.set("Failed to add item.".into());
                                }
                            }
                        });
                    },
                    "Add"
                }
            }
            if !error.read().is_empty() {
                p { class: "status-error", "{error}" }
            }
            if groups.is_empty() {
                p { class: "empty", "Your grocery list is empty. Add an item above." }
            } else {
                div { id: "grocery-list", class: "grocery-groups",
                    for (category, group_items) in groups {
                        GroupSection {
                            key: "{category}",
                            name: category,
                            items: group_items,
                            collapsed: collapsed,
                            items_sig: items,
                            error: error,
                        }
                    }
                }
            }
            div { class: "fab-stack",
                button {
                    id: "grocery-fab-add",
                    class: "fab",
                    title: "Add item",
                    onclick: move |_| {
                        if let Some(input) = web_sys::window()
                            .and_then(|w| w.document())
                            .and_then(|d| d.get_element_by_id("grocery-input"))
                        {
                            let _ = input.dyn_into::<web_sys::HtmlInputElement>()
                                .ok()
                                .map(|i| i.focus());
                        }
                    },
                    IconPlus {}
                }
            }
        }
    }
}

#[component]
fn GroupSection(
    name: String,
    items: Vec<GroceryItem>,
    mut collapsed: Signal<HashSet<String>>,
    mut items_sig: Signal<Vec<GroceryItem>>,
    mut error: Signal<String>,
) -> Element {
    let is_collapsed = collapsed.read().contains(&name);
    let key = name.clone();

    rsx! {
        button {
            key: "group-{key}",
            class: "grocery-group",
            onclick: move |_| {
                let name = name.clone();
                tracing::debug!("toggling grocery group {name:?}");
                collapsed.with_mut(|s| {
                    if !s.remove(&name) {
                        s.insert(name);
                    }
                });
            },
            span { class: "group-icon", IconMenu {} }
            span { class: "group-chevron", IconChevron { down: !is_collapsed } }
            span { class: "group-name", "{name}" }
        }
        if !is_collapsed {
            ul { class: "grocery-list",
                for item in items.iter().cloned() {
                    GroceryRow { key: "{item.id}", item: item, items_sig: items_sig, error: error }
                }
            }
        }
    }
}

#[component]
fn GroceryRow(
    item: GroceryItem,
    mut items_sig: Signal<Vec<GroceryItem>>,
    mut error: Signal<String>,
) -> Element {
    rsx! {
        li {
            id: "grocery-item-{item.id}",
            class: "grocery-item",
            button {
                class: "grocery-remove",
                r#type: "button",
                title: "Remove item",
                onclick: move |_| {
                    let id = item.id;
                    spawn(async move {
                        let client = reqwest::Client::new();
                        let url = format!("{}/api/grocery/{id}", api_base());
                        tracing::info!("Grocery remove: DELETE {url}");
                        let resp = client.delete(&url).send().await;
                        match resp {
                            Ok(r) if r.status().is_success() => {
                                tracing::info!("DELETE /api/grocery/{id} succeeded ({})", r.status());
                                items_sig.with_mut(|v| {
                                    v.retain(|i| i.id != id);
                                });
                            }
                            Ok(r) => {
                                tracing::error!("DELETE /api/grocery/{id} failed: {}", r.status());
                                error.set("Failed to remove item.".into());
                            }
                            Err(err) => {
                                tracing::error!("DELETE /api/grocery/{id} request failed: {err:#}");
                                error.set("Failed to remove item.".into());
                            }
                        }
                    });
                },
                IconX {}
            }
            span { "{item.name}" }
        }
    }
}

/// Re-fetch the grocery list from the backend.
async fn refresh(mut items: Signal<Vec<GroceryItem>>, mut error: Signal<String>) {
    match api_get::<Vec<GroceryItem>>("/api/grocery").await {
        Ok(list) => {
            tracing::debug!("grocery list refreshed: {} items", list.len());
            items.set(list);
            error.set(String::new());
        }
        Err(err) => {
            tracing::error!("grocery list refresh failed: {err:#}");
            error.set(err.to_string());
        }
    }
}

// ---------------------------------------------------------------------------
// Tabs: Meal plan & Settings
// ---------------------------------------------------------------------------

/// Local today as `YYYY-MM-DD` (the JS clock; wasm has no local TZ access).
fn today_iso() -> String {
    let date = js_sys::Date::new_0();
    format!(
        "{:04}-{:02}-{:02}",
        date.get_full_year(),
        date.get_month() + 1,
        date.get_date()
    )
}

/// `YYYY-MM-DD` shifted by `days` (negative goes back). Pure UTC millisecond
/// arithmetic: date-only strings parse as UTC midnight, so adding whole days
/// is exact in every timezone — a negative `days` must never wrap (the u32
/// cast in the old set_date version rolled negative sums into garbage dates,
/// duplicating whole day sections).
fn shift_iso(date: &str, days: i64) -> String {
    let millis = js_sys::Date::parse(date);
    let d = js_sys::Date::new(&((millis + (days as f64) * 86_400_000.0).into()));
    format!(
        "{:04}-{:02}-{:02}",
        d.get_utc_full_year(),
        d.get_utc_month() + 1,
        d.get_utc_date()
    )
}

/// Human label for a plan day: Today / Tomorrow / "Thu, Oct 1".
fn day_label(date: &str, today: &str) -> String {
    if date == today {
        return "Today".into();
    }
    if date == shift_iso(today, 1) {
        return "Tomorrow".into();
    }
    let millis = js_sys::Date::parse(date);
    if millis.is_nan() {
        return date.to_string();
    }
    let d = js_sys::Date::new(&millis.into());
    let weekday = d
        .to_locale_date_string("en-GB", &js_sys::Object::from(js_sys::JSON::parse("{\"weekday\":\"short\"}").unwrap()))
        ;
    let rest = d.to_locale_date_string(
        "en-GB",
        &js_sys::Object::from(js_sys::JSON::parse("{\"day\":\"numeric\",\"month\":\"short\"}").unwrap()),
    );
    format!("{weekday}, {rest}")
}

#[derive(Clone, PartialEq)]
struct PlanDay {
    /// `YYYY-MM-DD`.
    date: String,
    label: String,
    entries: Vec<MealPlanEntry>,
}

/// Which day the picker modal is adding to.
#[derive(Clone, PartialEq)]
struct PickerState {
    date: String,
    label: String,
}

#[component]
fn MealPlan() -> Element {
    let mut entries = use_signal(Vec::<MealPlanEntry>::new);
    let mut recipes = use_signal(Vec::<Recipe>::new);
    let mut error = use_signal(|| String::new());
    let mut loaded = use_signal(|| false);
    let mut picker = use_signal(|| None::<PickerState>);
    let navigator = use_navigator();
    let today = use_signal(today_iso);

    use_effect(move || {
        if !loaded() {
            loaded.set(true);
            spawn(async move {
                match api_get::<Vec<MealPlanEntry>>("/api/meal-plan").await {
                    Ok(list) => entries.set(list),
                    Err(err) => {
                        tracing::error!("meal plan refresh failed: {err:#}");
                        error.set(err.to_string());
                    }
                }
                match api_get::<Vec<Recipe>>("/api/recipes").await {
                    Ok(list) => recipes.set(list),
                    Err(err) => tracing::error!("recipes refresh failed: {err:#}"),
                }
            });
        }
    });

    // The rendered day range. It starts small (3 days back, 3 weeks ahead)
    // and grows a week at a time as the reader scrolls toward either end.
    let mut back = use_signal(|| 3i64);
    let mut forward = use_signal(|| 21i64);

    // Group every day in the range: entries grouped per day, empty days
    // render with their ⊕ so any day is plannable.
    let today_value = today.read().clone();
    let back_value = back.read().clone();
    let forward_value = forward.read().clone();
    let mut days: Vec<PlanDay> = ((-back_value)..=forward_value)
        .map(|offset| {
            let date = shift_iso(&today_value, offset);
            PlanDay {
                label: day_label(&date, &today_value),
                entries: entries
                    .read()
                    .iter()
                    .filter(|e| e.date == date)
                    .cloned()
                    .collect(),
                date,
            }
        })
        .collect();
    days.sort_by(|a, b| a.date.cmp(&b.date));

    // Infinite scroll: a light poll watches the content scroller. While the
    // reader is actively scrolling toward an end, the range grows a week at
    // a time to fill the timeline; prepends compensate the scroll position
    // so the view stays put. Polling (instead of scroll events) works in
    // every webview, including ones that swallow programmatic scroll events.
    #[cfg(target_arch = "wasm32")]
    use_effect(move || {
        spawn(async move {
            let mut last_top: i64 = 0;
            let mut pending: Option<(i64, i64)> = None; // (pre-extension height, user's scroll pos)
            loop {
                let promise = js_sys::Promise::new(&mut |resolve, _| {
                    if let Some(window) = web_sys::window() {
                        let _ = window
                            .set_timeout_with_callback_and_timeout_and_arguments_0(
                                &resolve,
                                250,
                            );
                    }
                });
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
                let Some(window) = web_sys::window() else { continue };
                let Some(document) = window.document() else { continue };
                let Some(element) = document
                    .get_element_by_id("content-scroller")
                    .and_then(|e| e.dyn_into::<web_sys::Element>().ok())
                else {
                    continue;
                };

                // A backward extension rendered since the last tick: restore
                // the reader's viewport over the prepended days.
                if let Some((pre_height, user_top)) = pending.take() {
                    let delta = element.scroll_height() as i64 - pre_height;
                    if delta > 0 {
                        element.set_scroll_top((user_top + delta) as i32);
                    }
                }

                let top = element.scroll_top() as i64;
                let height = element.scroll_height() as i64;
                let client = element.client_height() as i64;
                let bottom_dist = height - (top + client);
                let moving_up = top < last_top;

                if bottom_dist < 400 && forward() < 370 {
                    forward.set((forward() + 7).min(370));
                } else if moving_up && top < 400 && back() < 365 {
                    back.set(back + 7);
                    pending = Some((height, top));
                }
                last_top = top;
            }
        });
    });

    rsx! {
        div { class: "page",
            if !error.read().is_empty() {
                p { class: "status-error", "{error}" }
            }
            // The keyed day list lives in its own container: mixing a keyed
            // list with static siblings panics dioxus's differ when the
            // range grows.
            div { class: "plan-days", key: "{back_value}-{forward_value}",
                for day in days {
                    PlanDaySection {
                        day: day.clone(),
                        on_remove: move |entry_id: i64| {
                            spawn(async move {
                                let client = reqwest::Client::new();
                                match client
                                    .delete(format!("{}/api/meal-plan/{entry_id}", api_base()))
                                    .send()
                                    .await
                                {
                                    Ok(r) if r.status().is_success() => {
                                        entries.with_mut(|v| v.retain(|e| e.id != entry_id));
                                    }
                                    Ok(r) => tracing::error!("meal plan delete failed: {}", r.status()),
                                    Err(err) => tracing::error!("meal plan delete request failed: {err:#}"),
                                }
                            });
                        },
                        on_add: move |date: String| {
                            picker.set(Some(PickerState {
                                date,
                                label: day.label.clone(),
                            }));
                        },
                        on_open: move |recipe_id: i64| {
                            navigator.push(Route::RecipeDetail { id: recipe_id });
                        },
                    }
                }
            }

            if let Some(state) = picker.read().clone() {
                PlanPicker {
                    key: "{state.date}",
                    date: state.date.clone(),
                    label: state.label.clone(),
                    recipes: recipes.read().clone(),
                    on_add: move |(date, recipe_id): (String, i64)| {
                        picker.set(None);
                        spawn(async move {
                            let client = reqwest::Client::new();
                            match client
                                .post(format!("{}/api/meal-plan", api_base()))
                                .json(&serde_json::json!({ "date": date, "recipe_id": recipe_id }))
                                .send()
                                .await
                            {
                                Ok(resp) if resp.status().is_success() => {
                                    match resp.json::<MealPlanEntry>().await {
                                        Ok(entry) => entries.with_mut(|v| v.push(entry)),
                                        Err(err) => {
                                            tracing::error!("meal plan add: unreadable body: {err:#}")
                                        }
                                    }
                                }
                                Ok(resp) => {
                                    tracing::error!("meal plan add failed: {}", resp.status())
                                }
                                Err(err) => {
                                    tracing::error!("meal plan add request failed: {err:#}")
                                }
                            }
                        });
                    },
                }
            }
        }
    }
}

/// One day section of the meal plan: header (label + ⊕) and the day's
/// recipe cards.
#[component]
fn PlanDaySection(
    day: PlanDay,
    on_remove: EventHandler<i64>,
    on_add: EventHandler<String>,
    on_open: EventHandler<i64>,
) -> Element {
    let day_entries = day.entries;
    let day_is_empty = day_entries.is_empty();
    rsx! {
            div { class: "plan-day",
            div { class: "plan-day-head",
                span { class: "plan-day-label", "{day.label}" }
                button {
                    class: "plan-add",
                    r#type: "button",
                    title: "Add recipe",
                    onclick: move |_| on_add.call(day.date.clone()),
                    IconPlus {}
                }
            }
            div { class: "plan-cards",
                for entry in day_entries {
                    div { class: "plan-card",
                        key: "{entry.id}",
                        role: "button",
                        tabindex: "0",
                        onclick: move |_| on_open.call(entry.recipe.id),
                        button {
                            class: "plan-remove",
                            r#type: "button",
                            title: "Remove from plan",
                            onclick: move |ev: MouseEvent| {
                                ev.stop_propagation();
                                on_remove.call(entry.id);
                            },
                            IconX {}
                        }
                        div { class: "plan-card-media",
                            if let Some(thumb) = &entry.recipe.thumb {
                                img { src: "{thumb}", alt: "{entry.recipe.name}" }
                            } else if let Some(image) = &entry.recipe.image {
                                img { src: "{image}", alt: "{entry.recipe.name}" }
                            } else {
                                div { class: "plan-card-initials",
                                    {entry.recipe.name.split_whitespace().filter_map(|w| w.chars().next()).take(2).collect::<String>().to_uppercase()}
                                }
                            }
                        }
                        div { class: "plan-card-name", "{entry.recipe.name}" }
                    }
                }
                if day_is_empty {
                    span { class: "plan-empty", "No recipes" }
                }
            }
        }
    }
}

/// Search-and-pick dialog for adding a recipe to a plan day.
#[component]
fn PlanPicker(
    date: String,
    label: String,
    recipes: Vec<Recipe>,
    on_add: EventHandler<(String, i64)>,
) -> Element {
    let mut search = use_signal(String::new);
    let search_value = search.read().trim().to_lowercase();
    let matches: Vec<Recipe> = recipes
        .iter()
        .filter(|recipe| {
            search_value.is_empty() || recipe.name.to_lowercase().contains(&search_value)
        })
        .cloned()
        .collect();
    let matches_is_empty = matches.is_empty();
    let any_recipes = !recipes.is_empty();

    rsx! {
        div { class: "dialog-backdrop",
            onclick: move |_| {},
            div { class: "dialog plan-picker", role: "dialog",
                h2 { class: "dialog-title", "Add to {label}" }
                input {
                    id: "plan-search",
                    r#type: "text",
                    placeholder: "Search recipes…",
                    value: "{search}",
                    oninput: move |e: FormEvent| search.set(e.value()),
                }
                div { class: "plan-picker-list",
                div { class: "plan-picker-list",
                    for recipe in matches {
                        PlanPickerRow {
                            key: "{recipe.id}",
                            recipe: recipe.clone(),
                            date: date.clone(),
                            on_add: on_add.clone(),
                        }
                    }
                    if !any_recipes {
                        p { class: "empty", "No recipes yet — add some first." }
                    } else if matches_is_empty {
                        p { class: "empty", "No recipes match." }
                    }
                }
                }
            }
        }
    }
}

#[component]
fn Settings() -> Element {
    rsx! {
        PlaceholderPage {
            title: "Settings",
            text: "Theme, account and server settings — coming soon.",
        }
    }
}

#[component]
fn PlaceholderPage(title: String, text: String) -> Element {
    rsx! {
        div { class: "page",
            div { class: "card placeholder-card",
                h1 { "{title}" }
                p { "{text}" }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_levenshtein_distance() {
        // Basic tests
        assert_eq!(levenshtein_distance("", ""), 0);
        assert_eq!(levenshtein_distance("abc", ""), 3);
        assert_eq!(levenshtein_distance("", "abc"), 3);
        assert_eq!(levenshtein_distance("abc", "abc"), 0);
        assert_eq!(levenshtein_distance("abc", "abcd"), 1);
        assert_eq!(levenshtein_distance("abc", "abd"), 1); // c->d substitution
        assert_eq!(levenshtein_distance("abc", "axc"), 1); // b->x substitution
        assert_eq!(levenshtein_distance("abc", "a"), 2); // c and b deleted
        assert_eq!(levenshtein_distance("intention", "execution"), 5); // known distance
    }

    #[test]
    fn test_levenshtein_distance_unicode() {
        // Multi-byte characters count as one edit per character.
        assert_eq!(levenshtein_distance("café", "cafe"), 1);
        assert_eq!(levenshtein_distance("é", ""), 1);
        assert_eq!(levenshtein_distance("", "é"), 1);
        assert_eq!(levenshtein_distance("éé", "ée"), 1);
        // œ -> o,e is a substitution plus an insertion: true distance 2.
        assert_eq!(levenshtein_distance("œuf", "oeuf"), 2);
        // Same byte length but different characters.
        assert_eq!(levenshtein_distance(" café", " café"), 0);
    }

    #[test]
    fn test_rank_suggestions_empty_input() {
        let past = vec!["apple".to_string(), "banana".to_string(), "apricot".to_string()];
        // No typed text -> no suggestions (dropdown must stay closed).
        let result = rank_suggestions("", &past);
        assert!(result.is_empty());
        let result = rank_suggestions("   ", &past);
        assert!(result.is_empty());
    }

    #[test]
    fn test_rank_suggestions_fuzzy_matching() {
        let past = vec!["apple".to_string(), "appel".to_string(), "banana".to_string(), "apricot".to_string()];
        let result = rank_suggestions("apple", &past);
        // "apple" exact (dist 0), "appel" (dist 2), "banana" and "apricot" too far
        assert_eq!(result.len(), 2); // only apple and appel within distance 3
        let dists: Vec<usize> = result.iter().map(|(_, d)| *d).collect();
        assert_eq!(dists[0], 0);  // "apple" exact match
        assert_eq!(dists[1], 2);  // "appel" is 2 edits away
    }

    #[test]
    fn test_rank_suggestions_prefix_match() {
        let past = vec!["raspberry".to_string(), "apple".to_string()];
        let result = rank_suggestions("app", &past);
        // "app" is a prefix of "apple" -> ranked first with distance 0.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0], ("apple".to_string(), 0));
    }

    #[test]
    fn test_rank_suggestions_case_insensitive() {
        let past = vec!["Banana".to_string()];
        let result = rank_suggestions("bananna", &past);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "Banana");
    }

    #[test]
    fn test_rank_suggestions_no_match() {
        let past = vec!["xyz".to_string(), "qwert".to_string()];
        let result = rank_suggestions("apple", &past);
        // No matches within distance 3
        assert_eq!(result.len(), 0);
    }

    #[test]
    fn test_rank_suggestions_unicode_needle() {
        let past = vec!["café".to_string(), "thé".to_string()];
        let result = rank_suggestions("cafe", &past);
        // "café" is 1 edit away from "cafe"; "thé" is 4 edits away and is
        // correctly filtered out by the max-3 bound.
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].0, "café");
        assert_eq!(result[0].1, 1);
    }

    #[test]
    fn test_jump_target_for_maps_warnings_to_form_elements() {
        let no_qty = Ingredient {
            quantity: None,
            unit: None,
            name: "salt".into(),
            prep: None,
            section: None,
        };
        let with_qty = Ingredient {
            quantity: Some(1.0),
            unit: None,
            name: "eggs".into(),
            prep: None,
            section: None,
        };
        let ingredients = vec![with_qty, no_qty];

        // A quantity warning targets the rows named in the warning text.
        match jump_target_for(
            "1 ingredient quantity could not be parsed: \"pepper\"",
            &ingredients,
        ) {
            Some(JumpTarget::IngredientRows(names)) => {
                assert_eq!(names, vec!["pepper".to_string()]);
            }
            other => panic!("expected ingredient rows, got {other:?}"),
        }
        // Field warnings target their inputs/lists.
        assert_eq!(
            jump_target_for("recipe has no yield", &ingredients),
            Some(JumpTarget::Field("recipe-yield"))
        );
        assert_eq!(
            jump_target_for("recipe has no instructions", &ingredients),
            Some(JumpTarget::Field("step-rows"))
        );
        assert_eq!(
            jump_target_for("recipe has no ingredients", &ingredients),
            Some(JumpTarget::Field("ingredient-rows"))
        );
        // Informational warnings have no jump target.
        assert_eq!(
            jump_target_for("recipe was recovered from HTML instead of structured data", &ingredients),
            None
        );
        // A quantity warning with nothing left unmatched has no target.
        assert_eq!(jump_target_for("could not be parsed", &[]), None);
    }
}

/// One tappable recipe row inside the meal-plan picker dialog.
#[component]
fn PlanPickerRow(
    recipe: Recipe,
    date: String,
    on_add: EventHandler<(String, i64)>,
) -> Element {
    let row_date = date;
    rsx! {
        button {
            class: "plan-picker-row",
            r#type: "button",
            onclick: move |_| on_add.call((row_date.clone(), recipe.id)),
            if let Some(thumb) = &recipe.thumb {
                img { src: "{thumb}", alt: "" }
            }
            span { "{recipe.name}" }
        }
    }
}
