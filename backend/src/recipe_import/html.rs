//! Conservative HTML fallback extraction.
//!
//! Only runs when JSON-LD is missing, malformed or incomplete. First the
//! schema.org microdata attributes (`[itemprop=…]`), then a small set of
//! structural heuristics (ingredient-ish `<li>` lists under headings that
//! say "ingredients", ordered instruction lists). Everything recovered here
//! is *merged* under the JSON-LD result rather than replacing it.

use scraper::{ElementRef, Html, Selector};
use shared::{Ingredient, InstructionStep};

use super::normalize::{clean_text, parse_ingredient_line};

/// What the HTML pass recovered. All fields are optional — the merge step
/// only fills gaps in the JSON-LD result.
#[derive(Debug, Default)]
pub struct HtmlExtraction {
    pub name: Option<String>,
    pub ingredients: Vec<Ingredient>,
    pub instructions: Vec<InstructionStep>,
    pub yield_amount: Option<String>,
    /// Ingredient lines grouped under subheadings ("For the crust: …"),
    /// in display order. Used to map sections onto JSON-LD ingredients.
    pub ingredient_sections: Vec<(String, usize)>,
}

pub fn extract(document: &Html) -> HtmlExtraction {
    let mut out = HtmlExtraction {
        name: microdata_name(document).or_else(|| heading_name(document)),
        ingredients: microdata_ingredients(document),
        instructions: microdata_instructions(document),
        yield_amount: microdata_yield(document),
        ingredient_sections: Vec::new(),
    };

    // Structural fallback for whatever microdata missed; it carries section
    // grouping that microdata cannot express.
    if out.ingredients.is_empty() {
        let grouped = structural_ingredients_grouped(document);
        out.ingredient_sections = section_runs(&grouped);
        out.ingredients = grouped;
    }
    if out.instructions.is_empty() {
        out.instructions = structural_instructions(document);
    }
    out
}

/// Grouped structural ingredient extraction (sections from subheadings are
/// embedded in each ingredient).
fn structural_ingredients_grouped(document: &Html) -> Vec<Ingredient> {
    grouped_list_items_under_heading(document, "ingredient")
        .into_iter()
        .map(|(section, line)| {
            let mut ingredient = parse_ingredient_line(&line);
            ingredient.section = section;
            ingredient
        })
        .filter(|ingredient| !ingredient.name.is_empty())
        .collect()
}

/// Distinct consecutive section names with their item counts, e.g.
/// `[("For the crust", 2), ("For the filling", 3)]`.
fn section_runs(ingredients: &[Ingredient]) -> Vec<(String, usize)> {
    let mut runs: Vec<(String, usize)> = Vec::new();
    for ingredient in ingredients {
        if let Some(section) = ingredient.section.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            match runs.last_mut() {
                Some((last, count)) if last == section => *count += 1,
                _ => runs.push((section.to_string(), 1)),
            }
        }
    }
    runs
}

fn select_one_text(document: &Html, selector: &str) -> Option<String> {
    let selector = Selector::parse(selector).ok()?;
    document
        .select(&selector)
        .next()
        .map(|el| clean_text(&el.text().collect::<String>()))
        .filter(|t| !t.is_empty())
}

fn microdata_name(document: &Html) -> Option<String> {
    select_one_text(document, "[itemprop=\"name\"]").filter(|t| t.len() < 200)
}

fn heading_name(document: &Html) -> Option<String> {
    select_one_text(document, "h1").filter(|t| t.len() < 200)
}

fn microdata_yield(document: &Html) -> Option<String> {
    select_one_text(document, "[itemprop=\"recipeYield\"]")
}

fn microdata_ingredients(document: &Html) -> Vec<Ingredient> {
    let Ok(selector) = Selector::parse("[itemprop=\"recipeIngredient\"]") else {
        return Vec::new();
    };
    document
        .select(&selector)
        .map(|el| parse_ingredient_line(&el.text().collect::<String>()))
        .filter(|ingredient| !ingredient.name.is_empty())
        .collect()
}

fn microdata_instructions(document: &Html) -> Vec<InstructionStep> {
    // itemprop=recipeInstructions usually wraps a list; each child list item
    // (or the matched element itself) is one step.
    let Ok(selector) = Selector::parse("[itemprop=\"recipeInstructions\"]") else {
        return Vec::new();
    };
    let mut steps = Vec::new();
    let li = Selector::parse("li").expect("valid selector");
    for container in document.select(&selector) {
        let list_items: Vec<String> = container
            .select(&li)
            .map(|li| clean_text(&li.text().collect::<String>()))
            .filter(|t| !t.is_empty())
            .collect();
        if !list_items.is_empty() {
            steps.extend(
                list_items
                    .into_iter()
                    .map(|text| InstructionStep { text, section: None }),
            );
        } else {
            let text = clean_text(&container.text().collect::<String>());
            if !text.is_empty() && text.len() < 2000 {
                steps.push(InstructionStep { text, section: None });
            }
        }
    }
    steps
}

/// Instruction lists under headings about the method / directions / steps.
fn structural_instructions(document: &Html) -> Vec<InstructionStep> {
    for keyword in ["instruction", "method", "preparation", "direction", "step"] {
        let steps: Vec<InstructionStep> = list_items_under_heading(document, keyword)
            .into_iter()
            .map(|text| InstructionStep { text, section: None })
            .collect();
        if !steps.is_empty() {
            return steps;
        }
    }
    Vec::new()
}

/// `<ul>`/`<ol>` item texts that live under a heading containing `keyword`
/// (case-insensitive). Walks the tree in document order, tracking the most
/// recent heading — that is exactly how recipe pages are laid out.
fn list_items_under_heading(document: &Html, keyword: &str) -> Vec<String> {
    grouped_list_items_under_heading(document, keyword)
        .into_iter()
        .map(|(_, text)| text)
        .collect()
}

/// Like [`list_items_under_heading`] but tracks subheadings inside the
/// matched section: items following a deeper heading (`<h3>`+) inside the
/// matched section carry that subheading as their section.
fn grouped_list_items_under_heading(
    document: &Html,
    keyword: &str,
) -> Vec<(Option<String>, String)> {
    let mut items = Vec::new();
    let mut heading: Option<(usize, String)> = None;
    let mut subheading: Option<String> = None;
    walk_for_lists(
        document.root_element(),
        keyword,
        &mut heading,
        &mut subheading,
        &mut items,
    );
    items
}

fn walk_for_lists(
    element: ElementRef<'_>,
    keyword: &str,
    heading: &mut Option<(usize, String)>,
    subheading: &mut Option<String>,
    items: &mut Vec<(Option<String>, String)>,
) {
    for child in element.children() {
        let Some(child) = ElementRef::wrap(child) else {
            continue; // text / comment nodes
        };
        let tag = child.value().name();
        if let Some(level) = heading_level(tag) {
            // Keep the original text for section names; match keywords on a
            // lowercased copy.
            let raw = clean_text(&child.text().collect::<String>());
            let lowered = raw.to_lowercase();
            if let Some((matched_level, matched_text)) = heading.as_ref() {
                // A deeper heading inside the matched section groups items.
                if matched_text.contains(keyword) && level > *matched_level {
                    *subheading = (!raw.is_empty()).then_some(raw);
                    continue;
                }
            }
            *heading = Some((level, lowered));
            *subheading = None;
        } else if matches!(tag, "ul" | "ol") {
            if heading
                .as_ref()
                .is_some_and(|(_, text)| text.contains(keyword))
            {
                for li in child.select(&Selector::parse("li").expect("valid selector")) {
                    let text = clean_text(&li.text().collect::<String>());
                    if !text.is_empty() && text.len() < 300 {
                        items.push((subheading.clone(), text));
                    }
                }
            }
            // Lists nested under a non-matching heading: do not recurse into
            // them; their children cannot belong to a better heading.
        } else {
            walk_for_lists(child, keyword, heading, subheading, items);
        }
    }
}

fn heading_level(tag: &str) -> Option<usize> {
    match tag {
        "h1" => Some(1),
        "h2" => Some(2),
        "h3" => Some(3),
        "h4" => Some(4),
        "h5" => Some(5),
        "h6" => Some(6),
        _ => None,
    }
}
