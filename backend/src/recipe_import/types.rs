//! Intermediate types of the recipe importer.
//!
//! External Schema.org / webpage structures are intentionally *not* part of
//! `shared`: they are shape-tolerant intermediates that only exist inside the
//! importer and are normalized into `shared::RecipeInput` at the end.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use url::Url;

/// A page downloaded by the fetcher.
#[derive(Debug, Clone)]
pub struct FetchedPage {
    /// URL after following redirects (used as the recipe source).
    pub final_url: Url,
    pub html: String,
}

/// Which extraction strategy produced the recipe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtractionMethod {
    JsonLd,
    Html,
}

/// Shape-tolerant representation of one Schema.org `Recipe` object.
///
/// Fields that can legitimately appear as string / array / nested object in
/// the wild are pre-flattened to `Option<String>` here; `instructions_raw`
/// keeps the raw JSON so the HowToStep / HowToSection normalization can run
/// on the original structure.
#[derive(Debug, Clone, Default)]
pub struct SchemaRecipe {
    pub name: Option<String>,
    pub description: Option<String>,
    /// Kept for a later phase (photo import); not yet consumed.
    #[allow(dead_code)]
    pub image: Option<String>,
    pub author: Option<String>,
    pub ingredients: Vec<String>,
    /// Raw `recipeInstructions` value (string, array of strings / HowToSteps
    /// / HowToSections, …).
    pub instructions_raw: Option<Value>,
    pub recipe_yield: Option<String>,
    pub prep_time: Option<String>,
    pub cook_time: Option<String>,
    pub total_time: Option<String>,
    /// Kept for a later phase (nutrition panel); not yet consumed.
    #[allow(dead_code)]
    pub nutrition: Option<Value>,
    /// Canonical source URL — filled in by the pipeline from the fetch.
    #[allow(dead_code)]
    pub source_url: Option<Url>,
}

impl SchemaRecipe {
    /// Rough "how much useful data did this object carry" count, used to
    /// prefer the best candidate when a page embeds several Recipe objects.
    pub fn completeness(&self) -> u32 {
        let mut score = 0;
        if self.name.is_some() {
            score += 2;
        }
        if !self.ingredients.is_empty() {
            score += 3;
        }
        if self.instructions_raw.is_some() {
            score += 3;
        }
        if self.recipe_yield.is_some() {
            score += 1;
        }
        if self.description.is_some() {
            score += 1;
        }
        if self.image.is_some() {
            score += 1;
        }
        score
    }

    pub fn has_ingredients_and_instructions(&self) -> bool {
        !self.ingredients.is_empty() && self.instructions_raw.is_some()
    }
}

/// Importer-specific failure kinds, mapped to HTTP statuses by the handler.
#[derive(Debug)]
pub enum ImportError {
    /// The request body URL could not be parsed / used.
    InvalidUrl(String),
    /// The URL points at a local, private or link-local network target.
    DisallowedTarget(String),
    /// The page could not be downloaded (network, status, size, redirects).
    FetchFailed(String),
    /// The page was fetched but no recipe could be recovered from it.
    ExtractionFailed(String),
}

impl std::fmt::Display for ImportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ImportError::InvalidUrl(msg) => write!(f, "invalid recipe URL: {msg}"),
            ImportError::DisallowedTarget(msg) => {
                write!(f, "recipe URL is not allowed: {msg}")
            }
            ImportError::FetchFailed(msg) => write!(f, "failed to fetch recipe page: {msg}"),
            ImportError::ExtractionFailed(msg) => {
                write!(f, "could not extract a recipe from the page: {msg}")
            }
        }
    }
}

impl std::error::Error for ImportError {}

/// The response of `POST /api/recipes/import`: a normalized recipe plus
/// provenance. Nothing is persisted by the import endpoint itself.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecipePreview {
    pub recipe: shared::RecipeInput,
    pub method: ExtractionMethod,
    pub confidence: f32,
    pub warnings: Vec<String>,
}
