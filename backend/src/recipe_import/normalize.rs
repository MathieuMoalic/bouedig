//! Normalization of raw extracted data into `shared` types.
//!
//! * instructions: string / `HowToStep` / `HowToSection` shapes → ordered
//!   [`InstructionStep`]s whose `section` grouping survives
//! * ingredients: raw text lines → quantity / unit / name / prep with a
//!   modest deterministic parser (no NLP dependency). Uncertain lines stay
//!   intact as `name` rather than being corrupted.

use shared::{Ingredient, InstructionStep};

use super::json_ld::value_to_string;
use serde_json::Value;

/// Parse the raw `recipeInstructions` value (all common Schema.org shapes)
/// into ordered steps with their sections preserved.
pub fn parse_instructions(raw: Option<&Value>) -> Vec<InstructionStep> {
    let Some(raw) = raw else { return Vec::new() };
    match raw {
        // A single free-text blob: split it on sentence-ish boundaries so the
        // UI can number steps.
        Value::String(text) => split_text_steps(text),
        Value::Array(items) => {
            let mut steps = Vec::new();
            for item in items {
                match item {
                    // HowToSection: name + itemListElement of HowToSteps.
                    Value::Object(map) if map.get("@type").and_then(Value::as_str).map(|t| t.eq_ignore_ascii_case("HowToSection")).unwrap_or(false) => {
                        let section = map.get("name").and_then(|n| value_to_string(Some(n)));
                        for element in map
                            .get("itemListElement")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            if let Some(step) = how_to_step_text(element) {
                                steps.push(InstructionStep {
                                    text: step,
                                    section: section.clone(),
                                });
                            }
                        }
                    }
                    // HowToStep or bare object with a text field.
                    Value::Object(_) => {
                        if let Some(text) = how_to_step_text(item) {
                            steps.push(InstructionStep { text, section: None });
                        }
                    }
                    Value::String(text) => {
                        steps.extend(split_text_steps(text));
                    }
                    _ => {}
                }
            }
            steps
        }
        _ => Vec::new(),
    }
}

/// Extract the step text from a `HowToStep`-ish object (`text`, `name`, or
/// nested `itemListElement`).
fn how_to_step_text(value: &Value) -> Option<String> {
    match value {
        Value::Object(map) => {
            if let Some(text) = map.get("text").and_then(|t| value_to_string(Some(t))) {
                return Some(clean_text(&text));
            }
            if let Some(name) = map.get("name").and_then(|t| value_to_string(Some(t))) {
                // Some sites put a short title in `name` and nothing else.
                return Some(clean_text(&name));
            }
            // Single-item itemListElement wrapping.
            map.get("itemListElement")
                .and_then(|l| how_to_step_text(l))
        }
        Value::String(text) => Some(clean_text(text)),
        _ => None,
    }
    .filter(|t| !t.is_empty())
}

/// Split a free-text instruction blob into steps. Only splits on hard
/// boundaries (newlines, numbered "1." prefixes) — never mid-sentence.
fn split_text_steps(text: &str) -> Vec<InstructionStep> {
    let mut steps = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // "1. Preheat…" / "2) Mix…" — strip the leading marker.
        let body = strip_leading_number(line);
        if !body.is_empty() {
            steps.push(InstructionStep { text: body.to_string(), section: None });
        }
    }
    steps
}

/// Remove a leading "1.", "1)", "- ", "*" marker from a line.
fn strip_leading_number(line: &str) -> &str {
    let mut rest = line.trim_start();
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if !digits.is_empty() {
        let after = &rest[digits.len()..];
        let after = after.trim_start_matches(['.', ')']).trim_start();
        // Only treat it as a marker when something meaningful follows.
        if !after.is_empty() {
            rest = after;
            return rest;
        }
    }
    if let Some(stripped) = rest.strip_prefix("- ").or_else(|| rest.strip_prefix("* ")) {
        return stripped.trim();
    }
    rest.trim()
}

/// Collapse whitespace inside extracted text.
pub fn clean_text(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Ordered unique section names used by the given steps (first-seen order),
/// for `instruction_sections`.
pub fn instruction_sections(steps: &[InstructionStep]) -> Vec<String> {
    let mut sections: Vec<String> = Vec::new();
    for step in steps {
        if let Some(section) = step.section.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if !sections.iter().any(|s| s == section) {
                sections.push(section.to_string());
            }
        }
    }
    sections
}

/// Parse raw `recipeIngredient` strings into structured ingredients.
pub fn parse_schema_ingredients(raw: &[String]) -> Vec<Ingredient> {
    raw.iter()
        .map(|line| parse_ingredient_line(line))
        .filter(|ingredient| !ingredient.name.is_empty())
        .collect()
}

/// Ordered unique ingredient section names (first-seen order). The importer
/// itself does not group ingredients today — the hook exists so callers keep
/// a single source of truth for the section list.
pub fn ingredient_sections(ingredients: &[Ingredient]) -> Vec<String> {
    let mut sections: Vec<String> = Vec::new();
    for ingredient in ingredients {
        if let Some(section) = ingredient.section.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
            if !sections.iter().any(|s| s == section) {
                sections.push(section.to_string());
            }
        }
    }
    sections
}

// ---------------------------------------------------------------------------
// Ingredient line parsing
// ---------------------------------------------------------------------------

/// Common cooking units (case-insensitive), singular + plural, with and
/// without a trailing period (abbreviations like "tbsp.").
const UNITS: &[&str] = &[
    // metric weight
    "g", "gram", "grams", "kg", "kilogram", "kilograms",
    // metric volume
    "ml", "milliliter", "milliliters", "millilitre", "millilitres", "l", "liter",
    "liters", "litre", "litres", "dl", "cl",
    // imperial weight
    "oz", "ounce", "ounces", "lb", "lbs", "pound", "pounds",
    // us volume
    "tsp", "teaspoon", "teaspoons", "tbsp", "tablespoon", "tablespoons",
    "cup", "cups", "fl oz", "floz", "pint", "pints", "quart", "quarts",
    "gallon", "gallons", "qt", "pt",
    // lengths (butter sticks, sheets, rods…)
    "cm", "mm", "inch", "inches", "in",
    // colloquial
    "pinch", "pinches", "dash", "dashes", "sprig", "sprigs", "clove", "cloves",
    "can", "cans", "packet", "packets", "pack", "packs", "bunch", "bunches",
    "slice", "slices", "stick", "sticks", "sheet", "sheets", "leaf", "leaves",
    "splash", "splashes", "handful", "handfuls",
];

/// Unicode (and ASCII) vulgar fractions mapped to their numeric value.
const FRACTIONS: &[(char, f64)] = &[
    ('¼', 0.25),
    ('½', 0.5),
    ('¾', 0.75),
    ('⅐', 1.0 / 7.0),
    ('⅑', 1.0 / 9.0),
    ('⅒', 0.1),
    ('⅓', 1.0 / 3.0),
    ('⅔', 2.0 / 3.0),
    ('⅕', 0.2),
    ('⅖', 0.4),
    ('⅗', 0.6),
    ('⅘', 0.8),
    ('⅙', 1.0 / 6.0),
    ('⅚', 5.0 / 6.0),
    ('⅛', 0.125),
    ('⅜', 0.375),
    ('⅝', 0.625),
    ('⅞', 0.875),
];

/// Preparation phrases moved into `prep` (after the ingredient's comma).
const PREP_PHRASES: &[&str] = &[
    "to taste", "as needed", "or to taste", "divided", "at room temperature",
    "room temperature", "optional", "for garnish", "for serving", "plus more",
    "plus", "or more", "as desired", "lightly beaten", "beaten", "softened",
    "melted", "chilled", "finely chopped", "chopped", "minced", "diced",
    "sliced", "grated", "shredded", "peeled", "crushed", "julienned",
    "toasted", "roughly chopped", "coarsely chopped", "thinly sliced",
    "cut into chunks", "cubed", "torn", "rinsed", "drained", "pitted",
    "halved", "quartered", "zested", "juiced", "sifted", "cooled", "cooked",
    "uncooked", "raw", "fresh", "freshly ground", "ground", "unsalted",
    "salted", "extra-virgin", "extra virgin", "virgin", "cold", "hot", "warm",
    "large", "medium", "small", "ripe", "unripe", "packed", "heaping",
    "heaped", "level", "finely grated", "zest and juice", "from 1 lemon",
];

/// Parse one raw ingredient line into the structured model. When anything is
/// uncertain the whole (cleaned) line is kept as `name` — nothing invented.
pub fn parse_ingredient_line(line: &str) -> Ingredient {
    let line = clean_text(line);
    if line.is_empty() {
        return Ingredient {
            quantity: None,
            unit: None,
            name: String::new(),
            prep: None,
            section: None,
        };
    }

    // Split off the preparation suffix at the first comma (…, finely chopped).
    let (main, prep) = match line.split_once(',') {
        Some((main, rest)) => {
            let rest = rest.trim();
            if rest.split_whitespace().count() <= 4 && looks_like_prep(rest) {
                (main.trim(), Some(rest.to_string()))
            } else {
                // Long/odd comma content stays part of the name ("sauce, homemade, spicy").
                (line.as_str(), None)
            }
        }
        None => (line.as_str(), None),
    };

    let mut remainder = main.trim().to_string();
    let quantity = parse_leading_quantity(&mut remainder);
    // Eat the whitespace between the quantity and the unit ("1.5 l stock").
    let remainder_trimmed = remainder.trim_start().to_string();

    let unit = if let Some((u, rest)) = split_leading_unit(&remainder_trimmed) {
        remainder = rest.to_string();
        Some(u)
    } else {
        remainder = remainder_trimmed;
        None
    };

    let name = remainder.trim().trim_matches(|c: char| !c.is_alphanumeric()).to_string();
    let name = if name.is_empty() { main.trim().to_string() } else { name };

    Ingredient {
        quantity,
        unit,
        name,
        prep,
        section: None,
    }
}

/// Whether a comma-suffix looks like preparation rather than part of the name.
fn looks_like_prep(text: &str) -> bool {
    let lowered = text.to_lowercase();
    PREP_PHRASES.iter().any(|phrase| lowered.contains(phrase))
        || lowered.starts_with("for ")
        || lowered.starts_with("plus ")
        || lowered.starts_with("or ")
        || lowered.starts_with("to ")
}

/// Consume a leading quantity from `remainder` (mutating it). Supports:
/// unicode fractions, `1/2`, `1 1/2` mixed numbers, decimals (`1.5`, `1,5`),
/// integers, and ranges (`1-2`, `1 to 2`, `1½–2`). A range yields its lower
/// bound (the plan forbids inventing numbers; the upper bound is lost only
/// when the UI model cannot express it, which it cannot today).
fn parse_leading_quantity(remainder: &mut String) -> Option<f64> {
    let (value, rest) = scan_quantity(remainder.trim_start())?;
    *remainder = rest.to_string();
    Some(value)
}

/// Scan a quantity at the very start of `text`; return it plus the rest.
fn scan_quantity(text: &str) -> Option<(f64, &str)> {
    // Leading unicode fraction (¼ cup …): value stands alone.
    if let Some((value, rest)) = leading_unicode_fraction(text) {
        return Some((value, rest));
    }

    // Optional first number (integer or decimal, '.' or ',').
    let digits_len = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.' || *c == ',')
        .map(char::len_utf8)
        .sum::<usize>();
    let first = if digits_len > 0 {
        let token = &text[..digits_len];
        if let Some(value) = parse_number_token(token) {
            value
        } else {
            // Not a usable number (e.g. "1.2.3") — bail without consuming.
            return None;
        }
    } else {
        // "a pinch of salt" — the article counts as 1, but only when a unit
        // follows, so we do not invent quantities for "a nice wine".
        let lowered = text.to_lowercase();
        for article in ["a ", "an "] {
            if lowered.starts_with(article) {
                let rest = &text[article.len()..];
                if split_leading_unit(rest).is_some() {
                    return Some((1.0, rest));
                }
                return None;
            }
        }
        return None;
    };

    let after_first = &text[digits_len..];

    // Fraction attached directly to the number ("1/2"): the numerator is
    // `first` itself.
    if let Some(after_slash) = after_first.strip_prefix('/') {
        let denominator: String = after_slash
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let denominator_len = denominator.len();
        if let Ok(denominator) = denominator.parse::<f64>() {
            if denominator != 0.0 {
                let consumed = &after_slash[denominator_len..];
                return Some((first / denominator, consumed));
            }
        }
    }

    // ASCII fraction after a space (mixed number "1 1/2").
    let fraction_start = after_first
        .strip_prefix(' ')
        .unwrap_or(after_first);
    if fraction_start.starts_with(|c: char| c.is_ascii_digit()) {
        let numerator: String = fraction_start
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        if let Some(after_num) = fraction_start[numerator.len()..].strip_prefix('/') {
            let denominator: String = after_num
                .chars()
                .take_while(|c| c.is_ascii_digit())
                .collect();
            let denominator_len = denominator.len();
            if let (Ok(numerator), Ok(denominator)) = (
                numerator.parse::<f64>(),
                denominator.parse::<f64>(),
            ) {
                if denominator != 0.0 {
                    let consumed = &after_num[denominator_len..];
                    return Some((first + numerator / denominator, consumed));
                }
            }
        }
    }
    // Trailing unicode fraction glued to the integer: "1½".
    let trimmed = after_first.trim_start();
    if let Some((value, rest)) = leading_unicode_fraction(trimmed) {
        return Some((first + value, rest));
    }

    // Range: "1-2", "1 – 2", "1 to 2" → lower bound. The separators are
    // matched on the whitespace-trimmed remainder, so "to 2" works for
    // "1 to 2 onions". A following number is required, so "1 tomato" is
    // never mistaken for a range.
    for separator in ["-", "–", "—", "to "] {
        if let Some(after_sep) = trimmed.strip_prefix(separator) {
            let after_sep = after_sep.trim_start();
            let upper: String = after_sep
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '.')
                .collect();
            if parse_number_token(&upper).is_some() {
                let consumed = &after_sep[upper.len()..];
                return Some((first, consumed));
            }
        }
    }

    Some((first, after_first))
}

/// Unicode fraction at the very start of the text, plus the rest after it.
fn leading_unicode_fraction(text: &str) -> Option<(f64, &str)> {
    let first = text.chars().next()?;
    let value = FRACTIONS
        .iter()
        .find(|(fraction_char, _)| *fraction_char == first)
        .map(|(_, value)| *value)?;
    let rest = &text[first.len_utf8()..];
    Some((value, rest))
}

fn parse_number_token(token: &str) -> Option<f64> {
    // Exactly one separator ('.' or ','), at least one digit.
    let normalized = if token.contains(',') && !token.contains('.') {
        token.replace(',', ".")
    } else {
        token.to_string()
    };
    let separators = normalized.matches('.').count();
    if separators > 1 {
        return None;
    }
    if normalized.chars().all(|c| c == '.') {
        return None;
    }
    normalized.parse::<f64>().ok()
}

/// If `text` starts with a known unit (case-insensitive, optional trailing
/// '.'), return the canonical unit and the rest.
fn split_leading_unit(text: &str) -> Option<(String, &str)> {
    let lowered = text.to_lowercase();
    // Longest-first so "tablespoons" wins over "t".
    let mut sorted: Vec<&str> = UNITS.to_vec();
    sorted.sort_by_key(|u| std::cmp::Reverse(u.len()));
    for unit in sorted {
        if lowered.starts_with(unit) {
            let after = &text[unit.len()..];
            let after = after.strip_prefix('.').unwrap_or(after);
            // Must be followed by whitespace/end: "cups" yes, "cupcake" no.
            if after.is_empty() || after.starts_with(char::is_whitespace) {
                return Some((unit.to_string(), after));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_number_and_unit() {
        let i = parse_ingredient_line("200 g flour");
        assert_eq!(i.quantity, Some(200.0));
        assert_eq!(i.unit.as_deref(), Some("g"));
        assert_eq!(i.name, "flour");
        assert_eq!(i.prep, None);
    }

    #[test]
    fn unicode_fraction() {
        let i = parse_ingredient_line("½ cup milk");
        assert_eq!(i.quantity, Some(0.5));
        assert_eq!(i.unit.as_deref(), Some("cup"));
        assert_eq!(i.name, "milk");

        let i = parse_ingredient_line("¾ tsp salt");
        assert_eq!(i.quantity, Some(0.75));
        assert_eq!(i.unit.as_deref(), Some("tsp"));

        let i = parse_ingredient_line("¼ cup sugar");
        assert_eq!(i.quantity, Some(0.25));
    }

    #[test]
    fn ascii_and_mixed_fractions() {
        let i = parse_ingredient_line("1/2 cup butter");
        assert_eq!(i.quantity, Some(0.5));
        assert_eq!(i.name, "butter");

        let i = parse_ingredient_line("1 1/2 cups all-purpose flour");
        assert_eq!(i.quantity, Some(1.5));
        assert_eq!(i.unit.as_deref(), Some("cups"));
        assert_eq!(i.name, "all-purpose flour");

        let i = parse_ingredient_line("1½ cups sugar");
        assert_eq!(i.quantity, Some(1.5));
    }

    #[test]
    fn decimals_and_european_separators() {
        let i = parse_ingredient_line("1.5 cups water");
        assert_eq!(i.quantity, Some(1.5));
        let i = parse_ingredient_line("1,5 l stock");
        assert_eq!(i.quantity, Some(1.5));
        assert_eq!(i.unit.as_deref(), Some("l"));
    }

    #[test]
    fn ranges_take_the_lower_bound() {
        let i = parse_ingredient_line("2-3 tomatoes");
        assert_eq!(i.quantity, Some(2.0));
        assert_eq!(i.name, "tomatoes");

        let i = parse_ingredient_line("1 to 2 onions");
        assert_eq!(i.quantity, Some(1.0));
        assert_eq!(i.name, "onions");

        let i = parse_ingredient_line("1–2 tbsp oil");
        assert_eq!(i.quantity, Some(1.0));
        assert_eq!(i.unit.as_deref(), Some("tbsp"));
    }

    #[test]
    fn preparation_suffix_moves_to_prep() {
        let i = parse_ingredient_line("2 eggs, beaten");
        assert_eq!(i.quantity, Some(2.0));
        assert_eq!(i.name, "eggs");
        assert_eq!(i.prep.as_deref(), Some("beaten"));

        let i = parse_ingredient_line("1 onion, finely chopped");
        assert_eq!(i.name, "onion");
        assert_eq!(i.prep.as_deref(), Some("finely chopped"));

        let i = parse_ingredient_line("50 g cheese, grated");
        assert_eq!(i.quantity, Some(50.0));
        assert_eq!(i.prep.as_deref(), Some("grated"));
    }

    #[test]
    fn no_quantity_is_fine() {
        let i = parse_ingredient_line("salt, to taste");
        assert_eq!(i.quantity, None);
        assert_eq!(i.unit, None);
        assert_eq!(i.name, "salt");
        assert_eq!(i.prep.as_deref(), Some("to taste"));

        let i = parse_ingredient_line("Fresh cilantro leaves");
        assert_eq!(i.quantity, None);
        assert_eq!(i.name, "Fresh cilantro leaves");
    }

    #[test]
    fn unknown_unit_stays_in_the_name() {
        // "knob" is not a known unit — everything except the number stays.
        let i = parse_ingredient_line("1 knob of butter");
        assert_eq!(i.quantity, Some(1.0));
        assert!(i.name.to_lowercase().contains("butter"), "name: {}", i.name);
    }

    #[test]
    fn unit_prefix_is_not_matched_mid_word() {
        let i = parse_ingredient_line("2 cupcakes"); // must NOT parse unit "cup" from "cupcakes"
        assert_eq!(i.quantity, Some(2.0));
        assert_eq!(i.unit, None);
        assert_eq!(i.name, "cupcakes");
    }

    #[test]
    fn abbreviations_with_period() {
        let i = parse_ingredient_line("2 tbsp. olive oil");
        assert_eq!(i.quantity, Some(2.0));
        assert_eq!(i.unit.as_deref(), Some("tbsp"));
        assert_eq!(i.name, "olive oil");
    }

    #[test]
    fn a_pinch_counts_as_one() {
        let i = parse_ingredient_line("a pinch of salt");
        assert_eq!(i.quantity, Some(1.0));
        assert_eq!(i.unit.as_deref(), Some("pinch"));
        assert_eq!(i.name, "of salt"); // modest parser: leftover words stay in the name
    }

    #[test]
    fn instructions_plain_strings() {
        let raw: Value = serde_json::from_str(r#"["Preheat the oven.","Mix everything together."]"#).unwrap();
        let steps = parse_instructions(Some(&raw));
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].text, "Preheat the oven.");
        assert_eq!(steps[0].section, None);
    }

    #[test]
    fn instructions_single_blob_splits_on_newlines() {
        let raw: Value =
            serde_json::from_str(r#""1. Preheat the oven.\n2. Mix everything together.""#).unwrap();
        let steps = parse_instructions(Some(&raw));
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].text, "Preheat the oven.");
        assert_eq!(steps[1].text, "Mix everything together.");
    }

    #[test]
    fn instructions_how_to_steps() {
        let raw: Value = serde_json::from_str(
            r#"[{"@type":"HowToStep","text":"Preheat the oven."},
                {"@type":"HowToStep","name":"Mix"}]"#,
        )
        .unwrap();
        let steps = parse_instructions(Some(&raw));
        assert_eq!(steps.len(), 2);
        assert_eq!(steps[0].text, "Preheat the oven.");
        assert_eq!(steps[1].text, "Mix");
    }

    #[test]
    fn instructions_how_to_sections_preserve_grouping() {
        let raw: Value = serde_json::from_str(
            r#"[
                {"@type":"HowToSection","name":"For the sauce",
                 "itemListElement":[
                    {"@type":"HowToStep","text":"Simmer the tomatoes."},
                    {"@type":"HowToStep","text":"Season."}]},
                {"@type":"HowToStep","text":"Boil the pasta."}
            ]"#,
        )
        .unwrap();
        let steps = parse_instructions(Some(&raw));
        assert_eq!(steps.len(), 3);
        assert_eq!(steps[0].section.as_deref(), Some("For the sauce"));
        assert_eq!(steps[0].text, "Simmer the tomatoes.");
        assert_eq!(steps[1].section.as_deref(), Some("For the sauce"));
        assert_eq!(steps[2].section, None);

        let sections = instruction_sections(&steps);
        assert_eq!(sections, vec!["For the sauce"]);
    }

    #[test]
    fn instructions_none_and_odd_shapes() {
        assert!(parse_instructions(None).is_empty());
        let raw: Value = serde_json::from_str("42").unwrap();
        assert!(parse_instructions(Some(&raw)).is_empty());
    }

    #[test]
    fn strip_leading_number_variants() {
        assert_eq!(strip_leading_number("1. Preheat"), "Preheat");
        assert_eq!(strip_leading_number("2) Mix"), "Mix");
        assert_eq!(strip_leading_number("- chill"), "chill");
        assert_eq!(strip_leading_number("Step stays"), "Step stays");
        // A number with nothing after it is not a marker.
        assert_eq!(strip_leading_number("350"), "350");
    }
}
