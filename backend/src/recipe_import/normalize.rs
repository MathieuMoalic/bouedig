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
    "litres", "dl", "cl",
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
///
/// Metric preference: when a line carries both systems ("1 lb (454g)
/// potatoes", "4 cups (945 mL) broth", "1 can (15-ounce/425g) beans") the
/// metric reading wins and the imperial text is dropped or converted.
pub fn parse_ingredient_line(line: &str) -> Ingredient {
    let line = normalize_parens(&clean_text(line));
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
            let rest = strip_outer_parens(rest.trim());
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
    let mut quantity = parse_leading_quantity(&mut remainder);
    // Eat the whitespace between the quantity and the unit ("1.5 l stock").
    let remainder_trimmed = remainder.trim_start().to_string();

    // A leading parenthetical with a digit ("(454g) sweet potatoes") must not
    // block unit detection: skip over it, keep it in the name.
    let (mut unit, remainder) = if remainder_trimmed.starts_with('(') {
        match leading_group(&remainder_trimmed) {
            Some((group, after_group)) if group_content_has_digit(group) => {
                let after_group_trim = after_group.trim_start();
                match split_leading_unit(after_group_trim) {
                    Some((u, rest)) => (Some(u), format!("{group} {}", rest.trim_start())),
                    None => (None, remainder_trimmed.clone()),
                }
            }
            _ => match split_leading_unit(&remainder_trimmed) {
                Some((u, rest)) => (Some(u), rest.to_string()),
                None => (None, remainder_trimmed.clone()),
            },
        }
    } else {
        match split_leading_unit(&remainder_trimmed) {
            Some((u, rest)) => (Some(u), rest.to_string()),
            None => (None, remainder_trimmed.clone()),
        }
    };

    let mut remainder = remainder.trim_start().to_string();
    // Metric preference (1): an imperial main unit with a metric alternate
    // group ("1 pound (454g) sweet potatoes") becomes 454 g — the group and
    // the imperial unit disappear.
    let imperial_main = unit.as_deref().is_some_and(is_imperial_unit) && quantity.is_some();
    if imperial_main && remainder.starts_with('(') {
        if let Some((group, after_group)) = leading_group(&remainder) {
            if let Some((alt_qty, alt_unit)) =
                parse_alt_metric_quantity(strip_outer_parens(group))
            {
                quantity = Some(alt_qty);
                unit = Some(alt_unit);
                remainder = after_group.trim_start().to_string();
            }
        }
    }

    // Metric preference (2): an imperial main unit with no alternate converts
    // directly ("2 lb potatoes" → 907 g). tsp/tbsp stay — even metric
    // cookbooks measure small volumes that way.
    if unit.as_deref().is_some_and(is_imperial_unit) {
        if let (Some(q), Some(u)) = (quantity, unit.clone()) {
            if let Some((metric_qty, metric_unit)) = to_metric(q, &u) {
                quantity = Some(metric_qty);
                unit = Some(metric_unit);
            }
        }
    }

    // Metric preference (3): imperial fragments inside a kept group
    // ("(15-ounce/425g)" → "(425g)", "(6 ounces)" → "(170g)") are stripped or
    // converted so stored names never show them.
    if let Some((group, _)) = leading_group(&remainder) {
        if group_content_has_digit(group) {
            let content = strip_outer_parens(group);
            let cleaned = strip_imperial_in_group(content);
            if cleaned != content {
                remainder = remainder.replacen(group, &format!("({cleaned})"), 1);
            }
        }
    }

    let mut name = strip_unbalanced_parens(remainder.trim());
    let mut prep = prep;
    // A short trailing parenthetical note ("oil (divided)", "noodles (*see
    // note)") is preparation information, not part of the name.
    if let Some((group, start)) = trailing_group(&name) {
        let content = strip_outer_parens(group);
        if prep.is_none() && content.split_whitespace().count() <= 4 {
            let cut = name[..start].trim_end().to_string();
            let note = content.to_string();
            name = cut;
            prep = Some(note);
        }
    }
    let name = name.trim_matches(|c: char| matches!(c, ',' | ';' | '/')).to_string();
    let name = if name.is_empty() { main.trim().to_string() } else { name };

    Ingredient {
        quantity,
        unit,
        name,
        prep: prep.map(|p| strip_unbalanced_parens(&p)),
        section: None,
    }
}

/// Normalize the parenthesis conventions recipe sites emit before parsing:
/// * doubled author-note wrappers `((…))` lose one layer
/// * WP Recipe Maker's comma style `(, finely diced)` becomes `, finely diced`
/// * stray spaces inside groups `( text )` are trimmed, double spaces collapse
///
/// Matching is stack-based, so nested groups (`(diced (see Note 1))`) and
/// unbalanced strays are handled without ever leaving a dangling paren.
pub fn normalize_parens(input: &str) -> String {
    let text = clean_text(input);
    if text.is_empty() {
        return text;
    }
    let chars: Vec<char> = text.chars().collect();

    // Match every paren with its partner; unmatched ones are dropped.
    let mut match_of = vec![None; chars.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        if *c == '(' {
            stack.push(i);
        } else if *c == ')' {
            if let Some(open) = stack.pop() {
                match_of[open] = Some(i);
            }
        }
    }
    let unmatched_opens: Vec<usize> = stack;

    // Recursively render the segment [from..to) (to exclusive, or end).
    fn render(
        chars: &[char],
        match_of: &[Option<usize>],
        unmatched: &[usize],
        from: usize,
        to: usize,
    ) -> String {
        let mut out = String::new();
        let mut i = from;
        while i < to {
            if unmatched.contains(&i) || chars[i] == ')' && !within_paired(i, match_of, from, to) {
                i += 1;
                continue;
            }
            if chars[i] == '(' {
                let close = match_of[i].map(|c| c as usize);
                let Some(close) = close else { i += 1; continue };
                if close >= to {
                    i += 1;
                    continue;
                }
                let content: String = render(chars, match_of, unmatched, i + 1, close);
                let trimmed = content.trim();
                // `((note))`: two adjacent opens mean the author doubled the
                // parens around a note — emit the content, dropping exactly
                // one layer. Non-adjacent nesting (`(diced (see Note 1))`)
                // is kept as-is.
                if chars.get(i + 1) == Some(&'(') {
                    out.push_str(trimmed);
                } else if let Some(rest) = trimmed.strip_prefix(',') {
                    // `(, finely diced)` — a comma separator wrapped in parens.
                    out.push(',');
                    let rest = rest.trim_start();
                    if !rest.is_empty() {
                        out.push(' ');
                        out.push_str(rest);
                    }
                } else {
                    out.push('(');
                    out.push_str(trimmed);
                    out.push(')');
                }
                i = close + 1;
            } else {
                out.push(chars[i]);
                i += 1;
            }
        }
        clean_text(&out)
    }

    let rendered = render(&chars, &match_of, &unmatched_opens, 0, chars.len());
    // A group left over right before a comma reads better merged into it:
    // handled downstream; here just normalize spaces around commas.
    rendered.replace(" ,", ",")
}

/// Whether the paren at `i` participates in a pair fully inside [from, to).
fn within_paired(i: usize, match_of: &[Option<usize>], from: usize, to: usize) -> bool {
    match match_of.get(i).copied().flatten() {
        Some(close) => close < to && i >= from,
        None => false,
    }
}

/// Does `text` consist of exactly one balanced group (plus surrounding spaces)?
fn is_single_group(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    if chars.first() != Some(&'(') || chars.last() != Some(&')') || chars.len() < 2 {
        return false;
    }
    let mut depth = 0i32;
    for (i, c) in chars.iter().enumerate() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                // The outer group must close exactly at the last char.
                if depth == 0 && i != chars.len() - 1 {
                    return false;
                }
                if depth < 0 {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}

/// If `text` contains a balanced group at its first '(', return the group and
/// everything after it.
fn leading_group(text: &str) -> Option<(&str, &str)> {
    let start = text.find('(')?;
    let rest = &text[start..];
    let mut depth = 0i32;
    for (byte_idx, c) in rest.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                if depth == 0 {
                    let end = byte_idx + c.len_utf8();
                    return Some((&rest[..end], &rest[end..]));
                }
            }
            _ => {}
        }
    }
    None
}

/// If `text` ends with a balanced single group (ignoring trailing spaces),
/// return the group text and the byte offset where it starts.
fn trailing_group(text: &str) -> Option<(&str, usize)> {
    let trimmed = text.trim_end();
    if !trimmed.ends_with(')') {
        return None;
    }
    let mut depth = 0i32;
    for (byte_idx, c) in trimmed.char_indices().rev() {
        match c {
            ')' => depth += 1,
            '(' => {
                depth -= 1;
                if depth == 0 {
                    let group = &trimmed[byte_idx..];
                    return if is_single_group(group) {
                        Some((group, byte_idx))
                    } else {
                        None
                    };
                }
                if depth < 0 {
                    return None;
                }
            }
            _ => {}
        }
    }
    None
}

fn group_content_has_digit(group: &str) -> bool {
    strip_outer_parens(group).chars().any(|c| c.is_ascii_digit())
}

/// Remove one outer paren pair when the text is wrapped in a single group.
fn strip_outer_parens(text: &str) -> &str {
    let trimmed = text.trim();
    if is_single_group(trimmed) {
        trimmed
            .strip_prefix('(')
            .and_then(|t| t.strip_suffix(')'))
            .unwrap_or(trimmed)
            .trim()
    } else {
        trimmed
    }
}

/// Drop paren characters that never find a partner, so no mangled fragment
/// (e.g. `454g)` or `(see note`) can ever reach the stored recipe.
fn strip_unbalanced_parens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut depth = 0i32;
    for c in text.chars() {
        match c {
            '(' => {
                depth += 1;
                out.push(c);
            }
            ')' => {
                if depth > 0 {
                    depth -= 1;
                    out.push(c);
                }
            }
            _ => out.push(c),
        }
    }
    // Any still-open groups: remove their dangling '(' characters.
    if depth > 0 {
        let mut result = String::with_capacity(out.len());
        let mut open = 0i32;
        for c in out.chars().rev() {
            match c {
                ')' => {
                    open += 1;
                    result.push(c);
                }
                '(' => {
                    if open > 0 {
                        open -= 1;
                    } else {
                        continue;
                    }
                    result.push(c);
                }
                _ => result.push(c),
            }
        }
        result.chars().rev().collect()
    } else {
        out
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
/// Range upper bounds (`½–1 tsp`, `1-2`, `¼–⅓`) are consumed so they never
/// leak into the ingredient name; the quantity is the lower bound.
fn scan_quantity(text: &str) -> Option<(f64, &str)> {
    let (value, rest) = scan_quantity_base(text)?;
    Some(consume_range(value, rest))
}

fn scan_quantity_base(text: &str) -> Option<(f64, &str)> {
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

    Some((first, after_first))
}

/// Eat a range separator plus upper bound (`-2`, `–1`, `–⅓`, `to 2`,
/// `–1/3`) after a quantity. Requires a number to follow, so "1-ounce can"
/// is untouched. A "fraction" upper bound smaller than the first value
/// ("2-1/2 cups") is the classic two-and-a-half shorthand, not a range —
/// that becomes a mixed number instead.
fn consume_range<'a>(value: f64, rest: &'a str) -> (f64, &'a str) {
    let trimmed = rest.trim_start();
    for separator in ["-", "–", "—", "to "] {
        if let Some(after_sep) = trimmed.strip_prefix(separator) {
            let after_sep = after_sep.trim_start();
            if let Some((upper_total, consumed_len)) = parse_upper_bound(after_sep) {
                if upper_total < value {
                    // "2-1/2" → two and a half, not a range.
                    return (value + upper_total, &after_sep[consumed_len..]);
                }
                return (value, &after_sep[consumed_len..]);
            }
            // Unicode upper bound ("¼–⅓ tsp").
            if let Some((_, upper_rest)) = leading_unicode_fraction(after_sep) {
                return (value, upper_rest);
            }
        }
    }
    (value, rest)
}

/// Parse an ASCII upper bound at the start of `text`: an integer/decimal,
/// optionally followed by a fraction (`1/3`). Returns its value and the
/// consumed byte length.
fn parse_upper_bound(text: &str) -> Option<(f64, usize)> {
    let digits: String = text
        .chars()
        .take_while(|c| c.is_ascii_digit() || *c == '.')
        .collect();
    if parse_number_token(&digits).is_none() {
        return None;
    }
    let mut consumed = digits.len();
    let mut total: f64 = digits.parse().unwrap_or(0.0);
    if let Some(after_slash) = text[consumed..].strip_prefix('/') {
        let denominator: String = after_slash
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect();
        let denominator_len = denominator.len();
        if let Ok(denominator) = denominator.parse::<f64>() {
            if denominator != 0.0 {
                if let Ok(numerator) = digits.parse::<f64>() {
                    total = numerator / denominator;
                }
                consumed += 1 + denominator_len;
            }
        }
    }
    Some((total, consumed))
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
                return Some((canonical_unit(unit), after));
            }
        }
    }
    None
}

/// Fold unit spelling variants onto one short canonical form
/// ("Tablespoons"/"tablespoon"/"tbsp." → "tbsp", "ounces" → "oz", …).
fn canonical_unit(unit: &str) -> String {
    match unit {
        "gram" | "grams" => "g",
        "kilogram" | "kilograms" => "kg",
        "milliliter" | "milliliters" | "millilitre" | "millilitres" => "ml",
        "liter" | "liters" | "litre" | "litres" => "l",
        "ounce" | "ounces" => "oz",
        "pound" | "pounds" | "lbs" => "lb",
        "teaspoon" | "teaspoons" => "tsp",
        "tablespoon" | "tablespoons" => "tbsp",
        "cups" => "cup",
        "pints" => "pint",
        "quarts" => "quart",
        "gallons" => "gallon",
        "inches" => "inch",
        "pinches" => "pinch",
        "dashes" => "dash",
        "sprigs" => "sprig",
        "cloves" => "clove",
        "cans" => "can",
        "packet" | "packets" | "packs" | "pack" => "pack",
        "bunch" | "bunches" => "bunch",
        "slices" => "slice",
        "sticks" => "stick",
        "sheets" => "sheet",
        "leaves" => "leaf",
        "splashes" => "splash",
        "handful" | "handfuls" => "handful",
        other => other,
    }
    .to_string()
}

/// Whether a raw line reads like it was meant to carry a quantity (a leading
/// number, fraction or "a/an <unit>"). Used to distinguish "salt, to taste"
/// (fine without a quantity) from "1.2.3 potatoes" (a failed parse).
/// Imperial units the metric preference applies to. tsp/tbsp deliberately
/// stay — small volumes are measured that way in metric kitchens too.
fn is_imperial_unit(unit: &str) -> bool {
    matches!(
        unit,
        "cup" | "lb" | "oz" | "pint" | "quart" | "gallon" | "fl oz" | "floz" | "inch"
    )
}

/// Convert an imperial quantity to metric (grams or millilitres), rounded
/// for display: whole numbers at 10+, one decimal below.
fn to_metric(quantity: f64, unit: &str) -> Option<(f64, String)> {
    let (converted, metric_unit): (f64, &str) = match canonical_unit(unit).as_str() {
        "lb" => (quantity * 453.592, "g"),
        "oz" => (quantity * 28.3495, "g"),
        "cup" => (quantity * 240.0, "ml"),
        "pint" => (quantity * 473.176, "ml"),
        "quart" => (quantity * 946.353, "ml"),
        "gallon" => (quantity * 3785.41, "ml"),
        "fl oz" | "floz" => (quantity * 29.5735, "ml"),
        "inch" => (quantity * 2.54, "cm"),
        _ => return None,
    };
    // Large millilitre volumes read better in litres (2880 ml → 2.9 l).
    let (value, metric_unit) = if metric_unit == "ml" && converted >= 1000.0 {
        (converted / 1000.0, "l")
    } else {
        (converted, metric_unit)
    };
    Some((round_metric(value), metric_unit.to_string()))
}

fn round_metric(value: f64) -> f64 {
    if value >= 20.0 {
        value.round()
    } else if value >= 1.0 {
        (value * 10.0).round() / 10.0
    } else {
        (value * 100.0).round() / 100.0
    }
}

/// Parse a group content that is entirely a metric alternate quantity —
/// "454g", "945 mL", "90-100g", "1 1/2 kg" — into (quantity, unit) with the
/// unit normalized to g/ml. Mixed or imperial content ("15-ounce/425g",
/// "6 ounces") does not qualify.
fn parse_alt_metric_quantity(content: &str) -> Option<(f64, String)> {
    let trimmed = content.trim();
    let lowered = trimmed.to_lowercase();
    // Longest suffix first so "kg" wins over "g".
    for unit in ["kg", "ml", "cl", "dl", "g", "l"] {
        let Some(prefix) = lowered.strip_suffix(unit) else {
            continue;
        };
        let prefix = prefix.trim_end();
        if prefix.is_empty() || !prefix.ends_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        // The whole prefix must be a quantity (number/fraction/range).
        let (value, rest) = scan_quantity(prefix)?;
        if !rest.trim().is_empty() {
            return None;
        }
        let (factor, canonical): (f64, &str) = match unit {
            "kg" => (1000.0, "g"),
            "cl" => (10.0, "ml"),
            "dl" => (100.0, "ml"),
            "l" => (1000.0, "ml"),
            _ => (1.0, unit),
        };
        return Some((round_metric(value * factor), canonical.to_string()));
    }
    None
}

/// Remove (or convert) imperial fragments from a parenthetical that stays in
/// the name: "15-ounce/425g" → "425g", "6 ounces" → "170g".
fn strip_imperial_in_group(content: &str) -> String {
    let imperial_word =
        |part: &str| matches!(part, p if p.to_lowercase().split(|c: char| !c.is_ascii_alphabetic()).any(|word| matches!(word, "oz" | "ounce" | "ounces" | "lb" | "lbs" | "pound" | "pounds" | "cup" | "cups" | "pint" | "quart" | "gallon" | "inch" | "floz" | "fl")));
    let parts: Vec<&str> = content.split('/').collect();
    let metric_parts: Vec<&str> = parts
        .iter()
        .copied()
        .filter(|part| !imperial_word(part))
        .collect();
    if metric_parts.len() < parts.len() && !metric_parts.is_empty() {
        // Some parts were imperial: keep the metric remainder.
        return metric_parts.join("/");
    }
    if parts.len() == 1 && imperial_word(content) {
        // Pure imperial ("6 ounces"): convert to metric grams.
        if let Some((value, rest)) = scan_quantity(content.trim()) {
            if rest.trim().is_empty() || rest.trim().len() <= 8 {
                if let Some((metric_qty, metric_unit)) =
                    to_metric(value, unit_word(rest))
                {
                    return format!("{}{}", round_metric(metric_qty), metric_unit);
                }
            }
        }
    }
    content.to_string()
}

/// The unit word inside a leftover fragment (" ounces" → "oz"…), if any.
fn unit_word(rest: &str) -> &str {
    let lowered = rest.trim().to_lowercase();
    for candidate in [
        "fl oz", "floz", "ounce", "ounces", "oz", "pound", "pounds", "lb",
        "lbs", "cup", "cups", "pint", "pints", "quart", "quarts", "gallon",
        "gallons", "inch", "inches",
    ] {
        if lowered.contains(candidate) {
            return candidate;
        }
    }
    ""
}

pub fn looks_quantified(line: &str) -> bool {
    let line = line.trim_start();
    let Some(first) = line.chars().next() else {
        return false;
    };
    if first.is_ascii_digit() || FRACTIONS.iter().any(|(c, _)| *c == first) {
        return true;
    }
    let lowered = line.to_lowercase();
    ["a ", "an "].iter().any(|article| {
        lowered.starts_with(article)
            && split_leading_unit(&line[article.len()..]).is_some()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn units_are_canonicalized() {
        // Spelled-out forms fold onto the short canonical unit. Imperial
        // units (cup/lb/oz) convert to metric instead — see
        // imperial_units_convert_to_metric below.
        for (line, expected) in [
            ("2 tablespoons olive oil", "tbsp"),
            ("2 tablespoon olive oil", "tbsp"),
            ("1 teaspoon salt", "tsp"),
            ("200 grams flour", "g"),
            ("500 milliliters milk", "ml"),
        ] {
            let parsed = parse_ingredient_line(line);
            assert_eq!(parsed.unit.as_deref(), Some(expected), "unit of {line:?}");
        }
    }

    #[test]
    fn plain_number_and_unit() {
        let i = parse_ingredient_line("200 g flour");
        assert_eq!(i.quantity, Some(200.0));
        assert_eq!(i.unit.as_deref(), Some("g"));
        assert_eq!(i.name, "flour");
        assert_eq!(i.prep, None);
    }

    #[test]
    #[test]
    fn unicode_fraction() {
        // ½ cup converts to 120 ml.
        let i = parse_ingredient_line("½ cup milk");
        assert_eq!(i.quantity, Some(120.0));
        assert_eq!(i.unit.as_deref(), Some("ml"));
        assert_eq!(i.name, "milk");

        let i = parse_ingredient_line("¾ tsp salt");
        assert_eq!(i.quantity, Some(0.75));
        assert_eq!(i.unit.as_deref(), Some("tsp"));

        let i = parse_ingredient_line("¼ cup sugar");
        assert_eq!(i.quantity, Some(60.0));
        assert_eq!(i.unit.as_deref(), Some("ml"));
    }

    #[test]
    fn ascii_and_mixed_fractions() {
        let i = parse_ingredient_line("1/2 cup butter");
        assert_eq!(i.quantity, Some(120.0));
        assert_eq!(i.unit.as_deref(), Some("ml"));
        assert_eq!(i.name, "butter");

        let i = parse_ingredient_line("1 1/2 cups all-purpose flour");
        assert_eq!(i.quantity, Some(360.0));
        assert_eq!(i.unit.as_deref(), Some("ml"));
        assert_eq!(i.name, "all-purpose flour");

        let i = parse_ingredient_line("1½ cups sugar");
        assert_eq!(i.quantity, Some(360.0));
        assert_eq!(i.unit.as_deref(), Some("ml"));
    }

    #[test]
    fn decimals_and_european_separators() {
        let i = parse_ingredient_line("1.5 cups water");
        assert_eq!(i.quantity, Some(360.0));
        assert_eq!(i.unit.as_deref(), Some("ml"));
        let i = parse_ingredient_line("1,5 l stock");
        // Litres are already metric: no conversion.
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

    // -------------------------------------------------------------------
    // Real-world ingredient lines captured verbatim from recipe sites
    // (WP Recipe Maker / Mediavine Create JSON-LD). These lock in the
    // paren/range/entity handling against regressions.
    // -------------------------------------------------------------------

    /// Assert name and prep have balanced parens (the core regression check
    /// shared with the live suite).
    fn assert_balanced(ingredient: &Ingredient) {
        let balance = |text: &str| {
            let mut depth: i32 = 0;
            for c in text.chars() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                assert!(depth >= 0, "unbalanced ')' in {text:?}");
            }
            assert_eq!(depth, 0, "unbalanced '(' in {text:?}");
        };
        balance(&ingredient.name);
        if let Some(prep) = &ingredient.prep {
            balance(prep);
        }
    }

    #[test]
    fn doubled_note_wrapper_from_mediavine_create() {
        // minimalistbaker.com, verbatim.
        let i = parse_ingredient_line("1 small green chili ((optional // I used a serrano pepper // omit for less heat))");
        assert_eq!(i.quantity, Some(1.0));
        assert_eq!(i.unit, None);
        assert_eq!(
            i.name,
            "small green chili (optional // I used a serrano pepper // omit for less heat)"
        );
        assert_balanced(&i);
    }

    #[test]
    fn doubled_wrapper_becomes_prep() {
        // minimalistbaker.com, verbatim.
        let i = parse_ingredient_line("2 1/4 cups light coconut milk* ((canned is best))");
        assert_eq!(i.quantity, Some(540.0));
        assert_eq!(i.unit.as_deref(), Some("ml"));
        assert_eq!(i.name, "light coconut milk*");
        assert_eq!(i.prep.as_deref(), Some("canned is best"));
        assert_balanced(&i);
    }

    #[test]
    fn metric_parens_stay_balanced_in_name() {
        // rainbowplantlife.com, verbatim: imperial main + metric alternate —
        // the metric value wins and the imperial text disappears.
        let i = parse_ingredient_line("1 pound (454g) sweet potatoes, (peeled and finely diced (see Note 3) )");
        assert_eq!(i.quantity, Some(454.0));
        assert_eq!(i.unit.as_deref(), Some("g"));
        assert_eq!(
            i.name,
            "sweet potatoes, (peeled and finely diced (see Note 3))"
        );
        assert_eq!(i.prep, None, "long note stays in the name, conservatively");
        assert_balanced(&i);
    }

    #[test]
    fn metric_paren_before_name_with_unit_after() {
        // rainbowplantlife.com, verbatim: "1 (15-ounce/425g) can cannellini
        // beans, (drained and rinsed)". The can count stays; the imperial
        // fragment inside the group is stripped.
        let i = parse_ingredient_line("1 (15-ounce/425g) can cannellini beans, (drained and rinsed)");
        assert_eq!(i.quantity, Some(1.0));
        assert_eq!(i.unit.as_deref(), Some("can"));
        assert_eq!(i.name, "(425g) cannellini beans");
        assert_eq!(i.prep.as_deref(), Some("drained and rinsed"));
        assert_balanced(&i);
    }
    #[test]
    fn unicode_fraction_quantity_with_metric_paren() {
        // rainbowplantlife.com, verbatim: an exact metric alternate wins over
        // converting the imperial primary (128 g is the author's precise
        // value).
        let i = parse_ingredient_line("½ cup (128g) creamy peanut butter ((no sugar added) )");
        assert_eq!(i.quantity, Some(128.0));
        assert_eq!(i.unit.as_deref(), Some("g"));
        assert_eq!(i.name, "creamy peanut butter");
        assert_eq!(i.prep.as_deref(), Some("no sugar added"));
        assert_balanced(&i);
    }

    #[test]
    fn wprm_comma_inside_parens_becomes_prep() {
        // veganhuggs.com, verbatim: WP Recipe Maker writes "(, finely diced)".
        let i = parse_ingredient_line("1 large onion (, finely diced)");
        assert_eq!(i.quantity, Some(1.0));
        assert_eq!(i.name, "large onion");
        assert_eq!(i.prep.as_deref(), Some("finely diced"));
        assert_balanced(&i);
    }

    #[test]
    fn doubled_parens_with_nested_groups_stay_balanced() {
        // veganhuggs.com, verbatim: 12 cups → 2.9 l.
        let i = parse_ingredient_line("12 cups fresh spinach ((loosely packed) rough chopped (about 14 oz))");
        assert_eq!(i.quantity, Some(2.9));
        assert_eq!(i.unit.as_deref(), Some("l"));
        assert_eq!(i.name, "fresh spinach (loosely packed) rough chopped");
        assert_eq!(i.prep.as_deref(), Some("about 14 oz"));
        assert_balanced(&i);
    }

    #[test]
    fn wprm_see_note_wrapper() {
        // veganhuggs.com, verbatim.
        let i = parse_ingredient_line("15 lasagna noodles ((*see note))");
        assert_eq!(i.quantity, Some(15.0));
        assert_eq!(i.name, "lasagna noodles");
        assert_eq!(i.prep.as_deref(), Some("*see note"));
        assert_balanced(&i);
    }

    #[test]
    fn jar_size_stays_in_name() {
        // veganhuggs.com, verbatim.
        let i = parse_ingredient_line("2 25 ounce jars of marinara sauce (, a thicker variety (*see note))");
        assert_eq!(i.quantity, Some(2.0));
        assert_eq!(
            i.name,
            "25 ounce jars of marinara sauce, a thicker variety"
        );
        assert_eq!(i.prep.as_deref(), Some("*see note"));
        assert_balanced(&i);
    }

    #[test]
    fn unicode_range_consumes_upper_bound() {
        // veganricha.com: "½–1 tsp" must not leave "1 tsp" in the name.
        let i = parse_ingredient_line("½–1 tsp ground cumin");
        assert_eq!(i.quantity, Some(0.5));
        assert_eq!(i.unit.as_deref(), Some("tsp"));
        assert_eq!(i.name, "ground cumin");
    }

    #[test]
    fn unicode_range_with_fraction_upper_bound() {
        // veganricha.com: "¼–⅓ tsp".
        let i = parse_ingredient_line("¼–⅓ tsp cayenne (or Indian red chili powder)");
        assert_eq!(i.quantity, Some(0.25));
        assert_eq!(i.unit.as_deref(), Some("tsp"));
        assert_eq!(i.name, "cayenne (or Indian red chili powder)");
        assert_balanced(&i);
    }

    #[test]
    fn ascii_fraction_upper_bound_is_fully_consumed() {
        // The live veganricha.com line uses ASCII "1/3": consuming only the
        // "1" used to leave "3 tsp cayenne" in the name.
        let i = parse_ingredient_line("¼–1/3  tsp cayenne (or Indian red chili powder)");
        assert_eq!(i.quantity, Some(0.25));
        assert_eq!(i.unit.as_deref(), Some("tsp"));
        assert_eq!(i.name, "cayenne (or Indian red chili powder)");
        assert_balanced(&i);
    }

    #[test]
    fn dash_fraction_shorthand_is_a_mixed_number() {
        // "2-1/2 cups" conventionally means two and a half cups (2.5 → 600 ml).
        let i = parse_ingredient_line("2-1/2 cups flour");
        assert_eq!(i.quantity, Some(600.0));
        assert_eq!(i.unit.as_deref(), Some("ml"));
        assert_eq!(i.name, "flour");
    }

    #[test]
    fn trailing_note_moves_to_prep() {
        // veganricha.com: "2 tsp oil (divided)".
        let i = parse_ingredient_line("2 tsp oil (divided)");
        assert_eq!(i.quantity, Some(2.0));
        assert_eq!(i.unit.as_deref(), Some("tsp"));
        assert_eq!(i.name, "oil");
        assert_eq!(i.prep.as_deref(), Some("divided"));
    }

    #[test]
    fn can_size_parenthetical_after_unit() {
        // forksoverknives.com, verbatim: pure-imperial group converts.
        let i = parse_ingredient_line("1 can (6 ounces) tomato paste");
        assert_eq!(i.quantity, Some(1.0));
        assert_eq!(i.unit.as_deref(), Some("can"));
        assert_eq!(i.name, "(170g) tomato paste");
        assert_balanced(&i);
    }

    #[test]
    #[test]
    fn imperial_units_convert_to_metric() {
        let i = parse_ingredient_line("2 lb potatoes");
        assert_eq!(i.quantity, Some(907.0));
        assert_eq!(i.unit.as_deref(), Some("g"));
        assert_eq!(i.name, "potatoes");

        let i = parse_ingredient_line("4 cups water");
        assert_eq!(i.quantity, Some(960.0));
        assert_eq!(i.unit.as_deref(), Some("ml"));

        // tsp/tbsp stay — small volumes are measured that way everywhere.
        let i = parse_ingredient_line("2 tbsp olive oil");
        assert_eq!(i.unit.as_deref(), Some("tbsp"));
        let i = parse_ingredient_line("1 tsp salt");
        assert_eq!(i.unit.as_deref(), Some("tsp"));
    }



    #[test]
    fn unbalanced_strays_are_dropped() {
        // Safety net: any leftover fragment can never leak into the recipe.
        let i = parse_ingredient_line("2 cups 454g) chopped tomatoes (see note");
        assert_balanced(&i);
        assert!(!i.name.contains(')') || i.name.contains('('));
        assert!(!i.name.ends_with('('));
    }

    #[test]
    fn normalize_parens_variants() {
        assert_eq!(
            normalize_parens("oil ((use refined))"),
            "oil (use refined)"
        );
        assert_eq!(
            normalize_parens("onion (, finely diced)"),
            "onion, finely diced"
        );
        assert_eq!(
            normalize_parens("salt ( to taste )"),
            "salt (to taste)"
        );
        // Legit nesting survives.
        assert_eq!(
            normalize_parens("peppers, (diced (see Note 1) )"),
            "peppers, (diced (see Note 1))"
        );
        // Doubled wrapper around multiple groups keeps one layer.
        assert_eq!(
            normalize_parens("spinach ((loosely packed) rough chopped)"),
            "spinach (loosely packed) rough chopped"
        );
    }
}
