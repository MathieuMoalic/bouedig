//! JSON-LD extraction: find every Schema.org `Recipe` object embedded in
//! `<script type="application/ld+json">` blocks, wherever it hides — at the
//! top level, inside `@graph`, inside arrays, or nested deeper — and pick the
//! most useful candidate when several are present.

use scraper::{ElementRef, Html, Selector};
use serde_json::Value;

use super::types::SchemaRecipe;

/// Result of scanning a page's JSON-LD blocks.
#[derive(Debug, Default)]
pub struct JsonLdScan {
    pub candidates: Vec<SchemaRecipe>,
    pub warnings: Vec<String>,
}

impl JsonLdScan {
    /// Pick the best Recipe candidate:
    /// 1. one with both ingredients and instructions,
    /// 2. otherwise the most complete,
    /// 3. preferring names that resemble the page title,
    /// 4. falling back to the first sufficiently complete one (ties keep the
    ///    original document order).
    pub fn best_candidate(&self, page_title: Option<&str>) -> Option<SchemaRecipe> {
        let title = page_title.unwrap_or_default().to_lowercase();
        self.candidates.iter().enumerate().fold(
            None::<(usize, (bool, u32, bool))>,
            |best, (index, candidate)| {
                let rank = (
                    candidate.has_ingredients_and_instructions(),
                    candidate.completeness(),
                    name_resembles_title(candidate.name.as_deref(), &title),
                );
                match best {
                    // Strictly better wins; ties keep the earlier candidate.
                    Some((_, best_rank)) if rank <= best_rank => best,
                    _ => Some((index, rank)),
                }
            },
        )
        .map(|(index, _)| self.candidates[index].clone())
    }
}

/// Does the candidate name overlap with the page `<title>`? Titles almost
/// always contain the recipe name plus site branding ("… | Budget Bytes").
fn name_resembles_title(name: Option<&str>, title: &str) -> bool {
    let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
        return false;
    };
    if title.is_empty() {
        return false;
    }
    let name = name.to_lowercase();
    title.contains(&name) || name.contains(&title)
}

/// Scan every `application/ld+json` script of the document.
pub fn scan(document: &Html) -> JsonLdScan {
    let mut scan = JsonLdScan::default();
    let selector = match Selector::parse("script[type=\"application/ld+json\"]") {
        Ok(sel) => sel,
        Err(_) => return scan,
    };
    for element in document.select(&selector) {
        let text: String = element.text().collect::<Vec<_>>().join("");
        if text.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<Value>(&text) {
            Ok(value) => collect_recipes(&value, &mut scan.candidates),
            Err(_) => scan
                .warnings
                .push("some structured data (JSON-LD) on the page was malformed and was ignored"
                    .to_string()),
        }
    }
    if scan.candidates.len() > 1 {
        scan.warnings
            .push("multiple Recipe objects were found".to_string());
    }
    scan
}

/// Recursively walk a JSON value collecting every object whose `@type`
/// contains `Recipe`. Handles arrays, `@graph` and arbitrary nesting.
fn collect_recipes(value: &Value, out: &mut Vec<SchemaRecipe>) {
    match value {
        Value::Array(items) => {
            for item in items {
                collect_recipes(item, out);
            }
        }
        Value::Object(map) => {
            if type_contains_recipe(map.get("@type")) {
                out.push(parse_recipe(map));
                return;
            }
            // Not a recipe itself: keep digging (@graph, arbitrary wrappers).
            for child in map.values() {
                collect_recipes(child, out);
            }
        }
        _ => {}
    }
}

/// `@type` may be a string or an array of strings; match case-insensitively.
fn type_contains_recipe(type_value: Option<&Value>) -> bool {
    match type_value {
        Some(Value::String(s)) => s.eq_ignore_ascii_case("Recipe"),
        Some(Value::Array(items)) => items
            .iter()
            .any(|item| item.as_str().is_some_and(|s| s.eq_ignore_ascii_case("Recipe"))),
        _ => false,
    }
}

/// Flatten one Recipe object into the shape-tolerant [`SchemaRecipe`].
fn parse_recipe(map: &serde_json::Map<String, Value>) -> SchemaRecipe {
    SchemaRecipe {
        name: value_to_string(map.get("name")),
        description: value_to_string(map.get("description")),
        image: image_url(map.get("image")),
        author: author_name(map.get("author")),
        ingredients: string_list(map.get("recipeIngredient")),
        instructions_raw: map.get("recipeInstructions").cloned(),
        recipe_yield: value_to_joined_string(map.get("recipeYield")),
        prep_time: value_to_string(map.get("prepTime")),
        cook_time: value_to_string(map.get("cookTime")),
        total_time: value_to_string(map.get("totalTime")),
        nutrition: map.get("nutrition").cloned(),
        source_url: None,
    }
}

/// String, array of strings or `{ "name": … }` → first meaningful string.
pub(super) fn value_to_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => non_empty(s.trim()),
        Value::Array(items) => items.iter().find_map(|item| value_to_string(Some(item))),
        Value::Object(map) => ["name", "text", "url", "@id"]
            .iter()
            .find_map(|key| value_to_string(map.get(*key))),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

/// Like [`value_to_string`] but joins arrays (`recipeYield` occasionally
/// shows up as a list like `["8 servings", "1 cake"]`).
fn value_to_joined_string(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Array(items) => {
            let parts: Vec<String> = items
                .iter()
                .filter_map(|item| value_to_string(Some(item)))
                .collect();
            non_empty(&parts.join(", "))
        }
        other => value_to_string(Some(other)),
    }
}

fn image_url(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::String(s) => non_empty(s.trim()),
        Value::Array(items) => items.iter().find_map(|item| image_url(Some(item))),
        Value::Object(map) => map.get("url").and_then(|u| value_to_string(Some(u))),
        _ => None,
    }
}

fn author_name(value: Option<&Value>) -> Option<String> {
    match value? {
        Value::Object(map) => map.get("name").and_then(|n| value_to_string(Some(n))),
        Value::Array(items) => {
            let names: Vec<String> = items
                .iter()
                .filter_map(|item| author_name(Some(item)))
                .collect();
            non_empty(&names.join(", "))
        }
        Value::String(s) => non_empty(s.trim()),
        _ => None,
    }
}

/// `recipeIngredient` as string or array of strings.
fn string_list(value: Option<&Value>) -> Vec<String> {
    match value {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => non_empty(s.trim())
            .map(|s| vec![s])
            .unwrap_or_default(),
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(|item| value_to_string(Some(item)))
            .collect(),
        Some(_) => Vec::new(),
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// Convenience accessor for the page `<title>` used by candidate selection.
pub fn page_title(document: &Html) -> Option<String> {
    let selector = Selector::parse("title").ok()?;
    document
        .select(&selector)
        .next()
        .map(|el: ElementRef| el.text().collect::<String>())
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}
