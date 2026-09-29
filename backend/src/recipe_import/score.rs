//! Confidence scoring: judge an extraction by observable quality only —
//! what is present, how much, and whether the two extraction strategies
//! agree. The score is informational; a low score never blocks a preview.

/// Confidence + human-readable warnings for one extraction.
#[derive(Debug, Clone)]
pub struct ExtractionScore {
    pub confidence: f32,
    pub warnings: Vec<String>,
}

/// Inputs the scorer looks at.
pub struct ScoreInput<'a> {
    pub recipe: &'a shared::RecipeInput,
    /// True when the JSON-LD path produced the recipe (vs HTML recovery).
    pub from_json_ld: bool,
    /// Ingredient count recovered from the visible HTML, when HTML
    /// extraction ran: used to detect JSON-LD/HTML agreement.
    pub html_ingredient_count: Option<usize>,
    /// The page provided no structured data at all.
    pub html_only: bool,
    /// Names of the ingredient lines whose quantity could not be parsed.
    pub ingredients_without_quantity: Vec<String>,
}

pub fn score(input: &ScoreInput<'_>) -> ExtractionScore {
    let recipe = input.recipe;
    let mut warnings = Vec::new();
    let mut confidence = 0.0f32;

    // Name (required by the model anyway).
    if recipe.name.trim().is_empty() {
        warnings.push("recipe has no name".to_string());
    } else {
        confidence += 0.15;
    }

    // Ingredients.
    let ingredient_count = recipe.ingredients.len();
    if ingredient_count == 0 {
        warnings.push("recipe has no ingredients".to_string());
    } else {
        confidence += 0.25;
        // A recipe with 2–30 ingredients is typical; 1 or 50+ is suspicious
        // but not disqualifying.
        if (2..=30).contains(&ingredient_count) {
            confidence += 0.05;
        } else {
            warnings.push(format!(
                "unusual ingredient count ({ingredient_count}) — please review"
            ));
        }
        if input.ingredients_without_quantity.is_empty() {
            confidence += 0.05;
        } else {
            // Name the offending lines so the user can fix them directly.
            let names = &input.ingredients_without_quantity;
            let mut message = if names.len() == 1 {
                "ingredient quantity could not be parsed".to_string()
            } else {
                format!(
                    "{} ingredient quantities could not be parsed",
                    names.len()
                )
            };
            let shown: Vec<String> = names.iter().take(3).map(|n| format!("\"{n}\"")).collect();
            message.push_str(": ");
            message.push_str(&shown.join(", "));
            if names.len() > 3 {
                message.push_str(&format!(" and {} more", names.len() - 3));
            }
            warnings.push(message);
        }
    }

    // Instructions.
    let step_count = recipe.instructions.len();
    if step_count == 0 {
        warnings.push("recipe has no instructions".to_string());
    } else {
        confidence += 0.25;
        if (1..=40).contains(&step_count) {
            confidence += 0.05;
        }
    }

    // Yield.
    if recipe.yield_amount.trim().is_empty() {
        warnings.push("recipe has no yield".to_string());
    } else {
        confidence += 0.05;
    }

    // Provenance quality.
    if input.from_json_ld {
        confidence += 0.15;
        // Agreement between JSON-LD and the visible HTML is a strong signal.
        if let Some(html_count) = input.html_ingredient_count {
            if html_count > 0 && html_count == ingredient_count {
                confidence += 0.05;
            }
        }
    } else if input.html_only {
        warnings.push("recipe was recovered from HTML instead of structured data".to_string());
    }

    ExtractionScore {
        confidence: confidence.min(0.99),
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_recipe() -> shared::RecipeInput {
        shared::RecipeInput {
            name: "Cake".into(),
            sections: vec![],
            ingredients: (0..5)
                .map(|i| shared::Ingredient {
                    quantity: Some(i as f64 + 1.0),
                    unit: Some("g".into()),
                    name: format!("ingredient {i}"),
                    prep: None,
                    section: None,
                })
                .collect(),
            instructions: (0..5)
                .map(|i| shared::InstructionStep { text: format!("step {i}"), section: None })
                .collect(),
            instruction_sections: vec![],
            notes: String::new(),
            yield_amount: "8 servings".into(),
            source: "https://example.com/cake".into(),
        }
    }

    #[test]
    fn complete_json_ld_recipe_scores_high() {
        let recipe = base_recipe();
        let result = score(&ScoreInput {
            recipe: &recipe,
            from_json_ld: true,
            html_ingredient_count: Some(5),
            html_only: false,
            ingredients_without_quantity: Vec::new(),
        });
        assert!(result.confidence >= 0.9, "confidence {}", result.confidence);
        assert!(result.warnings.is_empty(), "{:?}", result.warnings);
    }

    #[test]
    fn empty_recipe_warns_about_everything() {
        let recipe = shared::RecipeInput {
            name: String::new(),
            sections: vec![],
            ingredients: vec![],
            instructions: vec![],
            instruction_sections: vec![],
            notes: String::new(),
            yield_amount: String::new(),
            source: String::new(),
        };
        let result = score(&ScoreInput {
            recipe: &recipe,
            from_json_ld: false,
            html_ingredient_count: None,
            html_only: true,
            ingredients_without_quantity: Vec::new(),
        });
        assert!(result.confidence < 0.3);
        let joined = result.warnings.join("; ");
        for expected in [
            "recipe has no name",
            "recipe has no ingredients",
            "recipe has no instructions",
            "recovered from HTML",
        ] {
            assert!(joined.contains(expected), "missing warning '{expected}': {joined}");
        }
    }

    #[test]
    fn unparsed_quantities_are_reported() {
        let mut recipe = base_recipe();
        recipe.ingredients[0].quantity = None;
        recipe.ingredients[1].quantity = None;
        let result = score(&ScoreInput {
            recipe: &recipe,
            from_json_ld: true,
            html_ingredient_count: None,
            html_only: false,
            ingredients_without_quantity: vec!["salt".into(), "pepper".into()],
        });
        let joined = result.warnings.join("; ");
        assert!(
            joined.contains("2 ingredient quantities could not be parsed"),
            "{joined}"
        );
        assert!(joined.contains("\"salt\""), "{joined}");
        assert!(joined.contains("\"pepper\""), "{joined}");
    }

    #[test]
    fn unparsed_quantities_name_the_lines() {
        let mut recipe = base_recipe();
        for ingredient in recipe.ingredients.iter_mut().take(5) {
            ingredient.quantity = None;
        }
        let result = score(&ScoreInput {
            recipe: &recipe,
            from_json_ld: true,
            html_ingredient_count: None,
            html_only: false,
            ingredients_without_quantity: (0..5).map(|i| format!("line {i}")).collect(),
        });
        let joined = result.warnings.join("; ");
        assert!(
            joined.contains("5 ingredient quantities could not be parsed"),
            "{joined}"
        );
        assert!(joined.contains("\"line 2\""), "{joined}");
        assert!(joined.contains("and 2 more"), "{joined}");
    }
}
