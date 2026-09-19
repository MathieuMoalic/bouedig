//! Bouedig mobile client (Dioxus on Android).

use std::collections::HashSet;

use dioxus::prelude::*;
use serde::de::DeserializeOwned;
use shared::{GroceryItem, GroceryUpdate, NewGroceryItem, NewRecipe, Recipe};

const WALLPAPER: Asset = asset!("/assets/background.avif");
const FAVICON: Asset = asset!("/assets/icon.png");

fn main() {
    // Logging + panic reporting first so nothing fails silently on-device.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();
    std::panic::set_hook(Box::new(|info| {
        tracing::error!("panic: {info}");
    }));
    tracing::info!("Bouedig mobile client starting");
    dioxus::launch(App);
}

#[derive(Clone, Routable, Debug, PartialEq)]
enum Route {
    #[layout(Layout)]
    #[route("/")]
    Recipes {},
    #[route("/add")]
    AddRecipe {},
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
// App shell: wallpaper + content + bottom navigation (blaz-style)
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

// ---------------------------------------------------------------------------
// API helpers
// ---------------------------------------------------------------------------

// A phone or emulator has no same-origin, so the backend address is absolute.
// 10.0.2.2 is the Android emulator alias for the host machine's loopback;
// change it to your LAN address (or reverse proxy URL) for real devices.
const API_BASE: &str = "http://10.0.2.2:3000";

fn api_base() -> String {
    API_BASE.to_string()
}

/// Recipe images arrive as server-relative URLs (`/api/images/...`); a
/// webview on a device needs them absolutised against the backend address.
fn absolutize(url: &str) -> String {
    if url.starts_with('/') {
        format!("{API_BASE}{url}")
    } else {
        url.to_string()
    }
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
    ingredients: &str,
    filename: &str,
    image: &[u8],
) -> (String, Vec<u8>) {
    let boundary = format!(
        "bouedig{}",
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0)
    );
    let mut body = Vec::new();
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"name\"\r\n\r\n{name}\r\n").as_bytes());
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"ingredients\"\r\n\r\n{ingredients}\r\n").as_bytes());
    body.extend_from_slice(
        format!("--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n").as_bytes(),
    );
    body.extend_from_slice(image);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

// ---------------------------------------------------------------------------
// Tab: Recipes (photo grid)
// ---------------------------------------------------------------------------

#[component]
fn Recipes() -> Element {
    let recipes = use_signal(Vec::<Recipe>::new);
    let error = use_signal(|| String::new());
    let mut loaded = use_signal(|| false);
    let navigator = use_navigator();

    use_effect(move || {
        if !loaded() {
            loaded.set(true);
            spawn(async move {
                refresh_recipes(recipes, error).await;
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
                    RecipeCard { recipe: recipe }
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
    let initials: String = recipe
        .name
        .split_whitespace()
        .filter_map(|w| w.chars().next())
        .take(2)
        .collect::<String>()
        .to_uppercase();

    let thumb_url = recipe.thumb.as_ref().map(|t| absolutize(t));

    rsx! {
        div { class: "recipe-card", id: "recipe-card-{recipe.id}",
            div { class: "recipe-card-media",
                if let Some(thumb) = thumb_url {
                    img { src: "{thumb}", loading: "lazy", alt: "{recipe.name}" }
                } else {
                    div { class: "recipe-placeholder", "{initials}" }
                }
                button { class: "card-fab", title: "Add to meal plan", IconCalendar {} }
            }
            div { class: "recipe-card-name", "{recipe.name}" }
        }
    }
}

/// Load the recipes grid from the backend.
async fn refresh_recipes(mut recipes: Signal<Vec<Recipe>>, mut error: Signal<String>) {
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
}

// ---------------------------------------------------------------------------
// Tab: Add recipe (opened via the + FAB)
// ---------------------------------------------------------------------------

#[component]
fn AddRecipe() -> Element {
    let mut name = use_signal(String::new);
    let mut ingredients = use_signal(String::new);
    let mut photo = use_signal(|| None::<(String, Vec<u8>)>);
    let mut status = use_signal(|| String::new());
    let mut status_error = use_signal(|| false);
    let navigator = use_navigator();

    let preview = shared::parse_ingredients(&ingredients.read()).join(", ");
    let photo_name = photo.read().as_ref().map(|(n, _)| n.clone());

    rsx! {
        div { class: "page",
            h1 { "Add a Recipe" }
            form {
                class: "card",
                onsubmit: move |e: FormEvent| {
                    e.prevent_default();
                    status_error.set(false);
                    let recipe = NewRecipe {
                        name: name.read().clone(),
                        ingredients: ingredients.read().clone(),
                    };
                    if recipe.name.trim().is_empty() {
                        status.set("Please enter a recipe name.".into());
                        status_error.set(true);
                        return;
                    }
                    let image = photo.read().clone();
                    spawn(async move {
                        let client = reqwest::Client::new();
                        let url = format!("{}/api/recipes", api_base());
                        tracing::info!(
                            "Add Recipe button: POST (name={:?}, ingredients={:?}, photo={})",
                            recipe.name,
                            recipe.ingredients,
                            image.is_some()
                        );
                        let result = match image {
                            Some((filename, bytes)) => {
                                let url = format!("{url}/photo");
                                let (content_type, body) =
                                    build_multipart(&recipe.name, &recipe.ingredients, &filename, &bytes);
                                client
                                    .post(url)
                                    .header("Content-Type", content_type)
                                    .body(body)
                                    .send()
                                    .await
                            }
                            None => client.post(url).json(&recipe).send().await,
                        };
                        match result {
                            Ok(r) if r.status().is_success() => {
                                tracing::info!("recipe saved ({})", r.status());
                                status.set("Recipe added! Ingredients moved to your grocery list.".into());
                                name.set(String::new());
                                ingredients.set(String::new());
                                photo.set(None);
                                // Back to the grid, which reloads automatically.
                                navigator.replace(Route::Recipes {});
                            }
                            Ok(r) => {
                                let msg = format!("Server error: {}", r.status());
                                tracing::error!("POST recipe failed: {msg}");
                                status.set(msg);
                                status_error.set(true);
                            }
                            Err(err) => {
                                tracing::error!("POST recipe request failed: {err:#}");
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
                    value: "{name}",
                    placeholder: "e.g. Pancakes",
                    oninput: move |e: FormEvent| name.set(e.value()),
                }
                label { "Ingredients" }
                textarea {
                    id: "recipe-ingredients",
                    value: "{ingredients}",
                    placeholder: "Flour, Milk\nEggs (comma or newline separated)",
                    oninput: move |e: FormEvent| ingredients.set(e.value()),
                }
                label { "Photo (optional)" }
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
                if let Some(fname) = photo_name {
                    p { class: "hint", "Selected: {fname}" }
                }
                p { class: "hint", "Will be added to the grocery list: {preview}" }
                button { id: "recipe-submit", r#type: "submit", "Add Recipe" }
            }
            p { id: "recipe-status", class: if status_error() { "error" } else { "" }, "{status}" }
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
                p { class: "empty", "Your grocery list is empty. Add a recipe or an item above." }
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
                        tracing::info!("Grocery + FAB clicked");
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
