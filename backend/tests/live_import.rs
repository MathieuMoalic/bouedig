//! Live regression suite for the recipe importer.
//!
//! These tests fetch REAL recipe pages over the network, so they are marked
//! `#[ignore]` and never run in a plain `cargo test` or CI. Run them
//! explicitly with:
//!
//! ```text
//! just test-import-live
//! # = cargo test -p backend --test live_import -- --ignored --test-threads=1
//! ```
//!
//! The URL list is the shared acceptance set for the importer: every listed
//! page must import with structured data, balanced parentheses in every
//! ingredient line, and no undecoded HTML entities. When a site changes or a
//! new site is added, edit `LIVE_URLS` and run the suite locally.

use backend::recipe_import;

/// The live acceptance set. Paywalled endpoints (e.g. tollbit.bbcgoodfood.com,
/// HTTP 402) are deliberately excluded — the importer reports them as clean
/// client errors, which is covered offline instead.
const LIVE_URLS: &[&str] = &[
    "https://rainbowplantlife.com/vegan-west-african-peanut-stew/",
    "https://minimalistbaker.com/1-pot-lentil-dal/",
    "https://www.loveandlemons.com/vegan-pasta/",
    "https://www.connoisseurusveg.com/vegan-souvlaki/",
    "https://jessicainthekitchen.com/vegan-steak/",
    "https://www.veganricha.com/baked-tofu-curry/",
    "https://veganhuggs.com/vegan-spinach-mushroom-lasagna/",
    "https://www.forksoverknives.com/recipes/vegan-baked-stuffed/lentil-and-rice-loaf-with-mashed-potatoes-and-gravy/",
];

/// Parentheses in every extracted field must balance — this is the core
/// regression for the WPRM/MV-Create `((note))`, `(, prep)` and `(454g)`
/// conventions that used to come out mangled.
fn assert_balanced(label: &str, text: &str) {
    let mut depth: i32 = 0;
    for c in text.chars() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        assert!(depth >= 0, "{label}: unbalanced ')' in {text:?}");
    }
    assert_eq!(depth, 0, "{label}: unbalanced '(' in {text:?}");
}

/// One integration test per URL keeps failures attributable; the loop body
/// holds every assertion.
macro_rules! live_test {
    ($name:ident, $index:literal) => {
        #[tokio::test]
        #[ignore = "live network test — run via `just test-import-live`"]
        async fn $name() {
            let url = LIVE_URLS[$index];
            let started = std::time::Instant::now();
            let preview = backend::recipe_import::import(url, false)
                .await
                .unwrap_or_else(|error| panic!("{url}: import failed: {error}"));
            let recipe = &preview.recipe;

            // Structured data is present.
            assert!(!recipe.name.trim().is_empty(), "{url}: empty name");
            assert!(
                recipe.ingredients.len() >= 5,
                "{url}: only {} ingredients",
                recipe.ingredients.len()
            );
            assert!(
                recipe.instructions.len() >= 3,
                "{url}: only {} instructions",
                recipe.instructions.len()
            );
            assert!(!recipe.yield_amount.trim().is_empty(), "{url}: empty yield");
            assert_eq!(
                recipe.source, url,
                "{url}: source must be the final canonical URL"
            );

            // No mangled fragments anywhere.
            for ingredient in &recipe.ingredients {
                assert_balanced("ingredient name", &ingredient.name);
                if let Some(prep) = &ingredient.prep {
                    assert_balanced("ingredient prep", prep);
                }
                assert!(
                    !ingredient.name.contains("&#"),
                    "{url}: undecoded entity in ingredient {:?}",
                    ingredient.name
                );
            }
            for step in &recipe.instructions {
                assert!(
                    !step.text.contains("&#"),
                    "{url}: undecoded entity in instruction {:?}",
                    step.text
                );
                // HTML tags from the source page must never survive into
                // stored instructions.
                let stray = step.text.chars().collect::<Vec<_>>();
                for window in stray.windows(2) {
                    assert!(
                        window != ['<', '/'] && !(window[0] == '<' && window[1].is_ascii_alphabetic()),
                        "{url}: HTML tag fragment in instruction {:?}",
                        step.text
                    );
                }
            }

            // Most ingredients should carry a parsed quantity (conservative
            // fallback keeps the raw text when they do not).
            let with_quantity = recipe
                .ingredients
                .iter()
                .filter(|i| i.quantity.is_some())
                .count();
            assert!(
                with_quantity * 2 >= recipe.ingredients.len(),
                "{url}: only {with_quantity}/{} ingredients parsed a quantity",
                recipe.ingredients.len()
            );

            // Sections, when an ingredient carries one, must be declared.
            for section in recipe.ingredients.iter().filter_map(|i| i.section.as_ref()) {
                assert!(
                    recipe.sections.iter().any(|s| s == section),
                    "{url}: ingredient section {section:?} missing from sections"
                );
            }
            for section in recipe
                .instructions
                .iter()
                .filter_map(|s| s.section.as_ref())
            {
                assert!(
                    recipe
                        .instruction_sections
                        .iter()
                        .any(|s| s == section),
                    "{url}: step section {section:?} missing from instruction_sections"
                );
            }

            println!(
                "{url}\n  method={:?} confidence={:.2} ingredients={} instructions={} yield={:?} elapsed={}ms",
                preview.method,
                preview.confidence,
                recipe.ingredients.len(),
                recipe.instructions.len(),
                recipe.yield_amount,
                started.elapsed().as_millis()
            );
        }
    };
}

live_test!(live_rainbowplantlife_peanut_stew, 0);
live_test!(live_minimalistbaker_lentil_dal, 1);
live_test!(live_loveandlemons_vegan_pasta, 2);
live_test!(live_connoisseurusveg_souvlaki, 3);
live_test!(live_jessicainthekitchen_vegan_steak, 4);
live_test!(live_veganricha_baked_tofu_curry, 5);
live_test!(live_veganhuggs_spinach_lasagna, 6);
live_test!(live_forksoverknives_lentil_rice_loaf, 7);
