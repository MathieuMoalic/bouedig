//! Bouedig web client (Dioxus + WASM).

use dioxus::prelude::*;
use serde::de::DeserializeOwned;
use shared::{GroceryItem, GroceryUpdate, NewGroceryItem, NewRecipe};

fn main() {
    dioxus::launch(App);
}

#[derive(Clone, Routable, Debug, PartialEq)]
enum Route {
    #[layout(Layout)]
    #[route("/")]
    AddRecipe {},
    #[route("/grocery")]
    Grocery {},
}

#[component]
fn App() -> Element {
    rsx! {
        Router::<Route> {}
    }
}

#[component]
fn Layout() -> Element {
    rsx! {
        style { include_str!("../assets/style.css") }
        div { class: "app",
            header { class: "navbar",
                nav {
                    Link { to: Route::AddRecipe, class: "nav-link", "Add Recipe" }
                    Link { to: Route::Grocery, class: "nav-link", "Grocery List" }
                }
            }
            main { class: "content",
                Outlet::<Route> {}
            }
        }
    }
}

// ---------------------------------------------------------------------------
// API helpers (same-origin `/api` works in dev via the dx proxy and in
// production behind the reverse proxy).
// ---------------------------------------------------------------------------

async fn api_get<T: DeserializeOwned>(path: &str) -> anyhow::Result<T> {
    let resp = reqwest::get(path).await?;
    let status = resp.status();
    let body = resp.text().await?;
    if !status.is_success() {
        anyhow::bail!("GET {path} failed: {status} ({body})");
    }
    Ok(serde_json::from_str(&body)?)
}

// ---------------------------------------------------------------------------
// Tab 1: Add Recipe
// ---------------------------------------------------------------------------

#[component]
fn AddRecipe() -> Element {
    let mut name = use_signal(String::new);
    let mut ingredients = use_signal(String::new);
    let mut status = use_signal(|| String::new());
    let mut status_error = use_signal(|| false);

    let preview = shared::parse_ingredients(&ingredients.read()).join(", ");

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
                    spawn(async move {
                        let client = reqwest::Client::new();
                        let resp = client.post("/api/recipes").json(&recipe).send().await;
                        match resp {
                            Ok(r) if r.status().is_success() => {
                                status.set("Recipe added! Ingredients moved to your grocery list.".into());
                                name.set(String::new());
                                ingredients.set(String::new());
                            }
                            Ok(r) => {
                                status.set(format!("Server error: {}", r.status()));
                                status_error.set(true);
                            }
                            Err(err) => {
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
                p { class: "hint", "Will be added to the grocery list: {preview}" }
                button { id: "recipe-submit", r#type: "submit", "Add Recipe" }
            }
            p { id: "recipe-status", class: if status_error() { "error" } else { "" }, "{status}" }
        }
    }
}

// ---------------------------------------------------------------------------
// Tab 2: Grocery List
// ---------------------------------------------------------------------------

#[component]
fn Grocery() -> Element {
    let mut items = use_signal(Vec::<GroceryItem>::new);
    let mut new_item = use_signal(String::new);
    let mut error = use_signal(|| String::new());
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
    let empty = list.is_empty();

    rsx! {
        div { class: "page",
            h1 { "Grocery List" }
            div { class: "add-row",
                input {
                    id: "grocery-input",
                    r#type: "text",
                    value: "{new_item}",
                    placeholder: "Add an item manually…",
                    oninput: move |e: FormEvent| new_item.set(e.value()),
                }
                button {
                    id: "grocery-add",
                    onclick: move |_| {
                        let item = NewGroceryItem { name: new_item.read().clone() };
                        if item.name.trim().is_empty() {
                            return;
                        }
                        new_item.set(String::new());
                        spawn(async move {
                            let client = reqwest::Client::new();
                            let resp = client.post("/api/grocery").json(&item).send().await;
                            if resp.is_err() {
                                error.set("Failed to add item.".into());
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
            if empty {
                p { class: "empty", "Your grocery list is empty. Add a recipe or an item above." }
            } else {
                ul { id: "grocery-list", class: "grocery-list",
                    for item in list {
                        GroceryRow { item: item, items: items, error: error }
                    }
                }
            }
        }
    }
}

#[component]
fn GroceryRow(item: GroceryItem, mut items: Signal<Vec<GroceryItem>>, mut error: Signal<String>) -> Element {
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
                        let resp = client
                            .patch(format!("/api/grocery/{id}"))
                            .json(&update)
                            .send()
                            .await;
                        match resp {
                            Ok(r) if r.status().is_success() => {
                                items.with_mut(|v| {
                                    if let Some(it) = v.iter_mut().find(|i| i.id == id) {
                                        it.bought = update.bought;
                                    }
                                });
                            }
                            _ => error.set("Failed to update item.".into()),
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
            items.set(list);
            error.set(String::new());
        }
        Err(err) => error.set(err.to_string()),
    }
}
