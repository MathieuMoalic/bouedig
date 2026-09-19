//! Data models shared between the backend and the frontends.

use serde::{Deserialize, Serialize};

/// Default grocery category for items added without an explicit group.
pub const DEFAULT_CATEGORY: &str = "Groceries";

/// A recipe as stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Recipe {
    pub id: i64,
    pub name: String,
    /// Raw ingredient list as entered by the user.
    pub ingredients: String,
    /// URL of the full-resolution photo, if one was uploaded.
    pub image: Option<String>,
    /// URL of the compressed thumbnail shown in the recipe grid.
    pub thumb: Option<String>,
}

/// JSON payload used to create a recipe *without* a photo. The web/mobile
/// clients normally post `multipart/form-data` instead; this model is kept
/// for API consumers that don't upload images.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewRecipe {
    pub name: String,
    pub ingredients: String,
}

/// A grocery list item as stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct GroceryItem {
    pub id: i64,
    pub name: String,
    pub bought: bool,
    /// Group the item belongs to (collapsible header in the UI).
    pub category: String,
}

/// Payload used to add a grocery item manually.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewGroceryItem {
    pub name: String,
    #[serde(default)]
    pub category: Option<String>,
}

/// Payload used to flip the `bought` checkbox of a grocery item.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct GroceryUpdate {
    pub bought: bool,
}

/// Split a raw ingredient string into individual ingredient names.
///
/// Accepts entries separated by commas or newlines, trims whitespace and
/// drops empty entries. Used by the backend when inserting a recipe so its
/// ingredients land on the grocery list, and by the UI for a live preview.
pub fn parse_ingredients(raw: &str) -> Vec<String> {
    raw.split([',', '\n'])
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_commas_and_newlines() {
        assert_eq!(
            parse_ingredients("Flour, Milk\nEggs ,  Butter\n\n"),
            vec!["Flour", "Milk", "Eggs", "Butter"]
        );
        assert!(parse_ingredients("   ").is_empty());
    }
}
