//! Data models shared between the backend and the frontends.

use serde::{Deserialize, Serialize};

/// Default grocery category for items added without an explicit group.
pub const DEFAULT_CATEGORY: &str = "Groceries";

/// A recipe as shown in the grid (summary; no ingredients/instructions).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Recipe {
    pub id: i64,
    pub name: String,
    /// URL of the full-resolution photo, if one was uploaded.
    pub image: Option<String>,
    /// URL of the compressed thumbnail shown in the recipe grid.
    pub thumb: Option<String>,
}

/// One structured ingredient line of a recipe.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Ingredient {
    /// e.g. `180`; optional (some ingredients are "Salt to taste").
    pub quantity: Option<f64>,
    /// e.g. `g`, `ml`, `tbsp`.
    pub unit: Option<String>,
    /// e.g. `buckwheat flour`.
    pub name: String,
    /// Optional preparation note, e.g. `finely chopped`.
    pub prep: Option<String>,
    /// Section this ingredient belongs to (e.g. "Crêpes", "Filling").
    #[serde(default)]
    pub section: Option<String>,
}

/// The full recipe shown on the detail page.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct RecipeDetail {
    pub id: i64,
    pub name: String,
    /// Ordered ingredient section names (may include empty sections).
    pub sections: Vec<String>,
    pub ingredients: Vec<Ingredient>,
    /// Ordered instruction steps (may carry their section name).
    pub instructions: Vec<InstructionStep>,
    /// Ordered instruction section names (may include empty sections).
    pub instruction_sections: Vec<String>,
    /// Free-form notes shown on the detail page.
    #[serde(default)]
    pub notes: String,
    /// e.g. "2 portions" or "12 cookies".
    #[serde(default)]
    #[serde(rename = "yield")]
    pub yield_amount: String,
    /// Where the recipe comes from (book, URL, person…).
    #[serde(default)]
    pub source: String,
    /// URL of the full-resolution photo, if one was uploaded.
    pub image: Option<String>,
    /// URL of the compressed thumbnail shown in the recipe grid.
    pub thumb: Option<String>,
}

/// One instruction step, optionally grouped under a named section.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct InstructionStep {
    pub text: String,
    #[serde(default)]
    pub section: Option<String>,
}

/// Payload used to create or update a recipe (name + structured fields).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipeInput {
    pub name: String,
    #[serde(default)]
    pub sections: Vec<String>,
    #[serde(default)]
    pub ingredients: Vec<Ingredient>,
    #[serde(default)]
    pub instructions: Vec<InstructionStep>,
    #[serde(default)]
    pub instruction_sections: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    #[serde(rename = "yield")]
    pub yield_amount: String,
    #[serde(default)]
    pub source: String,
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
