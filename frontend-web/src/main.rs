//! Bouedig web client (Dioxus + WASM).

use std::collections::HashSet;

use dioxus::prelude::*;
use serde::de::DeserializeOwned;
use wasm_bindgen::JsCast;
use shared::{
    GroceryItem, GroceryUpdate, Ingredient, NewGroceryItem, Recipe,
    RecipeDetail as RecipeDetailModel, RecipeInput,
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
    ingredients_json: &str,
    instructions_json: &str,
    image: Option<(&str, &[u8])>,
) -> (String, Vec<u8>) {
    // `SystemTime::now` is not implemented on wasm, use the JS clock.
    let boundary = format!("bouedig{}", js_sys::Date::now() as u64);
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"ingredients\"\r\n\r\n{ingredients_json}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"instructions\"\r\n\r\n{instructions_json}\r\n").as_bytes());
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
                button { class: "fab small", title: "Filter", IconList {} }
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

/// In-progress ingredient fields as raw strings (parsed on submit).
#[derive(Clone, Default, PartialEq)]
struct IngredientDraft {
    quantity: String,
    unit: String,
    name: String,
    prep: String,
}

/// In-progress ingredient row. `initial` seeds the input's `value` attribute
/// exactly once (dioxus only rewrites changed attributes, so a value bound to
/// live state would fight the caret while typing); `draft` mirrors what the
/// user typed, purely for parsing on submit.
#[derive(Clone, PartialEq)]
struct IngredientRow {
    id: u64,
    initial: IngredientDraft,
    draft: IngredientDraft,
}

impl IngredientRow {
    fn empty(id: u64) -> Self {
        Self { id, initial: IngredientDraft::default(), draft: IngredientDraft::default() }
    }
    fn from_draft(id: u64, draft: IngredientDraft) -> Self {
        Self { id, initial: draft.clone(), draft }
    }
}

/// In-progress text field: static initial value + live mirror for parsing.
#[derive(Clone, Default, PartialEq)]
struct TextField {
    initial: String,
    draft: String,
}

impl TextField {
    fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        Self { initial: value.clone(), draft: value }
    }
}

fn parse_drafts(rows: &[IngredientRow]) -> Vec<Ingredient> {
    rows.iter()
        .filter(|r| !r.draft.name.trim().is_empty())
        .map(|r| Ingredient {
            quantity: r.draft.quantity.trim().parse::<f64>().ok(),
            unit: (!r.draft.unit.trim().is_empty()).then(|| r.draft.unit.trim().to_string()),
            name: r.draft.name.trim().to_string(),
            prep: (!r.draft.prep.trim().is_empty()).then(|| r.draft.prep.trim().to_string()),
        })
        .collect()
}

fn parse_instructions(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect()
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
            RecipeFormFields { initial: RecipeDetailModel::default(), editing_id: None }
        },
        (Some(_), Some(initial)) => rsx! {
            RecipeFormFields { initial: initial, editing_id: editing }
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

#[component]
fn RecipeFormFields(initial: RecipeDetailModel, editing_id: Option<i64>) -> Element {
    let mut next_row_id = use_signal(|| initial.ingredients.len() as u64);
    let mut name = use_signal(|| TextField::new(initial.name.clone()));
    let mut rows = use_signal(|| {
        if initial.ingredients.is_empty() {
            vec![IngredientRow::empty(0)]
        } else {
            initial
                .ingredients
                .iter()
                .enumerate()
                .map(|(i, ingredient)| {
                    IngredientRow::from_draft(
                        i as u64,
                        IngredientDraft {
                            quantity: ingredient.quantity.map(fmt_qty).unwrap_or_default(),
                            unit: ingredient.unit.clone().unwrap_or_default(),
                            name: ingredient.name.clone(),
                            prep: ingredient.prep.clone().unwrap_or_default(),
                        },
                    )
                })
                .collect()
        }
    });
    let mut instructions_text = use_signal(|| TextField::new(initial.instructions.join("\n")));
    let mut photo = use_signal(|| None::<(String, Vec<u8>)>);
    let mut status = use_signal(|| String::new());
    let mut status_error = use_signal(|| false);
    let navigator = use_navigator();

    let existing_thumb = initial.thumb.clone();

    rsx! {
        div { class: "page",
            h1 { if editing_id.is_some() { "Edit Recipe" } else { "Add a Recipe" } }
            form {
                class: "card",
                onsubmit: move |e: FormEvent| {
                    e.prevent_default();
                    status_error.set(false);
                    let ingredients = parse_drafts(&rows.read());
                    let input = RecipeInput {
                        name: name.read().draft.trim().to_string(),
                        ingredients: ingredients.clone(),
                        instructions: parse_instructions(&instructions_text.read().draft),
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
                        let instructions_json = serde_json::to_string(&input.instructions).unwrap_or_default();

                        let result = match (editing_id, image) {
                            (None, None) => client
                                .post(format!("{base}/api/recipes"))
                                .json(&input)
                                .send()
                                .await
                                .map(|r| (r, None)),
                            (None, Some((filename, bytes))) => {
                                let (ct, body) = build_multipart(
                                    &input.name, &ingredients_json, &instructions_json,
                                    Some((&filename, &bytes)),
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
                                let image_ref = image.as_ref()
                                    .map(|(f, b)| (f.as_str(), b.as_slice()));
                                let (ct, body) = build_multipart(
                                    &input.name, &ingredients_json, &instructions_json, image_ref,
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
                                // Detailed responses carry the id; navigate there.
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
                    value: "{name.read().draft}",
                    placeholder: "e.g. Pancakes",
                    oninput: move |e: FormEvent| {
                        let v = e.value();
                        name.with_mut(|f| f.draft = v);
                    },
                }

                label { "Ingredients" }
                div { id: "ingredient-rows", class: "ingredient-rows",
                    for row in rows.read().clone() {
                        div { class: "ingredient-row", key: "{row.id}",
                            input {
                                id: "ing-qty-{row.id}",
                                class: "ing-qty",
                                r#type: "text",
                                value: "{row.draft.quantity}",
                                placeholder: "Qty",
                                oninput: move |e: FormEvent| {
                                    let v = e.value();
                                    let id = row.id;
                                    rows.with_mut(|r| {
                                        if let Some(row) = r.iter_mut().find(|r| r.id == id) {
                                            row.draft.quantity = v;
                                        }
                                    });
                                },
                            }
                            input {
                                id: "ing-unit-{row.id}",
                                class: "ing-unit",
                                r#type: "text",
                                value: "{row.draft.unit}",
                                placeholder: "Unit",
                                oninput: move |e: FormEvent| {
                                    let v = e.value();
                                    let id = row.id;
                                    rows.with_mut(|r| {
                                        if let Some(row) = r.iter_mut().find(|r| r.id == id) {
                                            row.draft.unit = v;
                                        }
                                    });
                                },
                            }
                            input {
                                id: "ing-name-{row.id}",
                                class: "ing-name",
                                r#type: "text",
                                value: "{row.draft.name}",
                                placeholder: "Ingredient",
                                oninput: move |e: FormEvent| {
                                    let v = e.value();
                                    let id = row.id;
                                    rows.with_mut(|r| {
                                        if let Some(row) = r.iter_mut().find(|r| r.id == id) {
                                            row.draft.name = v;
                                        }
                                    });
                                },
                            }
                            input {
                                id: "ing-prep-{row.id}",
                                class: "ing-prep",
                                r#type: "text",
                                value: "{row.draft.prep}",
                                placeholder: "Prep",
                                oninput: move |e: FormEvent| {
                                    let v = e.value();
                                    let id = row.id;
                                    rows.with_mut(|r| {
                                        if let Some(row) = r.iter_mut().find(|r| r.id == id) {
                                            row.draft.prep = v;
                                        }
                                    });
                                },
                            }
                            button {
                                id: "ing-remove-{row.id}",
                                class: "ing-remove",
                                title: "Remove ingredient",
                                r#type: "button",
                                onclick: move |_| {
                                    let id = row.id;
                                    tracing::debug!("removing ingredient row {id}");
                                    rows.with_mut(|r| r.retain(|r| r.id != id));
                                    if rows.read().is_empty() {
                                        rows.with_mut(|r| r.push(IngredientRow::empty(0)));
                                    }
                                },
                                "×"
                            }
                        }
                    }
                }
                button {
                    id: "add-ingredient",
                    class: "secondary-btn",
                    r#type: "button",
                    onclick: move |_| {
                        tracing::debug!("adding ingredient row");
                        let id = next_row_id.read().max(1);
                        rows.with_mut(|r| r.push(IngredientRow::empty(id)));
                        next_row_id.set(id + 1);
                    },
                    "+ Add ingredient"
                }

                label { "Instructions (one step per line)" }
                textarea {
                    id: "recipe-instructions",
                    value: "{instructions_text.read().draft}",
                    placeholder: "1. Mix the batter\n2. Cook in a hot pan",
                    oninput: move |e: FormEvent| {
                        let v = e.value();
                        instructions_text.with_mut(|f| f.draft = v);
                    },
                }

                label { "Photo (optional)" }
                if let Some(thumb) = &existing_thumb {
                    if editing_id.is_some() {
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
                    if editing_id.is_some() { "Save Changes" } else { "Add Recipe" }
                }
            }
            p { id: "recipe-status", class: if status_error() { "error" } else { "" }, "{status}" }
        }
    }
}

// ---------------------------------------------------------------------------
// Tab: Recipe detail (blaz-style)
// ---------------------------------------------------------------------------

#[component]
fn RecipeDetail(id: i64) -> Element {
    let mut detail = use_signal(|| None::<RecipeDetailModel>);
    let mut error = use_signal(|| String::new());
    let mut confirm_delete = use_signal(|| false);
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
                button { class: "hdr-btn ph", title: "Add to shopping list", IconCart {} }
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

            match loaded.as_ref() {
                Some(d) => rsx! {
                    h1 { class: "detail-name", "{d.name}" }
                    if let Some(url) = &image_url {
                        div { class: "detail-photo",
                            img { src: "{url}", alt: "{d.name}" }
                        }
                    }
                    div { class: "card",
                        h2 { "Ingredients" }
                        if d.ingredients.is_empty() {
                            p { class: "empty", "No ingredients yet." }
                        } else {
                            ul { class: "ingredient-list", id: "ingredient-list",
                                for ingredient in &d.ingredients {
                                    li { class: "ingredient-item", "{ingredient_line(ingredient)}" }
                                }
                            }
                        }
                    }
                    div { class: "card",
                        h2 { "Instructions" }
                        if d.instructions.is_empty() {
                            p { class: "empty", "No instructions yet." }
                        } else {
                            ol { class: "instruction-list", id: "instruction-list",
                                for (i, step) in d.instructions.iter().enumerate() {
                                    li { key: "{i}", "{step}" }
                                }
                            }
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
        }
    }
}

/// One ingredient line, e.g. `180 g buckwheat flour, finely chopped`.
fn ingredient_line(ingredient: &Ingredient) -> String {
    let mut line = String::new();
    if let Some(q) = ingredient.quantity {
        line.push_str(&fmt_qty(q));
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

// ---------------------------------------------------------------------------
// Tab: Shopping (grouped grocery list)
// ---------------------------------------------------------------------------

#[component]
fn Grocery() -> Element {
    let items = use_signal(Vec::<GroceryItem>::new);
    let mut new_item = use_signal(String::new);
    let mut new_category = use_signal(String::new);
    let mut error = use_signal(|| String::new());
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
    for item in list {
        match groups.iter_mut().find(|(c, _)| *c == item.category) {
            Some((_, v)) => v.push(item),
            None => groups.push((item.category.clone(), vec![item])),
        }
    }

    rsx! {
        div { class: "page",
            div { class: "add-row",
                input {
                    id: "grocery-input",
                    r#type: "text",
                    value: "{new_item}",
                    placeholder: "Add an item manually…",
                    oninput: move |e: FormEvent| new_item.set(e.value()),
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
                            let resp = client.post(url).json(&item).send().await;
                            match &resp {
                                Ok(r) if r.status().is_success() => {
                                    tracing::info!("POST /api/grocery succeeded ({})", r.status());
                                }
                                Ok(r) => tracing::error!("POST /api/grocery failed: {}", r.status()),
                                Err(err) => {
                                    tracing::error!("POST /api/grocery request failed: {err:#}");
                                    error.set("Failed to add item.".into());
                                }
                            }
                            refresh(items, error).await;
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
            class: if item.bought { "grocery-item bought" } else { "grocery-item" },
            input {
                r#type: "checkbox",
                class: "grocery-check",
                id: "grocery-check-{item.id}",
                checked: item.bought,
                onclick: move |_| {
                    let id = item.id;
                    let update = GroceryUpdate { bought: !item.bought };
                    spawn(async move {
                        let client = reqwest::Client::new();
                        let url = format!("{}/api/grocery/{id}", api_base());
                        tracing::info!("Grocery checkbox: PATCH {url} (bought={})", update.bought);
                        let resp = client.patch(url).json(&update).send().await;
                        match resp {
                            Ok(r) if r.status().is_success() => {
                                tracing::info!("PATCH /api/grocery/{id} succeeded ({})", r.status());
                                items_sig.with_mut(|v| {
                                    if let Some(it) = v.iter_mut().find(|i| i.id == id) {
                                        it.bought = update.bought;
                                    }
                                });
                            }
                            Ok(r) => {
                                tracing::error!("PATCH /api/grocery/{id} failed: {}", r.status());
                                error.set("Failed to update item.".into());
                            }
                            Err(err) => {
                                tracing::error!("PATCH /api/grocery/{id} request failed: {err:#}");
                                error.set("Failed to update item.".into());
                            }
                        }
                    });
                },
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
// Tabs: Meal plan & Settings (placeholders)
// ---------------------------------------------------------------------------

#[component]
fn MealPlan() -> Element {
    rsx! {
        PlaceholderPage {
            title: "Meal plan",
            text: "Plan your week — coming soon.",
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
