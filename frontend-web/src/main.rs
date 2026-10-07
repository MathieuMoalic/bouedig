//! Bouedig web client (Dioxus + WASM).

use std::collections::{HashMap, HashSet};

use dioxus::prelude::*;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use shared::{
    GroceryItem, GroceryPatch, GroceryUpdate, Ingredient, InstructionStep, MealPlanEntry,
    NewGroceryBatch, NewGroceryItem, Recipe, RecipeDetail as RecipeDetailModel, RecipeInput,
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
    #[route("/import-image")]
    ImportImage {},
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

#[component]
fn IconSort() -> Element {
    rsx! {
        // Arrow-down-wide-narrow: the widest bar shrinks stepwise — reads as
        // "sort descending" at a glance.
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.8", "stroke-linecap": "round", "stroke-linejoin": "round",
            path { d: "m3 16 4 4 4-4" }
            path { d: "M7 4v16" }
            path { d: "M11 4h10" }
            path { d: "M11 8h7" }
            path { d: "M11 12h4" }
        }
    }
}

#[component]
fn IconImage() -> Element {
    rsx! {
        svg { class: "icon", view_box: "0 0 24 24", fill: "none", stroke: "currentColor", "stroke-width": "1.8", "stroke-linecap": "round", "stroke-linejoin": "round",
            rect { x: "3", y: "4", width: "18", height: "16", rx: "2" }
            circle { cx: "9", cy: "10", r: "1.6" }
            path { d: "M5 19l5.5-6 4 4.5L18 14l3 5" }
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

/// Grid sort modes; the active one persists through the settings API.
#[derive(Clone, Copy, PartialEq, Debug)]
enum SortMode {
    NameAsc,
    NameDesc,
    Updated,
    Random,
}

impl SortMode {
    fn setting_value(self) -> &'static str {
        match self {
            SortMode::NameAsc => "name_asc",
            SortMode::NameDesc => "name_desc",
            SortMode::Updated => "updated",
            SortMode::Random => "random",
        }
    }

    fn from_setting(value: &str) -> Option<Self> {
        match value {
            "name_asc" => Some(SortMode::NameAsc),
            "name_desc" => Some(SortMode::NameDesc),
            "updated" => Some(SortMode::Updated),
            "random" => Some(SortMode::Random),
            _ => None,
        }
    }

    fn label(self) -> &'static str {
        match self {
            SortMode::NameAsc => "Name (A–Z)",
            SortMode::NameDesc => "Name (Z–A)",
            SortMode::Updated => "Recently updated",
            SortMode::Random => "Random",
        }
    }
}

/// Body of `GET/PUT /api/settings/{key}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SettingValue {
    value: String,
}

/// Milliseconds since the epoch — random seeds and nothing else.
fn now_millis() -> u64 {
    #[cfg(target_arch = "wasm32")]
    {
        js_sys::Date::now() as u64
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0)
    }
}

/// `GET /api/session` → am I authenticated? 401 and network errors both mean
/// "no" (with auth off the endpoint always says yes).
async fn session_authenticated() -> bool {
    match reqwest::get(format!("{}/api/session", api_base())).await {
        Ok(response) => response.status().is_success(),
        Err(_) => false,
    }
}

/// Gate tab content behind the household login: children render only when
/// authenticated, otherwise a password form is shown.
#[component]
fn LoginGate(children: Element) -> Element {
    let mut authed = use_signal(|| None::<bool>);
    // Children (Settings) get the login state so they can log out.
    provide_context(authed);
    let mut password = use_signal(String::new);
    let mut error = use_signal(|| String::new());
    let mut busy = use_signal(|| false);

    use_effect(move || {
        spawn(async move {
            authed.set(Some(session_authenticated().await));
        });
    });

    let mut submit = move |_| {
        let target = password.read().trim().to_string();
        if target.is_empty() || *busy.read() {
            return;
        }
        busy.set(true);
        error.set(String::new());
        spawn(async move {
            let client = reqwest::Client::new();
            let url = format!("{}/api/login", api_base());
            match client
                .post(&url)
                .json(&serde_json::json!({ "password": target }))
                .send()
                .await
            {
                Ok(r) if r.status().is_success() => {
                    authed.set(Some(true));
                    password.set(String::new());
                }
                Ok(r) if r.status() == reqwest::StatusCode::UNAUTHORIZED => {
                    error.set("Wrong password.".into());
                }
                Ok(r) => error.set(format!("Login failed: {}", r.status())),
                Err(err) => error.set(format!("Could not reach the server: {err}")),
            }
            busy.set(false);
        });
    };

    match authed() {
        None => rsx! {
            div { class: "page",
                div { class: "card placeholder-card",
                    p { class: "empty", "Checking…" }
                }
            }
        },
        Some(true) => children,
        Some(false) => rsx! {
            div { class: "page",
                div { class: "card login-card",
                    h2 { "This area is private" }
                    p { class: "muted", "Enter the household password to continue." }
                    input {
                        id: "login-password",
                        r#type: "password",
                        placeholder: "Password",
                        value: "{password}",
                        oninput: move |e: FormEvent| password.set(e.value()),
                        onkeydown: move |e: KeyboardEvent| {
                            if e.key() == Key::Enter {
                                submit(());
                            }
                        },
                    }
                    if !error.read().is_empty() {
                        p { class: "status-error", "{error}" }
                    }
                    button {
                        id: "login-submit",
                        class: "btn-primary",
                        r#type: "button",
                        disabled: *busy.read(),
                        onclick: move |_| submit(()),
                        if *busy.read() { "Signing in…" } else { "Sign in" }
                    }
                }
            }
        },
    }
}

/// Deterministic in-place shuffle (xorshift): stable across re-renders for
/// one seed, and every seed press is a brand-new order.
fn seeded_shuffle(list: &mut [Recipe], seed: u64) {
    if list.len() < 2 {
        return;
    }
    let mut state = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    for i in (1..list.len()).rev() {
        let j = (next() % (i as u64 + 1)) as usize;
        list.swap(i, j);
    }
}

/// Sleep that works on wasm (js timer) and natively (thread sleep — the web
/// client never actually runs natively, this exists so the crate compiles
/// and its unit tests run on the host).
async fn sleep_ms(ms: i32) {
    #[cfg(target_arch = "wasm32")]
    {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            if let Some(window) = web_sys::window() {
                let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
            }
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::sleep(std::time::Duration::from_millis(ms.max(0) as u64));
    }
}

/// Best-effort clipboard write (the async clipboard API); `false` when the
/// browser refuses (permissions, non-secure context).
#[cfg(target_arch = "wasm32")]
async fn copy_clipboard(text: String) -> bool {
    let Some(window) = web_sys::window() else {
        return false;
    };
    let Ok(navigator) = js_sys::Reflect::get(&window.into(), &"navigator".into()) else {
        return false;
    };
    let Ok(clipboard) = js_sys::Reflect::get(&navigator, &"clipboard".into()) else {
        return false;
    };
    let Ok(write_text) = js_sys::Reflect::get(&clipboard, &"writeText".into()) else {
        return false;
    };
    let Ok(write_text) = write_text.dyn_into::<js_sys::Function>() else {
        return false;
    };
    match write_text.call1(&clipboard, &text.into()) {
        Ok(promise) => {
            let promise: js_sys::Promise = promise.into();
            wasm_bindgen_futures::JsFuture::from(promise).await.is_ok()
        }
        Err(_) => false,
    }
}

/// Native fallback so the crate compiles on the host (for unit tests); the
/// web client only ever runs as wasm.
#[cfg(not(target_arch = "wasm32"))]
async fn copy_clipboard(_text: String) -> bool {
    false
}

#[component]
fn Recipes() -> Element {
    let mut recipes = use_signal(Vec::<Recipe>::new);
    let mut error = use_signal(|| String::new());
    let mut loaded = use_signal(|| false);
    let navigator = use_navigator();

    let mut sort_mode = use_signal(|| SortMode::Updated);
    let mut random_seed = use_signal(|| 0u64);
    let mut add_menu = use_signal(|| false);
    let mut sort_menu = use_signal(|| false);
    let mut search_open = use_signal(|| false);
    let mut search_text = use_signal(|| String::new());
    let mut search_results = use_signal(|| None::<Vec<Recipe>>);
    let mut search_generation = use_signal(|| 0u64);

    use_effect(move || {
        if !loaded() {
            loaded.set(true);
            // Fresh random seed per mount; Random presses bump it again.
            random_seed.set(now_millis());
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
                // Restore the persisted sort choice (404 = keep the default).
                if let Ok(mode) = api_get::<SettingValue>("/api/settings/recipes_sort").await {
                    if let Some(mode) = SortMode::from_setting(&mode.value) {
                        sort_mode.set(mode);
                    }
                }
            });
        }
    });

    // Live search: debounce 300 ms, then fetch ranked matches. Stale
    // generations are dropped so typing fast never shows old results.
    use_effect(move || {
        let text = search_text.read().trim().to_string();
        if text.is_empty() {
            search_results.set(None);
            return;
        }
        search_generation += 1;
        let generation = search_generation();
        spawn(async move {
            sleep_ms(300).await;
            if generation != search_generation() {
                return;
            }
            let client = reqwest::Client::new();
            let url = format!("{}/api/recipes/search", api_base());
            if let Ok(response) = client.get(&url).query(&[("q", &text)]).send().await {
                if response.status().is_success() {
                    if let Ok(list) = response.json::<Vec<Recipe>>().await {
                        if generation == search_generation() {
                            search_results.set(Some(list));
                        }
                    }
                }
            }
        });
    });

    let display = use_memo(move || {
        let mut list = recipes.read().clone();
        match sort_mode() {
            SortMode::NameAsc => list.sort_by(|a, b| {
                a.name.to_lowercase().cmp(&b.name.to_lowercase())
            }),
            SortMode::NameDesc => list.sort_by(|a, b| {
                b.name.to_lowercase().cmp(&a.name.to_lowercase())
            }),
            SortMode::Updated => list.sort_by(|a, b| b.updated_at.cmp(&a.updated_at)),
            SortMode::Random => seeded_shuffle(&mut list, random_seed()),
        }
        list
    });
    let searching = search_text.read().trim().is_empty();
    let list = if searching {
        display.read().clone()
    } else {
        search_results.read().clone().unwrap_or_default()
    };

    let mut persist_sort = move |mode: SortMode| {
        sort_mode.set(mode);
        let payload = SettingValue {
            value: mode.setting_value().to_string(),
        };
        spawn(async move {
            let client = reqwest::Client::new();
            let url = format!("{}/api/settings/recipes_sort", api_base());
            let _ = client.put(&url).json(&payload).send().await;
        });
    };
    let sort_options: Vec<(SortMode, &'static str, &'static str, bool)> = [
        SortMode::NameAsc,
        SortMode::NameDesc,
        SortMode::Updated,
        SortMode::Random,
    ]
    .iter()
    .map(|mode| {
        (
            *mode,
            mode.setting_value(),
            mode.label(),
            *mode == sort_mode(),
        )
    })
    .collect();

    rsx! {
        div { class: "page",
            if !error.read().is_empty() {
                p { class: "status-error", "{error}" }
            }
            if search_open() {
                div { class: "search-bar",
                    input {
                        id: "recipe-search",
                        r#type: "text",
                        placeholder: "Search recipes…",
                        value: "{search_text}",
                        oninput: move |e: FormEvent| search_text.set(e.value()),
                    }
                    button {
                        id: "search-close",
                        class: "grocery-remove",
                        title: "Close search",
                        onclick: move |_| {
                            search_text.set(String::new());
                            search_results.set(None);
                            search_open.set(false);
                        },
                        IconX {}
                    }
                }
            }
            if list.is_empty() && !search_open() {
                p { class: "empty", "No recipes yet. Tap + to add your first one." }
            }
            if search_open() && list.is_empty() && !searching {
                p { class: "empty", "Nothing matches." }
            }
            div { id: "recipe-grid", class: "recipe-grid",
                for recipe in list {
                    RecipeCard {
                        key: "{recipe.id}",
                        recipe: recipe,
                    }
                }
            }

            if add_menu() {
                div { class: "menu-backdrop", onclick: move |_| add_menu.set(false) }
                div { class: "fab-menu fab-menu-add",
                    button {
                        id: "fab-menu-manual",
                        onclick: move |_| {
                            add_menu.set(false);
                            navigator.push(Route::AddRecipe {});
                        },
                        IconPlus {}
                        span { "Add manually" }
                    }
                    button {
                        id: "fab-menu-import-url",
                        onclick: move |_| {
                            add_menu.set(false);
                            navigator.push(Route::ImportRecipe {});
                        },
                        IconGlobe {}
                        span { "Import from URL" }
                    }
                    button {
                        id: "fab-menu-import-image",
                        onclick: move |_| {
                            add_menu.set(false);
                            navigator.push(Route::ImportImage {});
                        },
                        IconImage {}
                        span { "Import from image" }
                    }
                }
            }
            if sort_menu() {
                div { class: "menu-backdrop", onclick: move |_| sort_menu.set(false) }
                div { class: "fab-menu fab-menu-sort",
                    for (mode, value, label, selected) in sort_options.clone().into_iter() {
                        button {
                            id: "sort-{value}",
                            class: if selected { "selected" } else { "" },
                            onclick: move |_| {
                                sort_menu.set(false);
                                if mode == SortMode::Random {
                                    random_seed.set(now_millis());
                                }
                                persist_sort(mode);
                            },
                            span { class: "menu-check", if selected { "✓" } else { "" } }
                            span { "{label}" }
                        }
                    }
                }
            }
            div { class: "fab-stack",
                button {
                    id: "fab-search",
                    class: "fab small",
                    title: "Search",
                    onclick: move |_| {
                        if search_open() {
                            // Second press closes the bar and resets it,
                            // same as the ✕ inside it.
                            search_text.set(String::new());
                            search_results.set(None);
                            search_open.set(false);
                            return;
                        }
                        search_open.set(true);
                        // Put the cursor in the field right away.
                        spawn(async move {
                            sleep_ms(50).await;
                            if let Some(input) = web_sys::window()
                                .and_then(|w| w.document())
                                .and_then(|d| d.get_element_by_id("recipe-search"))
                            {
                                let _ = input
                                    .dyn_into::<web_sys::HtmlInputElement>()
                                    .ok()
                                    .map(|i| i.focus());
                            }
                        });
                    },
                    IconSearch {}
                }
                button {
                    id: "fab-sort",
                    class: "fab small",
                    title: "Sort recipes",
                    onclick: move |_| {
                        add_menu.set(false);
                        sort_menu.set(!sort_menu());
                    },
                    IconSort {}
                }
                button {
                    id: "fab-add-recipe",
                    class: "fab",
                    title: "Add recipe",
                    onclick: move |_| {
                        sort_menu.set(false);
                        add_menu.set(!add_menu());
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

/// Import a recipe from a photo: pick an image, the backend reads it with
/// the configured vision model, and the result lands in the same preview →
/// edit → save flow as the URL import. The picked photo becomes the recipe
/// image unless the user replaces it.
#[component]
fn ImportImage() -> Element {
    let mut images = use_signal(Vec::<(String, Vec<u8>)>::new);
    let mut loading = use_signal(|| false);
    let mut error = use_signal(|| String::new());
    let mut preview = use_signal(|| None::<ImportPreview>);
    let mut show_warnings = use_signal(|| true);

    // Read the picked files into memory; the submit posts them as-is. A new
    // pick replaces the selection.
    let pick = move |e: FormEvent| {
        let files = e.files();
        spawn(async move {
            let mut picked: Vec<(String, Vec<u8>)> = Vec::new();
            for file in files {
                match file.read_bytes().await {
                    Ok(bytes) => picked.push((file.name(), bytes.to_vec())),
                    Err(err) => {
                        tracing::error!("failed to read a picked image: {err:#}");
                    }
                }
            }
            if picked.is_empty() {
                error.set("Could not read any of those files.".into());
                return;
            }
            error.set(String::new());
            images.set(picked);
        });
    };

    let import = move |_| {
        let picked = images.read().clone();
        if picked.is_empty() || *loading.read() {
            return;
        }
        loading.set(true);
        error.set(String::new());
        spawn(async move {
            // reqwest multipart is unavailable on wasm — hand-roll the body
            // (same as the recipe photo upload).
            let boundary = format!("ImportImage{}", js_sys::Date::now() as u64);
            let mut body: Vec<u8> = Vec::new();
            for (filename, bytes) in &picked {
                body.extend_from_slice(
                    format!(
                        "--{boundary}\r\nContent-Disposition: form-data; name=\"image\"; \
                         filename=\"{filename}\"\r\nContent-Type: application/octet-stream\r\n\r\n"
                    )
                    .as_bytes(),
                );
                body.extend_from_slice(bytes);
                body.extend_from_slice(b"\r\n");
            }
            body.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());

            let client = reqwest::Client::new();
            let endpoint = format!("{}/api/recipes/import-image", api_base());
            let result = client
                .post(endpoint)
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                )
                .body(body)
                .timeout(std::time::Duration::from_secs(120))
                .send()
                .await;
            match result {
                Ok(resp) if resp.status().is_success() => match resp.json::<ImportPreview>().await {
                    Ok(p) => {
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
                    let message = serde_json::from_str::<serde_json::Value>(&message)
                        .ok()
                        .and_then(|v| v["error"].as_str().map(str::to_string))
                        .unwrap_or(message);
                    tracing::error!("image import failed: {status} {message}");
                    error.set(if message.is_empty() {
                        format!("the server answered {status}")
                    } else {
                        message
                    });
                }
                Err(err) => {
                    tracing::error!("image import request failed: {err:#}");
                    error.set("Could not reach the server.".into());
                }
            }
            loading.set(false);
        });
    };

    let images_snapshot = images.read().clone();
    // rsx bodies can't hold `let` statements, so precompute the picked-file
    // note ("N photos (M bytes)").
    let picked_note: Option<String> = (!images_snapshot.is_empty()).then(|| {
        let total: usize = images_snapshot.iter().map(|(_, b)| b.len()).sum();
        format!(
            "{} photo(s), {} bytes total — pages of the same recipe combine into one",
            images_snapshot.len(),
            total
        )
    });
    let preview_snapshot = preview.read().clone();
    match preview_snapshot {
        None => rsx! {
            div { class: "page",
                div { class: "card import-card",
                    h1 { "Import from image" }
                    p { class: "muted",
                        "Pick a photo of a recipe — a cookbook page, a recipe card \
                         or a screenshot. Several photos (e.g. two pages) combine \
                         into one recipe. Bouedig reads them and lets you review \
                         the result before saving."
                    }
                    input {
                        id: "import-image",
                        r#type: "file",
                        accept: "image/*",
                        multiple: "true",
                        onchange: pick,
                    }
                    if let Some(note) = &picked_note {
                        p { class: "muted", "{note}" }
                        button {
                            id: "import-image-submit",
                            class: "btn-primary",
                            r#type: "button",
                            disabled: *loading.read(),
                            onclick: import,
                            if *loading.read() { "Reading the photo…" } else { "Import" }
                        }
                    }
                    if !error.read().is_empty() {
                        p { class: "status-error", "{error}" }
                    }
                    if *loading.read() {
                        p { class: "muted", "Reading the photo can take half a minute." }
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
                                p { "Check these before saving:" }
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
                    }
                    RecipeFormFields {
                        initial: import_preview_to_detail(&p),
                        initial_image_url: None,
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
            RecipeFormFields {
                initial: RecipeDetailModel::default(),
                initial_image_url: None,
                editing_id: None,
            }
        },
        (Some(_), Some(initial)) => rsx! {
            RecipeFormFields {
                initial: initial,
                initial_image_url: None,
                editing_id: editing,
            }
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
    let import_image_url = use_signal(|| initial_image_url);
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
    // Layered "new section" dialog for the ingredient modal.
    let mut new_section_dialog = use_signal(|| false);
    let mut new_section_name = use_signal(String::new);

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

            // Layered on top of the ingredient modal: create a section and
            // assign it without losing what is already typed.
            if new_section_dialog() {
                div {
                    // z-index above the ingredient dialog it layers over.
                    class: "dialog-backdrop section-dialog",
                    onclick: move |_| new_section_dialog.set(false),
                    div { class: "dialog", role: "dialog", onclick: move |e: MouseEvent| e.stop_propagation(),
                        h3 { "New section" }
                        input {
                            id: "modal-new-section-name",
                            r#type: "text",
                            placeholder: "Section name",
                            value: "{new_section_name}",
                            oninput: move |e: FormEvent| new_section_name.set(e.value()),
                        }
                        div { class: "dialog-actions",
                            button {
                                class: "dialog-btn",
                                onclick: move |_| {
                                    new_section_dialog.set(false);
                                    new_section_name.set(String::new());
                                },
                                "Cancel"
                            }
                            button {
                                id: "modal-new-section-ok",
                                class: "dialog-btn primary",
                                onclick: move |_| {
                                    let name = new_section_name.read().trim().to_string();
                                    if name.is_empty() {
                                        return;
                                    }
                                    let id = *next_id.read();
                                    sections.with_mut(|s| s.push(SectionRow { id, name: name.clone() }));
                                    next_id.set(id + 1);
                                    modal.with_mut(|m| {
                                        if let Some(Modal::Ingredient { section_id, .. }) = m {
                                            *section_id = Some(id);
                                        }
                                    });
                                    new_section_dialog.set(false);
                                    new_section_name.set(String::new());
                                },
                                "Create"
                            }
                        }
                    }
                }
            }

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
                                div { class: "section-options",
                                    button {
                                        class: if section_id.is_none() { "cat-btn selected" } else { "cat-btn" },
                                        r#type: "button",
                                        onclick: move |_| {
                                            modal.with_mut(|m| {
                                                if let Some(Modal::Ingredient { section_id, .. }) = m {
                                                    *section_id = None;
                                                }
                                            });
                                        },
                                        "Main"
                                    }
                                    for s in sections_snapshot.iter().cloned() {
                                        button {
                                            key: "{s.id}",
                                            class: if section_id == Some(s.id) { "cat-btn selected" } else { "cat-btn" },
                                            r#type: "button",
                                            onclick: move |_| {
                                                let id = s.id;
                                                modal.with_mut(|m| {
                                                    if let Some(Modal::Ingredient { section_id, .. }) = m {
                                                        *section_id = Some(id);
                                                    }
                                                });
                                            },
                                            "{s.name}"
                                        }
                                    }
                                    button {
                                        class: "cat-btn section-add",
                                        r#type: "button",
                                        onclick: move |_| new_section_dialog.set(true),
                                        "+ Add section"
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
                                div { class: "section-options",
                                    button {
                                        class: if section_id.is_none() { "cat-btn selected" } else { "cat-btn" },
                                        r#type: "button",
                                        onclick: move |_| {
                                            modal.with_mut(|m| {
                                                if let Some(Modal::Step { section_id, .. }) = m {
                                                    *section_id = None;
                                                }
                                            });
                                        },
                                        "Main"
                                    }
                                    for s in step_sections_snapshot.iter().cloned() {
                                        button {
                                            key: "{s.id}",
                                            class: if section_id == Some(s.id) { "cat-btn selected" } else { "cat-btn" },
                                            r#type: "button",
                                            onclick: move |_| {
                                                let id = s.id;
                                                modal.with_mut(|m| {
                                                    if let Some(Modal::Step { section_id, .. }) = m {
                                                        *section_id = Some(id);
                                                    }
                                                });
                                            },
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
    // Cooking check-off: tap an ingredient line or a step to strike it
    // through, tap again to clear. Session-only by design — it resets when
    // the page is left or reloaded.
    let mut crossed_ingredients = use_signal(|| HashSet::<usize>::new());
    let mut crossed_steps = use_signal(|| HashSet::<usize>::new());
    let mut cart_sheet = use_signal(|| false);
    let mut day_chooser = use_signal(|| false);
    let mut added_note = use_signal(|| String::new());
    let mut authed = use_signal(|| true);
    let navigator = use_navigator();

    // Anonymous visitors browse read-only: the mutating header actions only
    // render once authenticated.
    use_effect(move || {
        spawn(async move {
            authed.set(session_authenticated().await);
        });
    });

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
    // group per section (in order). Each line carries a stable index for
    // the tap-to-cross check-off.
    let detail_groups: Vec<(Option<String>, Vec<(usize, Ingredient)>)> = loaded
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
            let mut index = 0usize;
            groups
                .into_iter()
                .map(|(section, group)| {
                    let numbered = group
                        .into_iter()
                        .map(|ingredient| {
                            index += 1;
                            (index, ingredient)
                        })
                        .collect::<Vec<_>>();
                    (section, numbered)
                })
                .collect()
        })
        .unwrap_or_default();

    let has_ingredients = loaded
        .as_ref()
        .is_some_and(|d| !d.ingredients.is_empty());

    // Shopping-list sheet lines: base quantities without prep text (scale
    // ignored on purpose) — the sheet shows exactly what will be added.
    let cart_lines: Vec<String> = loaded
        .as_ref()
        .map(|d| d.ingredients.iter().map(grocery_line).collect())
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
                if authed() {
                    button {
                        id: "hdr-share",
                        class: "hdr-btn",
                        title: "Share link",
                        onclick: move |_| {
                            let url = format!("{}/recipe/{id}", api_base());
                            spawn(async move {
                                if copy_clipboard(url.clone()).await {
                                    added_note.set("Link copied to clipboard".into());
                                } else {
                                    added_note.set(url);
                                }
                            });
                        },
                        IconShare {}
                    }
                    button {
                        id: "hdr-mealplan",
                        class: "hdr-btn",
                        title: "Add to meal plan",
                        onclick: move |_| {
                            added_note.set(String::new());
                            day_chooser.set(true);
                        },
                        IconCalendar {}
                    }
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
                                inputmode: "decimal",
                                autocomplete: "off",
                                value: "{scale_value}",
                                onfocus: move |_| {
                                    // Select the current value: click + type a
                                    // number replaces it, no manual deleting.
                                    // Select synchronously (real clicks) AND
                                    // deferred (programmatic focus restores the
                                    // caret after the handler; automated input
                                    // types before the deferred pass runs).
                                    if let Some(input) = web_sys::window()
                                        .and_then(|w| w.document())
                                        .and_then(|d| d.get_element_by_id("scale-input"))
                                        .and_then(|el| {
                                            el.dyn_into::<web_sys::HtmlInputElement>().ok()
                                        })
                                    {
                                        input.select();
                                    }
                                    spawn(async move {
                                        sleep_ms(20).await;
                                        if let Some(input) = web_sys::window()
                                            .and_then(|w| w.document())
                                            .and_then(|d| d.get_element_by_id("scale-input"))
                                            .and_then(|el| {
                                                el.dyn_into::<web_sys::HtmlInputElement>()
                                                    .ok()
                                            })
                                        {
                                            input.select();
                                        }
                                    });
                                },
                                oninput: move |e: FormEvent| {
                                    // Numbers only: keep digits and the decimal
                                    // point, drop everything else as typed.
                                    let cleaned: String = e
                                        .value()
                                        .chars()
                                        .filter(|c| c.is_ascii_digit() || *c == '.')
                                        .collect();
                                    scale_text.set(cleaned);
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
                                        for &(index, ref ingredient) in group {
                                            li {
                                                class: if crossed_ingredients.read().contains(&index) {
                                                    "ingredient-item crossed"
                                                } else {
                                                    "ingredient-item"
                                                },
                                                onclick: move |_| {
                                                    crossed_ingredients.with_mut(|s| {
                                                        if !s.remove(&index) {
                                                            s.insert(index);
                                                        }
                                                    });
                                                },
                                                "{ingredient_line(ingredient, effective_scale)}"
                                            }
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
                                        for &(number, ref step) in group {
                                            li {
                                                key: "{number}",
                                                class: if crossed_steps.read().contains(&number) {
                                                    "instruction-item crossed"
                                                } else {
                                                    "instruction-item"
                                                },
                                                onclick: move |_| {
                                                    crossed_steps.with_mut(|s| {
                                                        if !s.remove(&number) {
                                                            s.insert(number);
                                                        }
                                                    });
                                                },
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
                            a { class: "source-link", href: "{d.source}", target: "_blank", rel: "noopener", "{d.source}" }
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
                                .map(|name| NewGroceryItem { name, category: None, quantity: None, unit: None })
                                .collect(),
                            recipe_id: Some(id),
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

            if day_chooser() {
                DayChooser {
                    recipe_name: loaded
                        .as_ref()
                        .map(|d| d.name.clone())
                        .unwrap_or_default(),
                    on_pick: move |(date, label): (String, String)| {
                        day_chooser.set(false);
                        let recipe_id = id;
                        spawn(async move {
                            let client = reqwest::Client::new();
                            let url = format!("{}/api/meal-plan", api_base());
                            let payload = serde_json::json!({
                                "date": date,
                                "recipe_id": recipe_id,
                            });
                            match client.post(&url).json(&payload).send().await {
                                Ok(r) if r.status().is_success() => {
                                    added_note.set(format!("Added to {label}"));
                                }
                                Ok(r) => {
                                    tracing::error!("meal-plan add failed: {}", r.status());
                                    error.set(format!("Could not add to meal plan: {}", r.status()));
                                }
                                Err(err) => {
                                    tracing::error!("meal-plan add request failed: {err:#}");
                                    error.set(format!("Could not add to meal plan: {err}"));
                                }
                            }
                        });
                    },
                    on_cancel: move |_| day_chooser.set(false),
                }
            }
        }
    }
}

/// Bottom sheet listing the next two weeks, blaz style: every day shows the
/// recipes already planned on it (thumbnails), or "Nothing planned". Picking
/// a day schedules the recipe for it.
#[component]
fn DayChooser(
    recipe_name: String,
    on_pick: EventHandler<(String, String)>,
    on_cancel: EventHandler<()>,
) -> Element {
    let mut entries = use_signal(Vec::<MealPlanEntry>::new);
    let mut loaded = use_signal(|| false);
    use_effect(move || {
        if !loaded() {
            loaded.set(true);
            spawn(async move {
                if let Ok(list) = api_get::<Vec<MealPlanEntry>>("/api/meal-plan").await {
                    entries.set(list);
                }
            });
        }
    });

    let today = today_iso();
    let entries_snapshot = entries.read().clone();
    let days: Vec<(String, String, Vec<MealPlanEntry>)> = (0..14)
        .map(|offset| {
            let date = shift_iso(&today, offset);
            let label = day_label(&date, &today);
            let planned: Vec<MealPlanEntry> = entries_snapshot
                .iter()
                .filter(|e| e.date == date)
                .cloned()
                .collect();
            (date, label, planned)
        })
        .collect();

    rsx! {
        div { class: "sheet-backdrop",
            onclick: move |_| on_cancel.call(()),
            div { class: "sheet", role: "dialog",
                onclick: move |e: MouseEvent| e.stop_propagation(),
                h2 { class: "sheet-title", "Assign \u{201c}{recipe_name}\u{201d} to…" }
                div { class: "sheet-list day-list",
                    for (date, label, planned) in days {
                        button {
                            class: "day-btn",
                            onclick: move |_| {
                                on_pick.call((date.clone(), label.clone()));
                            },
                            div { class: "day-main",
                                span { class: "day-label", "{label}" }
                                if planned.is_empty() {
                                    span { class: "day-none", "Nothing planned" }
                                } else {
                                    div { class: "day-thumbs",
                                        for entry in planned.iter() {
                                            div { class: "day-thumb",
                                                if let Some(thumb) = &entry.recipe.thumb {
                                                    img { src: "{thumb}", alt: "{entry.recipe.name}" }
                                                } else {
                                                    div { class: "day-thumb-placeholder",
                                                        {entry.recipe.name.chars().next().unwrap_or('?').to_string()}
                                                    }
                                                }
                                                span { class: "day-thumb-name", "{entry.recipe.name}" }
                                            }
                                        }
                                    }
                                }
                            }
                            span { class: "day-chevron", "›" }
                        }
                    }
                }
                div { class: "sheet-actions day-actions",
                    button {
                        id: "day-cancel",
                        class: "dialog-btn day-cancel",
                        onclick: move |_| on_cancel.call(()),
                        "Cancel"
                    }
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

/// Shopping-list line for an ingredient: quantity + unit + name only —
/// the prep detail is recipe-cooking information, not shopping information.
fn grocery_line(ingredient: &Ingredient) -> String {
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
    rsx! {
        LoginGate {
            GroceryContent {}
        }
    }
}

#[component]
fn GroceryContent() -> Element {
    let mut items = use_signal(Vec::<GroceryItem>::new);
    let mut new_item = use_signal(String::new);
    let mut new_qty = use_signal(String::new);
    let mut new_unit = use_signal(String::new);
    // Every name ever seen on this list (server: /api/grocery/names) so the
    // add-row suggestions also cover past recipe additions, which are gone
    // from the live list once ticked off.
    let name_history = use_signal(Vec::<String>::new);
    let mut error = use_signal(|| String::new());
    let mut focused = use_signal(|| false);
    let collapsed = use_signal(|| HashSet::<String>::new());
    let mut loaded = use_signal(|| false);
    let mut plan_loaded = use_signal(|| false);
    let mut plan_entries = use_signal(Vec::<MealPlanEntry>::new);
    let mut edit_item = use_signal(|| None::<GroceryItem>);
    // The add-item bottom sheet (opened by the [+] FAB) replaces the old
    // always-visible add row.
    let mut add_sheet = use_signal(|| false);
    // The last item ticked off, kept around briefly so a mis-tick can be
    // undone; `undo_gen` invalidates the timer when a newer removal lands.
    let undo_item = use_signal(|| None::<GroceryItem>);
    let undo_gen = use_signal(|| 0u64);

    use_effect(move || {
        if !loaded() {
            loaded.set(true);
            spawn(async move {
                refresh(items, error).await;
                refresh_names(name_history).await;
            });
        }
    });

    // Focus the name field when the add sheet opens, so typing starts
    // immediately (a beat after mount — the input must exist first).
    use_effect(move || {
        if add_sheet() {
            spawn(async move {
                sleep_ms(50).await;
                if let Some(input) = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.get_element_by_id("grocery-input"))
                {
                    let _ = input
                        .dyn_into::<web_sys::HtmlInputElement>()
                        .ok()
                        .map(|i| i.focus());
                }
            });
        }
    });

    // Background classification flips categories a few seconds after an
    // add, so re-fetch periodically while the page is open. The loop dies
    // with the page via the mounted flag (use_drop).
    let mut mounted = use_signal(|| true);
    use_drop(move || mounted.set(false));
    use_effect(move || {
        spawn(async move {
            loop {
                sleep_ms(5000).await;
                if !mounted() {
                    break;
                }
                refresh(items, error).await;
                refresh_names(name_history).await;
            }
        });
    });

    // Plan dates feed the provenance line ("Lentil Loaf in 4 days"): one
    // fetch, mapped client-side so the day math uses the browser's today.
    use_effect(move || {
        if !plan_loaded() {
            plan_loaded.set(true);
            spawn(async move {
                match api_get::<Vec<MealPlanEntry>>("/api/meal-plan").await {
                    Ok(list) => plan_entries.set(list),
                    Err(err) => tracing::error!("meal-plan fetch for provenance failed: {err:#}"),
                }
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
    // Suggestion pool: the live list first (its casing wins), then the
    // ever-added history. History names come back lowercased from the
    // classifier cache, so give cache-only names a display capital.
    let mut past_names: Vec<String> = Vec::new();
    for item in &list {
        if !past_names.iter().any(|n| n.eq_ignore_ascii_case(&item.name)) {
            past_names.push(item.name.clone());
        }
    }
    for name in name_history.read().iter() {
        if !past_names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            let mut display = name.clone();
            if let Some(first) = display.get_mut(0..1) {
                first.make_ascii_uppercase();
            }
            past_names.push(display);
        }
    }

    let suggestions = rank_suggestions(&new_item.read(), &past_names);

    let today_value = today_iso();
    let plan_list = plan_entries.read().clone();
    let plan_map = planned_dates(&plan_list, &today_value);
    // Category dropdown options: the preset list first, then any group the
    // user has invented that isn't already covered.
    let mut category_options: Vec<String> = shared::GROCERY_CATEGORIES
        .iter()
        .map(|c| c.to_string())
        .collect();
    for (category, _) in &groups {
        if !category_options.contains(category) {
            category_options.push(category.clone());
        }
    }

    let edited_item = edit_item.read().clone();
    let edited_id = edited_item.as_ref().map(|item| item.id);
    let edit_source = edited_item
        .as_ref()
        .and_then(|item| item.recipe.as_ref())
        .map(|recipe| match plan_map.get(&recipe.id) {
            Some(date) => match plan_relative_label(date, &today_value) {
                Some(rel) => format!("{} {rel}", recipe.name),
                None => recipe.name.clone(),
            },
            None => recipe.name.clone(),
        })
        .unwrap_or_default();

    // Bind before rsx: the render reads the undo slot once per repaint.
    let undo_snapshot = undo_item.read().clone();

    rsx! {
        div { class: "page",
            if !error.read().is_empty() {
                p { class: "status-error", "{error}" }
            }
            if let Some(removed) = undo_snapshot {
                div { class: "undo-bar",
                    span { class: "undo-text", "Removed \"{removed.name}\"" }
                    button {
                        id: "undo-restore",
                        r#type: "button",
                        onclick: move |_| {
                            restore_removed_item(removed.clone(), items, undo_item, error);
                        },
                        "Undo"
                    }
                }
            }
            if !error.read().is_empty() {
                p { class: "status-error", "{error}" }
            }
            if groups.is_empty() {
                p { class: "empty", "Your grocery list is empty. Tap + to add an item." }
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
                            on_edit: move |item: GroceryItem| edit_item.set(Some(item)),
                            on_bought: move |item: GroceryItem| {
                                record_removed_item(item, undo_item, undo_gen, mounted);
                            },
                        }
                    }
                }
            }

            if let Some(item) = edited_item {
                GroceryEditSheet {
                    key: "edit-{item.id}",
                    item,
                    source: edit_source,
                    groups: category_options.clone(),
                    on_save: move |(name, qty, unit_text, category): (String, Option<f64>, String, String)| {
                        edit_item.set(None);
                        let id = edited_id.unwrap_or_default();
                        let payload = GroceryPatch {
                            name,
                            quantity: qty,
                            category: Some(category),
                            unit: Some(unit_text),
                        };
                        spawn(async move {
                            let client = reqwest::Client::new();
                            let url = format!("{}/api/grocery/{id}", api_base());
                            match client.put(&url).json(&payload).send().await {
                                Ok(r) if r.status().is_success() => {
                                    match r.json::<GroceryItem>().await {
                                        Ok(updated) => {
                                            items.with_mut(|v| {
                                                if let Some(slot) =
                                                    v.iter_mut().find(|i| i.id == id)
                                                {
                                                    *slot = updated;
                                                }
                                            });
                                        }
                                        Err(err) => {
                                            tracing::error!(
                                                "PUT /api/grocery unreadable body: {err:#}"
                                            );
                                            refresh(items, error).await;
                                        }
                                    }
                                }
                                Ok(r) => {
                                    tracing::error!("PUT /api/grocery failed: {}", r.status());
                                    error.set("Failed to update item.".into());
                                }
                                Err(err) => {
                                    tracing::error!("PUT /api/grocery request failed: {err:#}");
                                    error.set("Failed to update item.".into());
                                }
                            }
                        });
                    },
                    on_cancel: move |_| edit_item.set(None),
                }
            }
            if add_sheet() {
                div { class: "dialog-backdrop",
                    onclick: move |_| add_sheet.set(false),
                    div { class: "dialog add-dialog", role: "dialog",
                        onclick: move |e: MouseEvent| e.stop_propagation(),
                        h2 { class: "dialog-title", "Add item" }
                        div { class: "input-wrap",
                            div { class: "qty-unit-row",
                                input {
                                    id: "grocery-qty",
                                    class: "qty-input",
                                    r#type: "text",
                                    inputmode: "decimal",
                                    value: "{new_qty}",
                                    placeholder: "2",
                                    oninput: move |e: FormEvent| {
                                        // Digits and the decimal point only.
                                        let cleaned: String = e
                                            .value()
                                            .chars()
                                            .filter(|c| c.is_ascii_digit() || *c == '.')
                                            .collect();
                                        new_qty.set(cleaned);
                                    },
                                    autocomplete: "off",
                                }
                                input {
                                    id: "grocery-unit",
                                    class: "unit-input",
                                    r#type: "text",
                                    value: "{new_unit}",
                                    placeholder: "g, ml, packs…",
                                    oninput: move |e: FormEvent| new_unit.set(e.value()),
                                    autocomplete: "off",
                                }
                            }
                            input {
                                id: "grocery-input",
                                r#type: "text",
                                value: "{new_item}",
                                placeholder: "Add an item manually…",
                                oninput: move |e: FormEvent| new_item.set(e.value()),
                                onfocus: move |_| focused.set(true),
                                onblur: move |_| focused.set(false),
                                onkeydown: move |e: KeyboardEvent| {
                                    if e.key() == Key::Enter {
                                        // Bind first: the read guard must be
                                        // dropped before submit calls
                                        // `new_item.set()`.
                                        let name = new_item.read().clone();
                                        submit_grocery_item(
                                            name,
                                            parse_qty(&new_qty.read()),
                                            new_unit.read().trim().to_string(),
                                            new_item,
                                            items,
                                            name_history,
                                            error,
                                        );
                                    }
                                },
                                autocomplete: "off",
                            }
                            if focused()
                                && !new_item.read().trim().is_empty()
                                && !suggestions.is_empty()
                            {
                                div { class: "suggestions",
                                    for (name, _) in suggestions {
                                        button {
                                            class: "suggestion",
                                            r#type: "button",
                                            // mousedown, not click: the input's
                                            // blur (fired on mousedown) unmounts
                                            // this dropdown before a click could
                                            // land. Tapping a suggestion adds it
                                            // outright.
                                            onmousedown: move |_| {
                                                submit_grocery_item(
                                                    name.clone(),
                                                    parse_qty(&new_qty.read()),
                                                    new_unit.read().trim().to_string(),
                                                    new_item,
                                                    items,
                                                    name_history,
                                                    error,
                                                );
                                            },
                                            span { class: "suggestion-text", "{name}" }
                                        }
                                    }
                                }
                            }
                        }
                        div { class: "dialog-actions",
                            button {
                                id: "grocery-add",
                                class: "dialog-btn primary",
                                r#type: "button",
                                onclick: move |_| {
                                    let name = new_item.read().clone();
                                    submit_grocery_item(
                                        name,
                                        parse_qty(&new_qty.read()),
                                        new_unit.read().trim().to_string(),
                                        new_item,
                                        items,
                                        name_history,
                                        error,
                                    )
                                },
                                "Add"
                            }
                            button {
                                id: "add-cancel",
                                class: "dialog-btn",
                                r#type: "button",
                                onclick: move |_| add_sheet.set(false),
                                "Cancel"
                            }
                        }
                    }
                }
            }
            div { class: "fab-stack",
                button {
                    id: "grocery-fab-add",
                    class: "fab",
                    title: "Add item",
                    onclick: move |_| add_sheet.set(true),
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
    on_edit: EventHandler<GroceryItem>,
    on_bought: EventHandler<GroceryItem>,
) -> Element {
    let is_collapsed = collapsed.read().contains(&name);

    rsx! {
        div { class: "grocery-card",
            button {
                // No variable binding here: dioxus strips `key:` from plain
                // elements in release builds, which would strand it unused.
                key: "group-{name}",
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
                        GroceryRow {
                            key: "{item.id}",
                            item: item,
                            items_sig: items_sig,
                            error: error,
                            on_edit: on_edit.clone(),
                            on_bought: on_bought.clone(),
                        }
                    }
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
    on_edit: EventHandler<GroceryItem>,
    on_bought: EventHandler<GroceryItem>,
) -> Element {
    let item_id = item.id;
    let item_for_bought = item.clone();
    rsx! {
        li {
            id: "grocery-item-{item.id}",
            class: "grocery-item",
            onclick: move |_| on_edit.call(item.clone()),
            // Checking the box marks the item bought — today's toggle
            // semantics remove it from the list.
            input {
                r#type: "checkbox",
                class: "grocery-check",
                title: "Mark bought",
                onclick: move |e: MouseEvent| e.stop_propagation(),
                onchange: move |_| {
                    let id = item_id;
                    let bought_item = item_for_bought.clone();
                    spawn(async move {
                        let client = reqwest::Client::new();
                        let url = format!("{}/api/grocery/{id}", api_base());
                        match client
                            .patch(&url)
                            .json(&GroceryUpdate { bought: true })
                            .send()
                            .await
                        {
                            Ok(r) if r.status().is_success() => {
                                items_sig.with_mut(|v| v.retain(|i| i.id != id));
                                on_bought.call(bought_item);
                            }
                            Ok(r) => {
                                tracing::error!("PATCH /api/grocery/{id} failed: {}", r.status());
                                error.set("Failed to mark item bought.".into());
                            }
                            Err(err) => {
                                tracing::error!("PATCH /api/grocery/{id} request failed: {err:#}");
                                error.set("Failed to mark item bought.".into());
                            }
                        }
                    });
                },
            }
            // Amount prefix (as typed), only when the item has one.
            if let Some(prefix) = grocery_qty_prefix(item.quantity, &item.unit) {
                span { class: "grocery-qty", "{prefix}" }
            }
            span { "{item.name}" }
        }
    }
}

/// "500 g" / "2×"-style row prefix from the stored amount; None when the
/// item carries no quantity.
fn grocery_qty_prefix(quantity: Option<f64>, unit: &str) -> Option<String> {
    let qty = quantity?;
    let unit = unit.trim();
    // A trailing space keeps the DOM text "500 g oats" whole for
    // tests and screen readers despite the separate spans.
    Some(format!("{} {unit} ", fmt_qty(qty)))
}
#[component]
fn GroceryEditSheet(
    item: GroceryItem,
    source: String,
    groups: Vec<String>,
    on_save: EventHandler<(String, Option<f64>, String, String)>,
    on_cancel: EventHandler<()>,
) -> Element {
    let mut name = use_signal(|| item.name.clone());
    let mut group = use_signal(|| item.category.clone());
    let mut quantity = use_signal(|| match item.quantity {
        Some(q) => fmt_qty(q),
        None => String::new(),
    });
    let mut unit = use_signal(|| item.unit.clone());
    let name_value = name.read().trim().to_string();
    // Reading the live selection here subscribes the render to `group`, so
    // the highlight follows taps instead of staying on the loaded category.
    let group_value = group.read().clone();
    // The item's own group always has a button, even if it somehow fell out
    // of the merged category list. Precomputed so rsx can map plainly.
    let mut options = groups;
    if !options.iter().any(|g| g == &item.category) {
        options.insert(0, item.category.clone());
    }
    let categories: Vec<(String, &'static str, String)> = options
        .iter()
        .map(|g| {
            (
                g.clone(),
                shared::category_emoji(g),
                format!("cat-{}", g.replace(' ', "-")),
            )
        })
        .collect();

    rsx! {
        div { class: "sheet-backdrop",
            onclick: move |_| on_cancel.call(()),
            div { class: "sheet", role: "dialog",
                onclick: move |e: MouseEvent| e.stop_propagation(),
                h2 { class: "sheet-title", "Edit item" }
                if !source.is_empty() {
                    p { class: "sheet-subtitle", id: "sheet-item-source", "{source}" }
                }
                div { class: "sheet-form",
                    label { class: "sheet-field",
                        span { "Name" }
                        input {
                            id: "sheet-item-name",
                            r#type: "text",
                            value: "{name}",
                            oninput: move |e: FormEvent| name.set(e.value()),
                        }
                    }
                    div { class: "sheet-field",
                        span { "Amount" }
                        div { class: "qty-unit-row",
                            input {
                                id: "sheet-item-qty",
                                class: "qty-input",
                                r#type: "text",
                                inputmode: "decimal",
                                value: "{quantity}",
                                placeholder: "2",
                                oninput: move |e: FormEvent| {
                                    let cleaned: String = e
                                        .value()
                                        .chars()
                                        .filter(|c| c.is_ascii_digit() || *c == '.')
                                        .collect();
                                    quantity.set(cleaned);
                                },
                            }
                            input {
                                id: "sheet-item-unit",
                                class: "unit-input",
                                r#type: "text",
                                value: "{unit}",
                                placeholder: "g, ml, packs…",
                                oninput: move |e: FormEvent| unit.set(e.value()),
                            }
                        }
                    }
                    div { class: "sheet-field",
                        span { "Group" }
                        div { class: "sheet-cats",
                            for (label, emoji, button_id) in categories.iter() {
                                button {
                                    id: "{button_id}",
                                    class: if *label == group_value { "cat-btn selected" } else { "cat-btn" },
                                    r#type: "button",
                                    onclick: {
                                        let label = label.clone();
                                        move |_| group.set(label.clone())
                                    },
                                    span { class: "cat-emoji", "{emoji}" }
                                    span { class: "cat-name", "{label}" }
                                }
                            }
                        }
                    }
                }
                div { class: "sheet-actions",
                    button {
                        id: "sheet-item-cancel",
                        class: "dialog-btn",
                        onclick: move |_| on_cancel.call(()),
                        "Cancel"
                    }
                    button {
                        id: "sheet-item-save",
                        class: "dialog-btn primary",
                        disabled: name_value.is_empty(),
                        onclick: move |_| {
                            on_save.call((name.read().trim().to_string(), parse_qty(&quantity.read().trim()), unit.read().trim().to_string(), group.read().trim().to_string()));
                        },
                        "Save"
                    }
                }
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

/// Re-fetch the ever-added name history for the add-row suggestions
/// (live items plus everything the classifier has ever seen).
async fn refresh_names(mut history: Signal<Vec<String>>) {
    match api_get::<Vec<String>>("/api/grocery/names").await {
        Ok(names) => history.set(names),
        Err(err) => tracing::error!("grocery name history refresh failed: {err:#}"),
    }
}

/// Park the just-removed item in the undo slot and start its expiry timer.
/// A newer removal bumps the generation, invalidating the older timer.
fn record_removed_item(
    item: GroceryItem,
    mut undo_item: Signal<Option<GroceryItem>>,
    mut undo_gen: Signal<u64>,
    mounted: Signal<bool>,
) {
    undo_item.set(Some(item));
    undo_gen.with_mut(|g| *g += 1);
    let gen = undo_gen.read().clone();
    spawn(async move {
        sleep_ms(6000).await;
        if mounted() && undo_gen.read().clone() == gen {
            undo_item.set(None);
        }
    });
}

/// Undo a ticked-off item: re-add it through the batch endpoint so the
/// explicit category (which skips JEV) and the recipe provenance come back.
fn restore_removed_item(
    item: GroceryItem,
    mut items_sig: Signal<Vec<GroceryItem>>,
    mut undo_item: Signal<Option<GroceryItem>>,
    mut error: Signal<String>,
) {
    undo_item.set(None);
    spawn(async move {
        let payload = NewGroceryBatch {
            items: vec![NewGroceryItem {
                name: item.name.clone(),
                category: Some(item.category.clone()),
                quantity: item.quantity,
                unit: Some(item.unit.clone()),
            }],
            recipe_id: item.recipe.as_ref().map(|r| r.id),
        };
        let client = reqwest::Client::new();
        let url = format!("{}/api/grocery/batch", api_base());
        tracing::info!("Grocery undo: POST {url} (name={:?})", payload.items[0].name);
        match client.post(url).json(&payload).send().await {
            Ok(r) if r.status().is_success() => match r.json::<Vec<GroceryItem>>().await {
                Ok(created) => items_sig.with_mut(|v| v.extend(created)),
                Err(err) => {
                    tracing::error!("grocery restore returned an unreadable body: {err:#}");
                    error.set("Failed to restore item.".into());
                }
            },
            Ok(r) => {
                tracing::error!("grocery restore failed: {}", r.status());
                error.set("Failed to restore item.".into());
            }
            Err(err) => {
                tracing::error!("grocery restore request failed: {err:#}");
                error.set("Failed to restore item.".into());
            }
        }
    });
}

/// Add a manually typed item (Add button, Enter key, or a tapped
/// suggestion). All signals are Copy, so every handler can call this
/// freely; `new_item` is cleared so the input is ready for the next item.
/// The category is always None: JEV classifies new items, and manual
/// category changes happen in the edit sheet, which pins them.
/// Parse the amount field: blank → none, otherwise a plain number.
fn parse_qty(text: &str) -> Option<f64> {
    text.trim().parse::<f64>().ok().filter(|q| q.is_finite() && *q > 0.0)
}

fn submit_grocery_item(
    name: String,
    quantity: Option<f64>,
    unit: String,
    mut new_item: Signal<String>,
    mut items: Signal<Vec<GroceryItem>>,
    name_history: Signal<Vec<String>>,
    mut error: Signal<String>,
) {
    let name = name.trim().to_string();
    if name.is_empty() {
        return;
    }
    new_item.set(String::new());
    spawn(async move {
        let item = NewGroceryItem {
            name,
            category: None,
            quantity,
            unit: Some(unit),
        };
        let client = reqwest::Client::new();
        let url = format!("{}/api/grocery", api_base());
        tracing::info!("Grocery add: POST {url} (name={:?})", item.name);
        match client.post(url).json(&item).send().await {
            Ok(r) if r.status().is_success() => match r.json::<GroceryItem>().await {
                // Append the created item to the local state instead of
                // re-fetching: the list keeps whatever the user just removed.
                Ok(created) => {
                    items.with_mut(|v| v.push(created));
                    refresh_names(name_history).await;
                }
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

/// Days from `today` to `date` (positive = future). Both are `YYYY-MM-DD`
/// strings that parse as UTC midnight, so the difference is whole days.
fn days_from_today(date: &str, today: &str) -> i64 {
    ((js_sys::Date::parse(date) - js_sys::Date::parse(today)) / 86_400_000.0) as i64
}

/// Relative label for a plan date: today / tomorrow / in N days / N days ago.
fn plan_relative_label(date: &str, today: &str) -> Option<String> {
    if date.is_empty() {
        return None;
    }
    Some(match days_from_today(date, today) {
        0 => "today".to_string(),
        1 => "tomorrow".to_string(),
        n if n > 1 => format!("in {n} days"),
        -1 => "1 day ago".to_string(),
        n => format!("{} days ago", -n),
    })
}

/// One plan date per recipe id: the earliest upcoming date wins; if a recipe
/// is only planned in the past, its most recent past date is kept.
fn planned_dates(entries: &[MealPlanEntry], today: &str) -> HashMap<i64, String> {
    let mut map: HashMap<i64, String> = HashMap::new();
    for entry in entries {
        let candidate = entry.date.as_str();
        match map.get(&entry.recipe.id) {
            Some(current) => {
                let current = current.as_str();
                let keep_candidate = match (candidate >= today, current >= today) {
                    (true, true) => candidate < current,
                    (true, false) => true,
                    (false, true) => false,
                    (false, false) => candidate > current,
                };
                if keep_candidate {
                    map.insert(entry.recipe.id, candidate.to_string());
                }
            }
            None => {
                map.insert(entry.recipe.id, candidate.to_string());
            }
        }
    }
    map
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

/// Offset in whole Sat→Fri weeks relative to the week containing today.
/// Friday evening (from 17:00) is planning time — the household plans the
/// upcoming Saturday-to-Friday week then — so the tab opens on next week
/// from that moment; otherwise on the current week.
fn initial_week_offset() -> i64 {
    let now = js_sys::Date::new_0();
    if now.get_day() == 5 && now.get_hours() >= 17 {
        1
    } else {
        0
    }
}

/// The Saturday starting the Sat→Fri week `offset` weeks from the week
/// containing `today`. Same pure UTC-millisecond arithmetic as `shift_iso`:
/// date-only strings parse as UTC midnight, `getUTCDay()` is 0=Sun..6=Sat.
fn week_start(today: &str, offset: i64) -> String {
    let millis = js_sys::Date::parse(today);
    if millis.is_nan() {
        return today.to_string();
    }
    let d = js_sys::Date::new(&millis.into());
    let days_since_saturday = ((d.get_utc_day() + 1) % 7) as i64;
    shift_iso(today, 7 * offset - days_since_saturday)
}

/// "Sat 10 Oct" short label (en-GB, same locale style as `day_label`).
fn short_date(date: &str) -> String {
    let millis = js_sys::Date::parse(date);
    if millis.is_nan() {
        return date.to_string();
    }
    let d = js_sys::Date::new(&millis.into());
    let weekday = d.to_locale_date_string(
        "en-GB",
        &js_sys::Object::from(js_sys::JSON::parse("{\"weekday\":\"short\"}").unwrap()),
    );
    let rest = d.to_locale_date_string(
        "en-GB",
        &js_sys::Object::from(
            js_sys::JSON::parse("{\"day\":\"numeric\",\"month\":\"short\"}").unwrap(),
        ),
    );
    format!("{weekday} {rest}")
}

/// Header title for the open week: This/Next/Last week, else the date range.
fn week_title(offset: i64, start: &str) -> String {
    match offset {
        0 => "This week".into(),
        1 => "Next week".into(),
        -1 => "Last week".into(),
        _ => format!("{} – {}", short_date(start), short_date(&shift_iso(start, 6))),
    }
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
    rsx! {
        LoginGate {
            MealPlanContent {}
        }
    }
}

/// Day-to-day drag of a planned recipe card on the meal plan. `armed` flips
/// on once the pointer passes a small threshold, so a plain tap still opens
/// the recipe.
#[derive(Clone)]
struct PlanDrag {
    entry_id: i64,
    origin_date: String,
    recipe_id: i64,
    start_y: f64,
    last_y: f64,
    armed: bool,
}

/// The meal a plain tap on a card's ≡ handle offered to move: the bottom
/// sheet lists the open week's days to drop it on.
#[derive(Clone, PartialEq)]
struct MoveTarget {
    entry_id: i64,
    recipe_id: i64,
    name: String,
    origin_date: String,
}

/// A plain tap on a card's ≡ handle (pointerup without a drag): offer to
/// move the meal to another day of the open week via the bottom sheet.
/// Binds before reading so the signal guard can't outlive the lookup.
fn open_move_sheet(
    drag: &PlanDrag,
    entries: Signal<Vec<MealPlanEntry>>,
    mut move_sheet: Signal<Option<MoveTarget>>,
    mut suppress_open: Signal<bool>,
) {
    let found = entries
        .read()
        .iter()
        .find(|e| e.id == drag.entry_id)
        .map(|e| (e.recipe.id, e.recipe.name.clone(), e.date.clone()));
    suppress_open.set(true);
    if let Some((recipe_id, name, origin_date)) = found {
        move_sheet.set(Some(MoveTarget {
            entry_id: drag.entry_id,
            recipe_id,
            name,
            origin_date,
        }));
    }
}

#[component]
fn MealPlanContent() -> Element {
    let mut entries = use_signal(Vec::<MealPlanEntry>::new);
    let mut recipes = use_signal(Vec::<Recipe>::new);
    let mut error = use_signal(|| String::new());
    let mut loaded = use_signal(|| false);
    let mut picker = use_signal(|| None::<PickerState>);
    let navigator = use_navigator();
    let today = use_signal(today_iso);
    // In-progress day-to-day card drag + the date currently under the
    // pointer; `suppress_open` swallows the click that follows a real drag.
    let mut plan_drag = use_signal(|| None::<PlanDrag>);
    let mut drag_target = use_signal(|| None::<String>);
    let mut suppress_open = use_signal(|| false);

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

    // The open Sat→Fri week: 0 = the week containing today. Friday evening
    // opens on next week (see `initial_week_offset`).
    let mut week_offset = use_signal(initial_week_offset);
    // A plain tap on a card's ≡ handle opens the "Move to day…" sheet.
    let mut move_sheet = use_signal(|| None::<MoveTarget>);

    // The seven days of the open week: entries grouped per day, empty days
    // render with their ⊕ so any day is plannable. All entries are already
    // in memory, so flipping weeks is instant.
    let today_value = today.read().clone();
    let week_offset_value = week_offset.read().clone();
    let week_start_date = week_start(&today_value, week_offset_value);
    let week_heading = week_title(week_offset_value, &week_start_date);
    let days: Vec<PlanDay> = (0..7)
        .map(|i| {
            let date = shift_iso(&week_start_date, i);
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

    // Bind the drag snapshots before rsx: one read per repaint.
    let drag_snapshot = plan_drag.read().clone();
    let drag_target_snapshot = drag_target.read().clone();
    let move_sheet_snapshot = move_sheet.read().clone();
    // The drag hit-test needs no captured day list: it re-reads the rendered
    // `.plan-day` ids from the DOM (closures inside the `for day in days`
    // rsx loop are `FnMut` and can never move-capture a `Vec`).

    rsx! {
        div { class: "page",
            if !error.read().is_empty() {
                p { class: "status-error", "{error}" }
            }
            // Week pager head: previous / title / (jump to today) / next.
            div { class: "plan-week-head",
                button {
                    id: "plan-week-prev",
                    class: "plan-week-btn",
                    r#type: "button",
                    title: "Previous week",
                    onclick: move |_| week_offset.with_mut(|v| *v -= 1),
                    IconBack {}
                }
                div { class: "plan-week-mid",
                    span { class: "plan-week-title", "{week_heading}" }
                    if week_offset_value != 0 {
                        button {
                            id: "plan-week-today",
                            class: "plan-week-today",
                            r#type: "button",
                            onclick: move |_| week_offset.set(0),
                            "Today"
                        }
                    }
                }
                button {
                    id: "plan-week-next",
                    class: "plan-week-btn icon-flip",
                    r#type: "button",
                    title: "Next week",
                    onclick: move |_| week_offset.with_mut(|v| *v += 1),
                    IconBack {}
                }
            }
            div {
                class: "plan-days",
                onpointermove: move |ev: PointerEvent| {
                    let Some(d) = plan_drag.read().clone() else { return };
                    if !d.armed {
                        if (ev.client_coordinates().y - d.start_y).abs() < 8.0 {
                            return;
                        }
                        plan_drag.with_mut(|s| {
                            if let Some(s) = s {
                                s.armed = true;
                                s.last_y = ev.client_coordinates().y;
                            }
                        });
                    }
                    // Which rendered day section is under the pointer?
                    let y = ev.client_coordinates().y;
                    let target = hit_test_plan_day(y);
                    drag_target.set(target);
                },
                onpointerup: move |_ev: PointerEvent| {
                    let Some(d) = plan_drag.read().clone() else { return };
                    plan_drag.set(None);
                    drag_target.set(None);
                    if !d.armed {
                        // Plain tap on the handle: offer to move the meal.
                        open_move_sheet(&d, entries, move_sheet, suppress_open);
                        return;
                    }
                    suppress_open.set(true);
                    let target = hit_test_plan_day(d.last_y);
                    if let Some(date) = target {
                        if date != d.origin_date {
                            move_planned_entry(
                                d.entry_id,
                                d.recipe_id,
                                date,
                                entries,
                                error,
                            );
                        }
                    }
                },
                onpointerleave: move |_ev: PointerEvent| {
                    plan_drag.set(None);
                    drag_target.set(None);
                },
                // The OS claims the gesture for scrolling mid-drag.
                onpointercancel: move |_ev: PointerEvent| {
                    plan_drag.set(None);
                    drag_target.set(None);
                },
                for day in days {
                    PlanDaySection {
                        day: day.clone(),
                        drag_entry: drag_snapshot.as_ref().filter(|d| d.armed).map(|d| d.entry_id),
                        is_drag_target: drag_target_snapshot.as_deref() == Some(day.date.as_str()),
                        is_today: day.date == today_value,
                        on_card_down: move |(entry_id, origin_date, recipe_id, y): (
                            i64,
                            String,
                            i64,
                            f64,
                        )| {
                            suppress_open.set(false);
                            plan_drag.set(Some(PlanDrag {
                                entry_id,
                                origin_date,
                                recipe_id,
                                start_y: y,
                                last_y: y,
                                armed: false,
                            }));
                        },
                        on_card_move: move |(entry_id, y): (i64, f64)| {
                            let Some(mut d) = plan_drag.write().clone() else { return };
                            if d.entry_id != entry_id {
                                return;
                            }
                            if !d.armed {
                                if (y - d.start_y).abs() < 8.0 {
                                    return;
                                }
                                d.armed = true;
                            }
                            d.last_y = y;
                            plan_drag.set(Some(d));
                            let target = hit_test_plan_day(y);
                            drag_target.set(target);
                        },
                        on_card_drop: move |(entry_id, y): (i64, f64)| {
                            let Some(d) = plan_drag.read().clone() else { return };
                            if d.entry_id != entry_id {
                                return;
                            }
                            plan_drag.set(None);
                            drag_target.set(None);
                            if !d.armed {
                                // Plain tap on the handle (mouse released on
                                // it): offer to move the meal.
                                open_move_sheet(&d, entries, move_sheet, suppress_open);
                                return;
                            }
                            suppress_open.set(true);
                            let target = hit_test_plan_day(y);
                            if let Some(date) = target {
                                if date != d.origin_date {
                                    move_planned_entry(
                                        d.entry_id,
                                        d.recipe_id,
                                        date,
                                        entries,
                                        error,
                                    );
                                }
                            }
                        },
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
                            if suppress_open.read().clone() {
                                // The click that trails a completed drag.
                                suppress_open.set(false);
                                return;
                            }
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
                    on_cancel: move |_| picker.set(None),
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

            if let Some(target) = move_sheet_snapshot {
                MoveSheet {
                    key: "{target.entry_id}",
                    target,
                    week_start: week_start_date.clone(),
                    today: today_value.clone(),
                    entries: entries.read().clone(),
                    on_move: move |(entry_id, recipe_id, date): (i64, i64, String)| {
                        move_sheet.set(None);
                        move_planned_entry(entry_id, recipe_id, date, entries, error);
                    },
                    on_cancel: move |_| move_sheet.set(None),
                }
            }
        }
    }
}

/// Which rendered day section contains the viewport position `y`?
/// Direct DOM hit-test on web (the web build always runs as wasm). Takes no
/// captured data on purpose: closures inside the `for day in days` rsx loop
/// are `FnMut` and can never move-capture a `Vec`, so the day dates are
/// re-read from the rendered ids instead. The mobile app uses its own
/// eval-bridge variant over cached rects.
#[cfg(target_arch = "wasm32")]
fn hit_test_plan_day(y: f64) -> Option<String> {
    let window = web_sys::window()?;
    let document = window.document()?;
    let days = document.query_selector_all(".plan-day").ok()?;
    for i in 0..days.length() {
        let el = days.get(i)?.dyn_into::<web_sys::Element>().ok()?;
        let id = el.get_attribute("id")?;
        let date = id.strip_prefix("plan-day-")?;
        let rect = el.get_bounding_client_rect();
        if y >= rect.top() && y <= rect.bottom() {
            return Some(date.to_string());
        }
    }
    None
}

/// Host fallback: never called (no UI on the host build), keeps the crate
/// compilable for `cargo test`.
#[cfg(not(target_arch = "wasm32"))]
fn hit_test_plan_day(_y: f64) -> Option<String> {
    None
}

/// Move a planned entry to another day: optimistically re-date it locally,
/// then DELETE + POST (the move endpoint is a delete and a re-add). Any
/// failure refreshes from the server and surfaces the error.
fn move_planned_entry(
    entry_id: i64,
    recipe_id: i64,
    new_date: String,
    mut entries: Signal<Vec<MealPlanEntry>>,
    mut error: Signal<String>,
) {
    entries.with_mut(|v| {
        if let Some(e) = v.iter_mut().find(|e| e.id == entry_id) {
            e.date = new_date.clone();
        }
    });
    spawn(async move {
        let client = reqwest::Client::new();
        let base = api_base();
        let deleted = client
            .delete(format!("{base}/api/meal-plan/{entry_id}"))
            .send()
            .await;
        match deleted {
            Ok(r) if r.status().is_success() => {
                match client
                    .post(format!("{base}/api/meal-plan"))
                    .json(&serde_json::json!({ "date": new_date, "recipe_id": recipe_id }))
                    .send()
                    .await
                {
                    Ok(resp) if resp.status().is_success() => {
                        // Reconcile ids: swap the optimistically re-dated
                        // entry for the server's fresh one.
                        match resp.json::<MealPlanEntry>().await {
                            Ok(created) => entries.with_mut(|v| {
                                v.retain(|e| e.id != entry_id);
                                v.push(created);
                            }),
                            Err(err) => {
                                tracing::error!("meal plan move: unreadable body: {err:#}")
                            }
                        }
                    }
                    Ok(resp) => {
                        tracing::error!("meal plan move re-add failed: {}", resp.status());
                        error.set("Failed to move the recipe.".into());
                        if let Ok(list) = api_get::<Vec<MealPlanEntry>>("/api/meal-plan").await {
                            entries.set(list);
                        }
                    }
                    Err(err) => {
                        tracing::error!("meal plan move request failed: {err:#}");
                        error.set("Failed to move the recipe.".into());
                        if let Ok(list) = api_get::<Vec<MealPlanEntry>>("/api/meal-plan").await {
                            entries.set(list);
                        }
                    }
                }
            }
            Ok(r) => {
                tracing::error!("meal plan move delete failed: {}", r.status());
                error.set("Failed to move the recipe.".into());
                if let Ok(list) = api_get::<Vec<MealPlanEntry>>("/api/meal-plan").await {
                    entries.set(list);
                }
            }
            Err(err) => {
                tracing::error!("meal plan move request failed: {err:#}");
                error.set("Failed to move the recipe.".into());
                if let Ok(list) = api_get::<Vec<MealPlanEntry>>("/api/meal-plan").await {
                    entries.set(list);
                }
            }
        }
    });
}

/// One day section of the meal plan: header (label + ⊕) and the day's
/// recipe cards.
#[component]
fn PlanDaySection(
    day: PlanDay,
    drag_entry: Option<i64>,
    is_drag_target: bool,
    is_today: bool,
    on_card_down: EventHandler<(i64, String, i64, f64)>,
    on_card_move: EventHandler<(i64, f64)>,
    // Only the entry id and pointer y: the drop reads origin/recipe from the
    // drag state, so the handle's listener captures nothing non-Copy (two
    // closures in the `for entry` loop could never both own `entry.date`).
    on_card_drop: EventHandler<(i64, f64)>,
    on_remove: EventHandler<i64>,
    on_add: EventHandler<String>,
    on_open: EventHandler<i64>,
) -> Element {
    let day_entries = day.entries;
    let day_is_empty = day_entries.is_empty();
    let day_date = day.date.clone();
    let day_id = format!("plan-day-{day_date}");
    let day_class = if is_drag_target {
        "plan-day drag-target"
    } else if is_today {
        // Today's card glows a lighter green so the current day reads at a
        // glance in the week pager.
        "plan-day today"
    } else {
        "plan-day"
    };
    rsx! {
            div { class: "{day_class}", id: "{day_id}",
            div { class: "plan-day-head",
                span { class: "plan-day-label", "{day.label}" }
                button {
                    class: "plan-add",
                    r#type: "button",
                    title: "Add recipe",
                    onclick: move |_| on_add.call(day_date.clone()),
                    IconPlus {}
                }
            }
            div { class: "plan-cards",
                for entry in day_entries {
                    div {
                        class: if drag_entry == Some(entry.id) {
                            "plan-card dragging"
                        } else {
                            "plan-card"
                        },
                        key: "{entry.id}",
                        role: "button",
                        tabindex: "0",
                        onclick: move |_| on_open.call(entry.recipe.id),
                        button {
                            class: "plan-drag-handle",
                            r#type: "button",
                            title: "Drag or tap to move",
                            onpointerdown: move |ev: PointerEvent| {
                                ev.stop_propagation();
                                on_card_down.call((
                                    entry.id,
                                    entry.date.clone(),
                                    entry.recipe.id,
                                    ev.client_coordinates().y,
                                ));
                            },
                            // Handle-level move/drop: dispatched events on
                            // the handle reach these own-element listeners
                            // directly (the container's handlers only see
                            // trusted input events).
                            onpointermove: move |ev: PointerEvent| {
                                on_card_move.call((entry.id, ev.client_coordinates().y));
                            },
                            onpointerup: move |ev: PointerEvent| {
                                ev.stop_propagation();
                                on_card_drop.call((entry.id, ev.client_coordinates().y));
                            },
                            IconMenu {}
                        },
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
    on_cancel: EventHandler<()>,
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
            onclick: move |_| on_cancel.call(()),
            div { class: "dialog plan-picker", role: "dialog",
                onclick: move |e: MouseEvent| e.stop_propagation(),
                h2 { class: "dialog-title", "Add to {label}" }
                input {
                    id: "plan-search",
                    r#type: "text",
                    placeholder: "Search recipes…",
                    value: "{search}",
                    oninput: move |e: FormEvent| search.set(e.value()),
                }
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

/// Bottom sheet offering to move a planned meal to another day of the open
/// Sat→Fri week, blaz style: every day shows the recipes already planned on
/// it (thumbnails), or "Nothing planned". Tapping the meal's current day
/// just closes the sheet.
#[component]
fn MoveSheet(
    target: MoveTarget,
    week_start: String,
    today: String,
    entries: Vec<MealPlanEntry>,
    on_move: EventHandler<(i64, i64, String)>,
    on_cancel: EventHandler<()>,
) -> Element {
    // Copy-only pieces: the row closures are FnMut and could never each own
    // a String from `target`.
    let move_entry_id = target.entry_id;
    let move_recipe_id = target.recipe_id;
    let days: Vec<(String, String, Vec<MealPlanEntry>, bool)> = (0..7)
        .map(|i| {
            let date = shift_iso(&week_start, i);
            let label = day_label(&date, &today);
            let planned: Vec<MealPlanEntry> =
                entries.iter().filter(|e| e.date == date).cloned().collect();
            let is_origin = date == target.origin_date;
            (date, label, planned, is_origin)
        })
        .collect();

    rsx! {
        div { class: "sheet-backdrop",
            onclick: move |_| on_cancel.call(()),
            div { class: "sheet", role: "dialog",
                onclick: move |e: MouseEvent| e.stop_propagation(),
                h2 { class: "sheet-title", "Move \u{201c}{target.name}\u{201d} to…" }
                div { class: "sheet-list day-list",
                    for (date, label, planned, is_origin) in days {
                        button {
                            id: "move-day-{date}",
                            class: "day-btn",
                            r#type: "button",
                            onclick: move |_| {
                                if is_origin {
                                    on_cancel.call(());
                                } else {
                                    on_move.call((move_entry_id, move_recipe_id, date.clone()));
                                }
                            },
                            div { class: "day-main",
                                span { class: "day-label", "{label}" }
                                if planned.is_empty() {
                                    span { class: "day-none", "Nothing planned" }
                                } else {
                                    div { class: "day-thumbs",
                                        for entry in planned.iter() {
                                            div { class: "day-thumb",
                                                if let Some(thumb) = &entry.recipe.thumb {
                                                    img { src: "{thumb}", alt: "{entry.recipe.name}" }
                                                } else {
                                                    div { class: "day-thumb-placeholder",
                                                        {entry.recipe.name.chars().next().unwrap_or('?').to_string()}
                                                    }
                                                }
                                                span { class: "day-thumb-name", "{entry.recipe.name}" }
                                            }
                                        }
                                    }
                                }
                            }
                            span { class: "day-chevron", "›" }
                        }
                    }
                }
                div { class: "sheet-actions day-actions",
                    button {
                        id: "move-cancel",
                        class: "dialog-btn day-cancel",
                        r#type: "button",
                        onclick: move |_| on_cancel.call(()),
                        "Cancel"
                    }
                }
            }
        }
    }
}

#[component]
fn Settings() -> Element {
    rsx! {
        LoginGate { SettingsContent {} }
    }
}

/// The Settings UI itself — rendered INSIDE LoginGate so the authed signal
/// context provided there is reachable from here.
#[component]
fn SettingsContent() -> Element {
    let mut authed = use_context::<Signal<Option<bool>>>();
    let mut server_version = use_signal(|| String::from("…"));
    use_effect(move || {
        spawn(async move {
            match api_get::<serde_json::Value>("/api/version").await {
                Ok(v) => {
                    let version = v["version"].as_str().unwrap_or("?").to_string();
                    server_version.set(version);
                }
                Err(err) => tracing::error!("server version fetch failed: {err:#}"),
            }
        });
    });
    rsx! {
        div { class: "page",
                div { class: "card placeholder-card",
                    h1 { "Settings" }
                    p { class: "settings-version",
                        "App v{env!(\"CARGO_PKG_VERSION\")} — Server v{server_version}"
                    }
                    button {
                        id: "logout",
                        class: "btn-primary",
                        r#type: "button",
                        onclick: move |_| {
                            spawn(async move {
                                let _ = reqwest::Client::new()
                                    .post(format!("{}/api/logout", api_base()))
                                    .send()
                                    .await;
                                authed.set(Some(false));
                            });
                        },
                        "Log out"
                    }
                }
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
