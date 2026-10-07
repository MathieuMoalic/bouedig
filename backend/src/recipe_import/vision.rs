//! Recipe extraction from a photo via a vision model on OpenRouter.
//!
//! The client uploads a photo; we base64 it into an `image_url` content part
//! and ask a vision model for the recipe as strict JSON matching
//! `shared::RecipeInput`. The result is *not* trusted blindly: it flows
//! through the same preview pipeline as the URL importer, where the user
//! reviews and fixes it before saving.

use base64::Engine as _;

/// Hard cap on the uploaded photo (mirrors a phone camera JPEG; larger files
/// are rejected with 413 before any base64 work).
pub const MAX_IMAGE_BYTES: usize = 15 * 1024 * 1024;

/// Everything needed to reach the vision model. Absent from `AppState` when
/// no key or model is configured — image import then answers 503.
#[derive(Debug, Clone)]
pub struct VisionExtractor {
    pub http: reqwest::Client,
    /// Chat-completions URL; overridable so tests can point at a fixture.
    pub endpoint: String,
    pub api_key: String,
    /// OpenRouter model slug with vision capability (e.g. a Gemini Flash
    /// class model). No default: image import stays off until configured.
    pub model: String,
}

/// Failure kinds of a vision extraction, mapped to HTTP statuses by the
/// handler.
#[derive(Debug)]
pub enum VisionError {
    /// The model/API could not be reached or answered with an error.
    Upstream(String),
    /// The answer arrived but was not a usable recipe.
    BadResponse(String),
}

impl std::fmt::Display for VisionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VisionError::Upstream(msg) => {
                write!(f, "the vision model could not be reached: {msg}")
            }
            VisionError::BadResponse(msg) => {
                write!(f, "could not read a recipe from the photo: {msg}")
            }
        }
    }
}

impl std::error::Error for VisionError {}

impl VisionExtractor {
    /// Extractor from env-style knobs. `None` unless a key AND a model are
    /// configured (the model has no default — image import stays off until
    /// `BOUEDIG_VISION_MODEL` is set).
    pub fn from_parts(
        api_key: Option<String>,
        model: Option<String>,
        endpoint: Option<String>,
    ) -> Option<Self> {
        let api_key = api_key.filter(|k| !k.trim().is_empty())?;
        let model = model.filter(|m| !m.trim().is_empty())?;
        Some(Self {
            http: reqwest::Client::builder()
                // Vision models are slow: a photo routinely takes 15–60 s.
                .timeout(std::time::Duration::from_secs(90))
                .build()
                .unwrap_or_default(),
            endpoint: endpoint
                .filter(|e| !e.trim().is_empty())
                .unwrap_or_else(|| "https://openrouter.ai/api/v1/chat/completions".into()),
            api_key,
            model,
        })
    }

    /// Send the photo and parse the answer into a recipe. The returned
    /// warnings flag things worth a human review (unreadable regions the
    /// model itself calls out).
    pub async fn extract(
        &self,
        image: &[u8],
        mime: &str,
    ) -> Result<(shared::RecipeInput, Vec<String>), VisionError> {
        let data_uri = format!(
            "data:{mime};base64,{}",
            base64::engine::general_purpose::STANDARD.encode(image)
        );
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": SYSTEM_PROMPT},
                {"role": "user", "content": [
                    {"type": "text", "text": USER_PROMPT},
                    {"type": "image_url", "image_url": {"url": data_uri}},
                ]},
            ],
            "response_format": {"type": "json_object"},
            "temperature": 0,
            "max_tokens": 2000,
        });
        let response = self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
            .map_err(|err| VisionError::Upstream(err.to_string()))?;
        if !response.status().is_success() {
            let status = response.status();
            let detail = response.text().await.unwrap_or_default();
            tracing::warn!("vision model returned {status}: {detail}");
            return Err(VisionError::Upstream(format!("status {status}")));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|err| VisionError::BadResponse(err.to_string()))?;
        let content = value["choices"][0]["message"]["content"]
            .as_str()
            .ok_or_else(|| {
                VisionError::BadResponse("the model returned no answer".to_string())
            })?;
        Self::parse_recipe(content)
    }

    /// Strictly parse the model's answer into a recipe: a JSON object that
    /// deserializes into `RecipeInput` and carries the minimum a recipe
    /// needs (a name, ingredients, instructions).
    pub fn parse_recipe(content: &str) -> Result<(shared::RecipeInput, Vec<String>), VisionError> {
        let value: serde_json::Value = serde_json::from_str(content)
            .map_err(|err| VisionError::BadResponse(format!("answer was not JSON: {err}")))?;
        let mut recipe: shared::RecipeInput = serde_json::from_value(value)
            .map_err(|err| VisionError::BadResponse(format!("answer shape wrong: {err}")))?;
        recipe.name = recipe.name.trim().to_string();
        if recipe.name.is_empty() {
            return Err(VisionError::BadResponse(
                "the photo does not seem to contain a recipe (no name found)".to_string(),
            ));
        }
        if recipe.ingredients.is_empty() {
            return Err(VisionError::BadResponse(
                "no ingredients could be read from the photo".to_string(),
            ));
        }
        if recipe.instructions.is_empty() {
            return Err(VisionError::BadResponse(
                "no instructions could be read from the photo".to_string(),
            ));
        }
        // Conservative by house rules: surface what the model itself was
        // unsure about so the preview makes it visible.
        let warnings: Vec<String> = recipe
            .ingredients
            .iter()
            .filter(|i| i.quantity.is_none())
            .map(|i| {
                if i.name.len() > 40 {
                    format!("\"{}…\" has no quantity — please review", &i.name[..40])
                } else {
                    format!("\"{}\" has no quantity — please review", i.name)
                }
            })
            .collect();
        Ok((recipe, warnings))
    }
}

const SYSTEM_PROMPT: &str = "You are a recipe-extraction engine for a cooking app. \
    You read recipes from photos of cookbook pages, recipe cards, screenshots or \
    handwritten notes and answer with strict JSON only — no markdown, no commentary.";

const USER_PROMPT: &str = r#"Extract the recipe shown in this photo. Respond with only a JSON object with exactly these keys:

{"name": string, "sections": [string], "ingredients": [{"quantity": number | null, "unit": string | null, "name": string, "prep": string | null, "section": string | null}], "instructions": [{"text": string, "section": string | null}], "instruction_sections": [string], "notes": string, "yield": string}

Rules:
- "quantity" is a number when the photo shows one (0.5 for ½), otherwise null. Never guess a quantity.
- Convert imperial units to metric: volumes to ml, weights to g. Keep teaspoons (tsp) and tablespoons (tbsp) as-is. Use short unit names: g, kg, ml, l, tsp, tbsp, pinch, can, clove.
- "sections" lists the ingredient-group headings the photo shows; set each ingredient's "section" to its group, null when ungrouped.
- "instruction_sections" lists step-group headings; set each instruction's "section" likewise, null when ungrouped.
- Never invent ingredients or steps that are not visible. If a region is unreadable, extract the rest and append to "notes" like "unreadable: last two steps".
- "yield" is the servings/yield line, empty string when the photo shows none."#;

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD_ANSWER: &str = r#"{"name":"Pancakes","sections":[],"ingredients":[
        {"quantity":200,"unit":"g","name":"flour","prep":null,"section":null},
        {"quantity":null,"unit":null,"name":"salt","prep":"pinch","section":null}],
        "instructions":[{"text":"Whisk.","section":null}],
        "instruction_sections":[],"notes":"","yield":""}"#;

    #[test]
    fn parse_recipe_accepts_a_good_answer_and_warns_on_missing_quantities() {
        let (recipe, warnings) = VisionExtractor::parse_recipe(GOOD_ANSWER).unwrap();
        assert_eq!(recipe.name, "Pancakes");
        assert_eq!(recipe.ingredients.len(), 2);
        assert_eq!(warnings, vec!["\"salt\" has no quantity — please review"]);
    }

    #[test]
    fn parse_recipe_rejects_non_json_and_incomplete_recipes() {
        assert!(VisionExtractor::parse_recipe("sorry, I see no recipe").is_err());
        let no_name = GOOD_ANSWER.replace("\"Pancakes\"", "\"\"");
        assert!(VisionExtractor::parse_recipe(&no_name).is_err());
        let no_ingredients = GOOD_ANSWER.replace(
            &GOOD_ANSWER[GOOD_ANSWER.find("\"ingredients\"").unwrap()..GOOD_ANSWER.find("\"instructions\"").unwrap()],
            "\"ingredients\":[],",
        );
        assert!(VisionExtractor::parse_recipe(&no_ingredients).is_err());
    }

    #[test]
    fn parse_recipe_tolerates_extra_keys_and_missing_optionals() {
        let loose = r#"{"name":"Soup","ingredients":[{"name":"water","quantity":500,"unit":"ml"}],
            "instructions":[{"text":"Boil.","section":null}],"instructions_plaintext":"boil",
            "servings":"4"}"#;
        let (recipe, warnings) = VisionExtractor::parse_recipe(loose).unwrap();
        assert_eq!(recipe.ingredients[0].quantity, Some(500.0));
        assert!(warnings.is_empty());
    }

    #[tokio::test]
    async fn extract_reports_unreachable_endpoint_as_upstream() {
        let extractor =
            VisionExtractor::from_parts(Some("key".into()), Some("test-model".into()),
                Some("http://127.0.0.1:9/v1/chat/completions".into()))
            .unwrap();
        match extractor.extract(b"png", "image/png").await {
            Err(VisionError::Upstream(_)) => {}
            other => panic!("expected Upstream, got {other:?}"),
        }
    }
}
