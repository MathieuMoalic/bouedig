//! Data models shared between the backend and the frontends.

use serde::{Deserialize, Serialize};

/// Default grocery category for items added without an explicit group —
/// also the landing spot for items the auto-classifier can't place.
pub const DEFAULT_CATEGORY: &str = "Other";

/// Preset group suggestions offered in the grocery category dropdowns.
pub const GROCERY_CATEGORIES: &[&str] = &[
    "Other",
    "Fruits",
    "Vegetables",
    "Bakery",
    "Vegan",
    "Drinks",
    "Alcohol",
    "Seasoning",
    "Canned",
    "Pantry",
    "Non-Food",
    "Pharmacy",
    "Online",
    "Online Alcohol",
];

/// The categories the auto-classifier may assign. The manual-only groups
/// (Non-Food, Pharmacy, Online, Online Alcohol) are excluded — those only
/// make sense when a person is adding the item themselves.
pub const CLASSIFIER_CATEGORIES: &[&str] = &[
    "Other",
    "Fruits",
    "Vegetables",
    "Bakery",
    "Vegan",
    "Drinks",
    "Alcohol",
    "Seasoning",
    "Canned",
    "Pantry",
];

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
    /// Recipe the item was added from, if any (cleared when the recipe is
    /// deleted; `None` for manually added items).
    #[serde(default)]
    pub recipe: Option<Recipe>,
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

/// Payload for renaming / regrouping a grocery item.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroceryPatch {
    pub name: String,
    /// `None` or blank resets the item to the default group.
    #[serde(default)]
    pub category: Option<String>,
}

/// Payload for adding several grocery items at once (e.g. the ingredients
/// picked from a recipe's "add to shopping list" sheet). Every item in the
/// batch is stamped with `recipe_id` as its provenance.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewGroceryBatch {
    pub items: Vec<NewGroceryItem>,
    #[serde(default)]
    pub recipe_id: Option<i64>,
}

/// One recipe scheduled on one day of the meal plan. The recipe summary is
/// embedded so clients need no second request to render the plan.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MealPlanEntry {
    pub id: i64,
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    pub recipe: Recipe,
}

/// Payload used to add a recipe to a day of the meal plan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewMealPlanEntry {
    /// Local calendar date, `YYYY-MM-DD`.
    pub date: String,
    pub recipe_id: i64,
}
