//! Data models shared between the backend and the frontends.

use serde::{Deserialize, Serialize};

/// A recipe as stored in the database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Recipe {
    pub id: i64,
    pub name: String,
    /// Raw ingredient list as entered by the user.
    pub ingredients: String,
}

/// Payload used to create a new recipe.
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
}

/// Payload used to add a grocery item manually.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewGroceryItem {
    pub name: String,
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
