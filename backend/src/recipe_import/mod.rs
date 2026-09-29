//! Recipe web importer: URL → fetched page → JSON-LD (+ HTML fallback) →
//! normalized `shared::RecipeInput` preview with confidence and warnings.
//!
//! Nothing here persists anything — `POST /api/recipes/import` returns a
//! preview the client can edit and save through the normal endpoints.
//!
//! Dependency direction (see PLAN_recipe_parse.md):
//! `pipeline → fetch → JSON-LD / HTML extraction → normalization → shared`.

pub mod fetch;
pub mod html;
pub mod json_ld;
pub mod normalize;
pub mod score;
pub mod types;

use anyhow::Result;
use url::Url;

use shared::RecipeInput;

pub use types::{ExtractionMethod, ImportError, RecipePreview};

/// Run the whole import pipeline against `raw_url`.
///
/// `allow_private` bypasses the SSRF target checks — used only by tests that
/// serve fixture pages from a loopback listener. It must never be enabled by
/// request data in production code paths.
pub async fn import(raw_url: &str, allow_private: bool) -> Result<RecipePreview, ImportError> {
    let started = std::time::Instant::now();
    let url = fetch::validate_url(raw_url, allow_private).await?;
    let host = url.host_str().unwrap_or_default().to_string();

    let page = fetch::fetch_page(&url, allow_private).await?;
    let document = scraper::Html::parse_document(&page.html);
    let title = json_ld::page_title(&document);
    let scan = json_ld::scan(&document);

    let mut warnings = scan.warnings.clone();

    let (recipe, method, html_extraction) = if let Some(schema) = scan.best_candidate(title.as_deref()) {
        // JSON-LD base, merged with the visible HTML for anything missing.
        let html_extraction = html::extract(&document);
        let recipe = merge(schema, &html_extraction, &page.final_url, title.as_deref());
        (recipe, ExtractionMethod::JsonLd, html_extraction)
    } else {
        if scan.candidates.is_empty() {
            warnings.push(
                "no structured recipe data (JSON-LD) was found on the page".to_string(),
            );
        }
        let html_extraction = html::extract(&document);
        let recipe = from_html_only(&html_extraction, &page.final_url)
            .ok_or_else(|| ImportError::ExtractionFailed(
                "neither structured data nor recognizable ingredient/instruction lists were found"
                    .to_string(),
            ))?;
        (recipe, ExtractionMethod::Html, html_extraction)
    };

    // A name is required by the recipe model (merge already fell back to
    // the HTML h1 and the page <title>).
    if recipe.name.trim().is_empty() {
        return Err(ImportError::ExtractionFailed(
            "the page has no recipe name (no JSON-LD name, <h1> or <title>)".into(),
        ));
    }

    // Only lines that read like they were meant to carry a quantity (a
    // leading number the parser could not consume) count as failures —
    // "salt, to taste" legitimately has none and must not warn.
    let ingredients_without_quantity: Vec<String> = recipe
        .ingredients
        .iter()
        .filter(|i| i.quantity.is_none())
        .map(|i| i.name.clone())
        .filter(|name| normalize::looks_quantified(name))
        .collect();

    let extraction_score = score::score(&score::ScoreInput {
        recipe: &recipe,
        from_json_ld: method == ExtractionMethod::JsonLd,
        html_ingredient_count: (!html_extraction.ingredients.is_empty())
            .then(|| html_extraction.ingredients.len()),
        html_only: method == ExtractionMethod::Html,
        ingredients_without_quantity,
    });
    warnings.extend(extraction_score.warnings);

    let preview = RecipePreview {
        recipe,
        method,
        confidence: extraction_score.confidence,
        warnings,
    };

    // Diagnostics: hostname, method, outcome, confidence, elapsed — never
    // the page HTML or the JSON-LD payload.
    match method {
        ExtractionMethod::JsonLd => tracing::info!(
            host = %host,
            method = "json_ld",
            confidence = %format!("{:.2}", preview.confidence),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "recipe import succeeded"
        ),
        ExtractionMethod::Html => tracing::info!(
            host = %host,
            method = "html",
            confidence = %format!("{:.2}", preview.confidence),
            elapsed_ms = started.elapsed().as_millis() as u64,
            "recipe import succeeded (HTML recovery)"
        ),
    }

    Ok(preview)
}

/// Combine a JSON-LD recipe with the HTML recovery: JSON-LD wins for the
/// structured lists (it is the higher-quality source), HTML fills gaps.
fn merge(
    schema: types::SchemaRecipe,
    html: &html::HtmlExtraction,
    final_url: &Url,
    page_title: Option<&str>,
) -> RecipeInput {
    let mut ingredients = normalize::parse_schema_ingredients(&schema.ingredients);
    if ingredients.is_empty() {
        ingredients = html.ingredients.clone();
    } else if !html.ingredient_sections.is_empty() {
        // JSON-LD provided the quantities; the visible HTML provided the
        // section headings. Align them positionally only when the counts
        // agree exactly — otherwise we would risk mislabeling ingredients.
        let grouped_total: usize = html
            .ingredient_sections
            .iter()
            .map(|(_, count)| *count)
            .sum();
        if grouped_total == ingredients.len() {
            let mut index = 0usize;
            for (name, count) in &html.ingredient_sections {
                for ingredient in ingredients[index..index + count].iter_mut() {
                    ingredient.section = Some(name.clone());
                }
                index += count;
            }
        }
    }
    let mut instructions = normalize::parse_instructions(schema.instructions_raw.as_ref());
    if instructions.is_empty() {
        instructions = html.instructions.clone();
    }
    let instruction_sections = normalize::instruction_sections(&instructions);
    let sections = normalize::ingredient_sections(&ingredients);

    RecipeInput {
        // Name: JSON-LD first, then HTML h1, then the page <title>.
        name: schema
            .name
            .clone()
            .or_else(|| html.name.clone())
            .or_else(|| page_title.map(str::to_string))
            .unwrap_or_default(),
        sections,
        ingredients,
        instructions,
        instruction_sections,
        // Notes: the recipe's own description plus a compact timing/author
        // line when the page provided them — never the whole page text.
        notes: compose_notes(&schema),
        yield_amount: schema
            .recipe_yield
            .clone()
            .or_else(|| html.yield_amount.clone())
            .unwrap_or_default(),
        // Source is always the final canonical URL, never the author.
        source: final_url.to_string(),
    }
}

/// Description + timings/author as a short suffix. ISO-8601 durations
/// (`PT1H30M`) are humanized so the notes stay readable.
fn compose_notes(schema: &types::SchemaRecipe) -> String {
    let mut notes = schema.description.clone().unwrap_or_default();
    let mut facts: Vec<String> = Vec::new();
    if let Some(prep) = schema.prep_time.as_deref().and_then(humanize_duration) {
        facts.push(format!("Prep: {prep}"));
    }
    if let Some(cook) = schema.cook_time.as_deref().and_then(humanize_duration) {
        facts.push(format!("Cook: {cook}"));
    }
    // Total time only when prep/cook were not given separately.
    if schema.prep_time.is_none()
        && schema.cook_time.is_none()
    {
        if let Some(total) = schema.total_time.as_deref().and_then(humanize_duration) {
            facts.push(format!("Total time: {total}"));
        }
    }
    if let Some(author) = schema.author.as_deref().map(str::trim).filter(|a| !a.is_empty()) {
        facts.push(format!("by {author}"));
    }
    if !facts.is_empty() {
        let line = facts.join(" · ");
        if notes.is_empty() {
            notes = line;
        } else {
            notes = format!("{notes}\n{line}");
        }
    }
    notes
}

/// `PT1H30M` → "1 h 30 min"; passes through non-duration strings unchanged.
fn humanize_duration(value: &str) -> Option<String> {
    let rest = value.strip_prefix('P')?;
    let mut days_hours_minutes: Vec<(u64, &str)> = Vec::new();
    let mut number = String::new();
    for character in rest.chars() {
        if character.is_ascii_digit() {
            number.push(character);
            continue;
        }
        let unit = match character.to_ascii_uppercase() {
            // ISO-8601 separates date and time parts with 'T'.
            'T' => continue,
            'D' => "d",
            'H' => "h",
            'M' => "min",
            _ => return None,
        };
        let amount = number.parse::<u64>().ok()?;
        days_hours_minutes.push((amount, unit));
        number.clear();
    }
    if days_hours_minutes.is_empty() {
        return None;
    }
    Some(
        days_hours_minutes
            .into_iter()
            .map(|(amount, unit)| format!("{amount} {unit}"))
            .collect::<Vec<_>>()
            .join(" "),
    )
}

/// HTML-only extraction → `RecipeInput` (None when nothing was recovered).
fn from_html_only(html: &html::HtmlExtraction, final_url: &Url) -> Option<RecipeInput> {
    if html.ingredients.is_empty() && html.instructions.is_empty() {
        return None;
    }
    let sections = normalize::ingredient_sections(&html.ingredients);
    let instruction_sections = normalize::instruction_sections(&html.instructions);
    Some(RecipeInput {
        name: html.name.clone().unwrap_or_default(),
        sections,
        ingredients: html.ingredients.clone(),
        instructions: html.instructions.clone(),
        instruction_sections,
        notes: String::new(),
        yield_amount: html.yield_amount.clone().unwrap_or_default(),
        source: final_url.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE_URL: &str = "https://www.example.com/recipe/cake";

    fn parse(html: &str) -> scraper::Html {
        scraper::Html::parse_document(html)
    }

    #[test]
    fn merge_prefers_json_ld_and_fills_gaps_from_html() {
        let document = parse(
            r#"
            <html><head><title>Ignored title</title>
            <script type="application/ld+json">
            {"@type":"Recipe","name":"Chocolate Cake",
             "recipeIngredient":["200 g flour"],
             "recipeInstructions":[{"@type":"HowToStep","text":"Bake."}]}
            </script></head>
            <body><h1>Chocolate Cake!</h1></body></html>
            "#,
        );
        let scan = json_ld::scan(&document);
        let schema = scan.best_candidate(None).expect("recipe candidate");
        let html_extraction = html::extract(&document);
        let recipe = merge(schema, &html_extraction, &Url::parse(PAGE_URL).unwrap(), None);
        assert_eq!(recipe.name, "Chocolate Cake");
        assert_eq!(recipe.ingredients.len(), 1);
        assert_eq!(recipe.ingredients[0].name, "flour");
        assert_eq!(recipe.instructions[0].text, "Bake.");
        assert_eq!(recipe.source, PAGE_URL);
    }

    #[test]
    fn merge_uses_html_when_json_ld_has_no_ingredients() {
        let document = parse(
            r#"
            <html><body>
            <script type="application/ld+json">
            {"@type":"Recipe","name":"Soup",
             "recipeIngredient":[],
             "recipeInstructions":"1. Boil.\n2. Season."}
            </script>
            <h2>Ingredients</h2>
            <ul><li>1 l water</li><li>2 carrots</li></ul>
            </body></html>
            "#,
        );
        let scan = json_ld::scan(&document);
        let schema = scan.best_candidate(None).expect("recipe candidate");
        let html_extraction = html::extract(&document);
        let recipe = merge(schema, &html_extraction, &Url::parse(PAGE_URL).unwrap(), None);
        assert_eq!(recipe.ingredients.len(), 2);
        assert_eq!(recipe.ingredients[0].name, "water");
        // Instructions came from JSON-LD (split blob).
        assert_eq!(recipe.instructions.len(), 2);
    }

    #[test]
    fn html_only_extraction_minimal_requirements() {
        let document = parse(
            r#"
            <html><head><title>Best Soup — ExampleCook</title></head><body>
            <h1>Best Soup</h1>
            <h2>Ingredients</h2>
            <ul><li>1 l water</li><li>salt, to taste</li></ul>
            <h2>Instructions</h2>
            <ol><li>Boil the water.</li><li>Add salt.</li></ol>
            </body></html>
            "#,
        );
        let html_extraction = html::extract(&document);
        let recipe = from_html_only(&html_extraction, &Url::parse(PAGE_URL).unwrap())
            .expect("html extraction should produce a recipe");
        assert_eq!(recipe.name, "Best Soup");
        assert_eq!(recipe.ingredients.len(), 2);
        assert_eq!(recipe.instructions.len(), 2);
        assert_eq!(recipe.source, PAGE_URL);
    }

    #[test]
    fn html_only_with_nothing_recovers_nothing() {
        let document = parse("<html><body><p>Welcome to my blog!</p></body></html>");
        let html_extraction = html::extract(&document);
        assert!(from_html_only(&html_extraction, &Url::parse(PAGE_URL).unwrap()).is_none());
    }

    #[test]
    fn sections_are_derived_from_ingredients() {
        let document = parse(
            r#"
            <html><body>
            <script type="application/ld+json">
            {"@type":"Recipe","name":"Lasagna",
             "recipeIngredient":["250 g pasta sheets","500 g minced beef"],
             "recipeInstructions":[
                {"@type":"HowToSection","name":"Sauce",
                 "itemListElement":[{"@type":"HowToStep","text":"Simmer."}]},
                {"@type":"HowToStep","text":"Layer."}]}
            </script>
            </body></html>
            "#,
        );
        let scan = json_ld::scan(&document);
        let schema = scan.best_candidate(None).expect("recipe candidate");
        let html_extraction = html::extract(&document);
        let recipe = merge(schema, &html_extraction, &Url::parse(PAGE_URL).unwrap(), None);
        assert!(recipe.sections.is_empty(), "no ingredient sections used");
        assert_eq!(recipe.instruction_sections, vec!["Sauce".to_string()]);
        assert_eq!(recipe.instructions[0].section.as_deref(), Some("Sauce"));
        assert_eq!(recipe.instructions[1].section, None);
    }

    #[test]
    fn description_becomes_notes_but_never_the_page() {
        let document = parse(
            r#"
            <html><body>
            <script type="application/ld+json">
            {"@type":"Recipe","name":"X","description":"A lovely cake.",
             "recipeIngredient":["1 egg"],"recipeInstructions":["Mix."]}
            </script>
            </body></html>
            "#,
        );
        let scan = json_ld::scan(&document);
        let schema = scan.best_candidate(None).expect("recipe candidate");
        let recipe = merge(schema, &html::extract(&document), &Url::parse(PAGE_URL).unwrap(), None);
        assert_eq!(recipe.notes, "A lovely cake.");
    }

    // -----------------------------------------------------------------------
    // Fixture-driven tests (backend/tests/recipe_import/*.html)
    // -----------------------------------------------------------------------

    /// Read a fixture page and run the parse/merge pipeline on it.
    fn fixture(name: &str) -> scraper::Html {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/recipe_import")
            .join(name);
        let html = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("fixture {name} unreadable: {e}"));
        parse(&html)
    }

    /// Extract through the full document pipeline (JSON-LD + HTML merge).
    fn extract_fixture(name: &str) -> (RecipeInput, ExtractionMethod) {
        let document = fixture(name);
        let title = json_ld::page_title(&document);
        let scan = json_ld::scan(&document);
        let html_extraction = html::extract(&document);
        if let Some(schema) = scan.best_candidate(title.as_deref()) {
            let recipe = merge(schema, &html_extraction, &Url::parse(PAGE_URL).unwrap(), None);
            (recipe, ExtractionMethod::JsonLd)
        } else {
            (
                from_html_only(&html_extraction, &Url::parse(PAGE_URL).unwrap())
                    .expect("fixture must yield a recipe from HTML"),
                ExtractionMethod::Html,
            )
        }
    }

    #[test]
    fn fixture_json_ld_basic() {
        let (recipe, method) = extract_fixture("json_ld_basic.html");
        assert_eq!(method, ExtractionMethod::JsonLd);
        assert_eq!(recipe.name, "Crêpes Bretonnes");
        // Unicode fraction: ½ tsp salt.
        assert_eq!(recipe.ingredients[1].quantity, Some(0.5));
        assert_eq!(recipe.ingredients[1].unit.as_deref(), Some("tsp"));
        assert_eq!(recipe.ingredients[1].name, "salt");
        // Missing quantity line survives.
        assert_eq!(recipe.ingredients[4].name, "salt");
        assert_eq!(recipe.ingredients[4].quantity, None);
        assert_eq!(recipe.ingredients[4].prep.as_deref(), Some("to taste"));
        // Prep extraction.
        assert_eq!(recipe.ingredients[2].name, "eggs");
        assert_eq!(recipe.ingredients[2].prep.as_deref(), Some("beaten"));
        assert_eq!(recipe.yield_amount, "8 crêpes");
        // Timings humanized into notes (from the description + facts).
        assert!(recipe.notes.contains("Thin buckwheat crêpes"), "{}", recipe.notes);
        assert!(recipe.notes.contains("Prep: 20 min"), "{}", recipe.notes);
        assert_eq!(recipe.instructions.len(), 3);
        assert_eq!(recipe.source, PAGE_URL);
    }

    #[test]
    fn fixture_json_ld_graph() {
        let (recipe, method) = extract_fixture("json_ld_graph.html");
        assert_eq!(method, ExtractionMethod::JsonLd);
        assert_eq!(recipe.name, "Apple Cake");
        assert_eq!(recipe.ingredients.len(), 3);
        assert_eq!(recipe.instructions.len(), 3);
    }

    #[test]
    fn fixture_json_ld_array() {
        let (recipe, method) = extract_fixture("json_ld_array.html");
        assert_eq!(method, ExtractionMethod::JsonLd);
        assert_eq!(recipe.name, "Tomato Soup");
        assert_eq!(recipe.ingredients.len(), 3);
        assert_eq!(recipe.instructions.len(), 3);
    }

    #[test]
    fn fixture_json_ld_type_array() {
        let (recipe, _) = extract_fixture("json_ld_type_array.html");
        assert_eq!(recipe.name, "Onion Tart");
        assert_eq!(recipe.ingredients[0].name, "onions");
        assert_eq!(recipe.ingredients[0].prep.as_deref(), Some("thinly sliced"));
    }

    #[test]
    fn fixture_json_ld_multiple_prefers_complete() {
        let (recipe, _) = extract_fixture("json_ld_multiple.html");
        assert_eq!(recipe.name, "Forest Cake", "the stub recipe must not win");
        assert_eq!(recipe.ingredients.len(), 3);
    }

    #[test]
    fn fixture_how_to_steps() {
        let (recipe, _) = extract_fixture("how_to_steps.html");
        assert_eq!(recipe.name, "Pancakes");
        let texts: Vec<&str> = recipe
            .instructions
            .iter()
            .map(|s| s.text.as_str())
            .collect();
        assert_eq!(
            texts,
            vec![
                "Sift the flour into a bowl.",
                "Whisk in the milk and egg.",
                "Rest the batter for 30 minutes.",
                "Fry until golden.",
            ]
        );
    }

    #[test]
    fn fixture_how_to_sections() {
        let (recipe, _) = extract_fixture("how_to_sections.html");
        assert_eq!(recipe.instruction_sections, vec!["Meat sauce", "Assembly"]);
        assert_eq!(recipe.instructions.len(), 5);
        assert_eq!(recipe.instructions[0].section.as_deref(), Some("Meat sauce"));
        assert_eq!(recipe.instructions[1].section.as_deref(), Some("Meat sauce"));
        assert_eq!(recipe.instructions[2].section.as_deref(), Some("Assembly"));
        assert_eq!(recipe.instructions[3].section.as_deref(), Some("Assembly"));
        assert_eq!(recipe.instructions[4].section, None);
        assert_eq!(recipe.instructions[4].text, "Bake for 45 minutes.");
    }

    #[test]
    fn fixture_grouped_ingredients_sections_align() {
        // JSON-LD has 5 flat ingredients; the HTML groups the same 5 under
        // two subheadings — sections must be mapped positionally.
        let (recipe, _) = extract_fixture("grouped_ingredients.html");
        assert_eq!(recipe.ingredients.len(), 5);
        assert_eq!(recipe.sections, vec!["For the dough", "For the filling"]);
        assert_eq!(recipe.ingredients[0].section.as_deref(), Some("For the dough"));
        assert_eq!(recipe.ingredients[1].section.as_deref(), Some("For the dough"));
        assert_eq!(
            recipe.ingredients[2].section.as_deref(),
            Some("For the filling")
        );
        assert_eq!(recipe.ingredients[4].section.as_deref(), Some("For the filling"));
    }

    #[test]
    fn fixture_malformed_json_ld_falls_back_to_html() {
        let (recipe, method) = extract_fixture("malformed_json_ld.html");
        assert_eq!(method, ExtractionMethod::Html);
        assert_eq!(recipe.name, "Recovery Stew");
        assert_eq!(recipe.ingredients.len(), 3);
        assert_eq!(recipe.ingredients[0].quantity, Some(600.0));
        assert_eq!(recipe.instructions.len(), 2);
        // The malformed JSON-LD is surfaced as a warning by the scanner.
        let document = fixture("malformed_json_ld.html");
        let scan = json_ld::scan(&document);
        assert!(scan
            .warnings
            .iter()
            .any(|w| w.contains("malformed")), "{:?}", scan.warnings);
    }

    #[test]
    fn fixture_no_json_ld_uses_microdata() {
        let (recipe, method) = extract_fixture("no_json_ld.html");
        assert_eq!(method, ExtractionMethod::Html);
        assert_eq!(recipe.name, "Galette Complète");
        assert_eq!(recipe.yield_amount, "2 galettes");
        assert_eq!(recipe.ingredients.len(), 4);
        assert_eq!(recipe.ingredients[2].prep.as_deref(), Some("grated"));
        assert_eq!(recipe.instructions.len(), 3);
    }

    #[test]
    fn fixture_incomplete_recipe_title_fallback() {
        // The Recipe object has a name, so JSON-LD wins with almost nothing
        // else; the title fallback is exercised by the *scan* level.
        let (recipe, method) = extract_fixture("incomplete_recipe.html");
        assert_eq!(method, ExtractionMethod::JsonLd);
        assert_eq!(recipe.name, "Mystery Dish");
        assert!(recipe.ingredients.is_empty());
        assert!(recipe.instructions.is_empty());

        // A nameless recipe object picks up the page <title> instead.
        let document = parse(
            r#"
            <html><head><title>Fallback Title — Site</title>
            <script type="application/ld+json">
            {"@type":"Recipe","recipeIngredient":["1 egg"]}
            </script></head><body></body></html>
            "#,
        );
        let scan = json_ld::scan(&document);
        let schema = scan.best_candidate(None).expect("candidate");
        let merged = merge(
            schema,
            &html::extract(&document),
            &Url::parse(PAGE_URL).unwrap(),
            Some("Fallback Title — Site"),
        );
        assert_eq!(merged.name, "Fallback Title — Site");
    }

    #[test]
    fn empty_recipe_object_yields_low_confidence_warnings() {
        let recipe = shared::RecipeInput {
            name: "Empty".into(),
            sections: vec![],
            ingredients: vec![],
            instructions: vec![],
            instruction_sections: vec![],
            notes: String::new(),
            yield_amount: String::new(),
            source: String::new(),
        };
        let extraction_score = score::score(&score::ScoreInput {
            recipe: &recipe,
            from_json_ld: true,
            html_ingredient_count: None,
            html_only: false,
            ingredients_without_quantity: Vec::new(),
        });
        let joined = extraction_score.warnings.join("; ");
        assert!(joined.contains("recipe has no ingredients"));
        assert!(joined.contains("recipe has no instructions"));
        assert!(extraction_score.confidence < 0.5);
    }

    // -------------------------------------------------------------------
    // Captured real-world WPRM/MV-Create pages (trimmed). These lock the
    // importer to the exact structures the live sites emit — see
    // `tests/live_import.rs` for the always-live counterparts.
    // -------------------------------------------------------------------

    fn balanced(ingredient: &shared::Ingredient) {
        let check = |text: &str, what: &str| {
            let mut depth: i32 = 0;
            for c in text.chars() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                assert!(depth >= 0, "unbalanced ')' in {what}: {text:?}");
            }
            assert_eq!(depth, 0, "unbalanced '(' in {what}: {text:?}");
        };
        check(&ingredient.name, "name");
        if let Some(prep) = &ingredient.prep {
            check(prep, "prep");
        }
    }

    #[test]
    fn fixture_wprm_metric_parens_rainbowplantlife() {
        let (recipe, method) = extract_fixture("wprm_metric_parens.html");
        assert_eq!(method, ExtractionMethod::JsonLd);
        assert_eq!(recipe.name, "Gambian Peanut Stew");
        assert_eq!(recipe.yield_amount, "6");
        assert!(recipe.ingredients.len() >= 15);
        // Verbatim site strings, parsed conservatively.
        assert_eq!(recipe.ingredients[0].name, "unrefined coconut oil (use refined for a neutral flavor)");
        assert_eq!(recipe.ingredients[0].quantity, Some(1.5));
        assert_eq!(recipe.ingredients[1].name, "large yellow onion");
        assert_eq!(recipe.ingredients[1].prep.as_deref(), Some("diced"));
        // "1-2 jalapeño peppers, (diced (see Note 1) )"
        assert_eq!(recipe.ingredients[4].quantity, Some(1.0), "range lower bound");
        assert_eq!(recipe.ingredients[4].name, "jalapeño peppers");
        // "4 cups (945 mL) low-sodium vegetable broth"
        assert_eq!(recipe.ingredients[12].unit.as_deref(), Some("cup"));
        assert_eq!(
            recipe.ingredients[12].name,
            "(945 mL) low-sodium vegetable broth"
        );
        // "½ cup (128g) creamy peanut butter ((no sugar added) )"
        assert_eq!(recipe.ingredients[15].name, "(128g) creamy peanut butter");
        assert_eq!(recipe.ingredients[15].prep.as_deref(), Some("no sugar added"));
        // "1 (15-ounce/425g)  can cannellini beans, (drained and rinsed)"
        assert_eq!(recipe.ingredients[16].unit.as_deref(), Some("can"));
        assert_eq!(recipe.ingredients[16].name, "(15-ounce/425g) cannellini beans");
        assert_eq!(recipe.ingredients[16].prep.as_deref(), Some("drained and rinsed"));
        for ingredient in &recipe.ingredients {
            balanced(ingredient);
        }
        // JSON-LD entities are decoded in instruction text.
        assert!(recipe.instructions.iter().all(|s| !s.text.contains("&#")));
    }

    #[test]
    fn fixture_wprm_doubled_parens_minimalistbaker() {
        let (recipe, _) = extract_fixture("wprm_doubled_parens.html");
        assert_eq!(recipe.name, "1-Pot Lentil Green Curry");
        assert_eq!(recipe.yield_amount, "4");
        // "2 1/4 cups light coconut milk*  ((canned is best))"
        assert_eq!(recipe.ingredients[6].quantity, Some(2.25));
        assert_eq!(recipe.ingredients[6].unit.as_deref(), Some("cup"));
        assert_eq!(recipe.ingredients[6].name, "light coconut milk*");
        assert_eq!(recipe.ingredients[6].prep.as_deref(), Some("canned is best"));
        // "1 cup green lentils* ((well rinsed and drained))"
        assert_eq!(recipe.ingredients[10].name, "green lentils*");
        assert_eq!(recipe.ingredients[10].prep.as_deref(), Some("well rinsed and drained"));
        for ingredient in &recipe.ingredients {
            balanced(ingredient);
        }
        assert!(recipe.instructions.iter().all(|s| !s.text.contains("&#")));
    }

    #[test]
    fn fixture_wprm_comma_parens_veganhuggs() {
        let (recipe, _) = extract_fixture("wprm_comma_parens.html");
        assert_eq!(recipe.name, "Vegan Lasagna");
        assert_eq!(recipe.yield_amount, "10");
        // "1 large  onion (, finely diced)" — WPRM comma style.
        assert_eq!(recipe.ingredients[2].name, "large onion");
        assert_eq!(recipe.ingredients[2].prep.as_deref(), Some("finely diced"));
        // "Salt (, to taste)"
        assert_eq!(recipe.ingredients[9].name, "Salt");
        assert_eq!(recipe.ingredients[9].prep.as_deref(), Some("to taste"));
        // "12 cups fresh spinach ((loosely packed) rough chopped (about 14 oz))"
        assert_eq!(recipe.ingredients[8].quantity, Some(12.0));
        assert_eq!(
            recipe.ingredients[8].name,
            "fresh spinach (loosely packed) rough chopped"
        );
        assert_eq!(recipe.ingredients[8].prep.as_deref(), Some("about 14 oz"));
        // "15  lasagna noodles ((*see note))"
        assert_eq!(recipe.ingredients[12].name, "lasagna noodles");
        assert_eq!(recipe.ingredients[12].prep.as_deref(), Some("*see note"));
        for ingredient in &recipe.ingredients {
            balanced(ingredient);
        }
    }
}
