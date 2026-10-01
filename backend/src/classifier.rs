//! Grocery-category classification via Jev on OpenRouter.
//!
//! The app is cache-first: `ingredient_categories` remembers every confident
//! answer, and only cache misses reach the API — from a background task, so
//! adds never wait on the network. Jev answers typed questions; over
//! OpenRouter's chat interface (model slug `typesafe/jev-router`) we ask one
//! strict JSON Choice question per ingredient and trust the answer only at
//! confidence ≥ 0.5.

use shared::CLASSIFIER_CATEGORIES;

/// Minimum confidence for an API answer to be applied (and cached).
pub const MIN_CONFIDENCE: f64 = 0.5;

/// Everything needed to reach the classifier. Absent from `AppState` when
/// no key is configured — the app then simply keeps everything in "Other".
#[derive(Debug, Clone)]
pub struct Classifier {
    pub http: reqwest::Client,
    /// Chat-completions URL; overridable so tests can point at a fixture.
    pub endpoint: String,
    pub api_key: String,
    /// OpenRouter model slug, e.g. `typesafe-ai/jev`.
    pub model: String,
}

impl Classifier {
    /// Classifier from env-style knobs (`None` when unconfigured).
    pub fn from_parts(
        api_key: Option<String>,
        model: Option<String>,
        endpoint: Option<String>,
    ) -> Option<Self> {
        let api_key = api_key.filter(|k| !k.trim().is_empty())?;
        Some(Self {
            http: reqwest::Client::builder()
                // Generous: classification runs in the background, and the
                // router's first calls after idle can be slow.
                .timeout(std::time::Duration::from_secs(8))
                .build()
                .unwrap_or_default(),
            endpoint: endpoint
                .filter(|e| !e.trim().is_empty())
                .unwrap_or_else(|| "https://openrouter.ai/api/v1/chat/completions".into()),
            api_key,
            model: model
                .filter(|m| !m.trim().is_empty())
                .unwrap_or_else(|| "typesafe/jev-router".into()),
        })
    }

    /// Ask the model which category `name` belongs to. `None` means "keep
    /// Other" — network failure, unparseable answer, low confidence, or an
    /// answer outside the allowed set.
    pub async fn classify(&self, name: &str) -> Option<String> {
        let categories = CLASSIFIER_CATEGORIES.join(", ");
        let system = "You categorize grocery items for a shopping list. \
            The household is strictly plant-based (vegan): plain words like \
            \"milk\", \"sausages\", \"cheese\" or \"butter\" mean the vegan \
            version and belong in Vegan. Answer only with JSON.";
        let user = format!(
            "Ingredient: {name:?}\n\
             Choose exactly one category from [{categories}].\n\
             Respond with JSON: {{\"category\": \"...\", \"confidence\": 0.0}} \
             where confidence is your probability that the category is right."
        );
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": user},
            ],
            "response_format": {"type": "json_object"},
            "temperature": 0,
            "max_tokens": 60,
        });
        let response = match self
            .http
            .post(&self.endpoint)
            .bearer_auth(&self.api_key)
            .json(&body)
            .send()
            .await
        {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!("classifier request for {name:?} failed: {err}");
                return None;
            }
        };
        if !response.status().is_success() {
            tracing::warn!("classifier returned {} for {name:?}", response.status());
            return None;
        }
        // OpenRouter shape: choices[0].message.content is the JSON answer.
        let value: serde_json::Value = response.json().await.ok()?;
        let content = value["choices"][0]["message"]["content"].as_str()?;
        Self::parse_answer(content)
    }

    /// Strictly validate a raw answer: JSON object, category from the
    /// allowed set, confidence at or above the threshold.
    pub fn parse_answer(content: &str) -> Option<String> {
        let value: serde_json::Value = serde_json::from_str(content).ok()?;
        let category = value["category"].as_str()?.trim();
        let confidence = value["confidence"].as_f64().unwrap_or(0.0);
        if confidence < MIN_CONFIDENCE {
            return None;
        }
        CLASSIFIER_CATEGORIES
            .iter()
            .find(|c| c.eq_ignore_ascii_case(category))
            .map(|c| c.to_string())
    }
}

/// Normalized cache key for an ingredient name (what the add handlers and
/// the background task both use).
pub fn cache_key(name: &str) -> String {
    name.trim().to_lowercase()
}

/// Remember a confident classification for future adds.
pub async fn remember(db: &sqlx::SqlitePool, name: &str, category: &str) {
    let _ = sqlx::query(
        "INSERT INTO ingredient_categories (name, category) VALUES (?, ?) \
         ON CONFLICT(name) DO UPDATE SET category = excluded.category",
    )
    .bind(cache_key(name))
    .bind(category)
    .execute(db)
    .await;
}

/// Classify the just-inserted `(item_id, name)` pairs, move their rows out
/// of "Other" when confident, and remember the answers. Runs in the
/// background after the add response has been sent — failures are logged
/// and the items simply stay in "Other" (uncached, so a later add retries).
/// Distinct names are classified concurrently.
pub async fn classify_pending(
    db: sqlx::SqlitePool,
    classifier: Classifier,
    pending: Vec<(i64, String)>,
) {
    // One API call per distinct name; every row of that name moves together.
    let mut by_name: std::collections::HashMap<String, Vec<i64>> =
        std::collections::HashMap::new();
    for (id, name) in pending {
        by_name.entry(cache_key(&name)).or_default().push(id);
    }
    let mut tasks = Vec::new();
    for (key, ids) in by_name {
        let db = db.clone();
        let classifier = classifier.clone();
        tasks.push(tokio::spawn(async move {
            let Some(category) = classifier.classify(&key).await else {
                tracing::debug!("no confident category for {key:?}; leaving in Other");
                return;
            };
            for id in ids {
                let _ = sqlx::query("UPDATE grocery_items SET category = ? WHERE id = ?")
                    .bind(&category)
                    .bind(id)
                    .execute(&db)
                    .await;
            }
            remember(&db, &key, &category).await;
        }));
    }
    for task in tasks {
        let _ = task.await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_answer_accepts_allowed_category_at_high_confidence() {
        let answer = Classifier::parse_answer(r#"{"category":"Vegan","confidence":0.9}"#);
        assert_eq!(answer.as_deref(), Some("Vegan"));
    }

    #[test]
    fn parse_answer_is_case_insensitive_and_trims() {
        let answer = Classifier::parse_answer(r#"{"category":" vegan ","confidence":0.6}"#);
        assert_eq!(answer.as_deref(), Some("Vegan"));
    }

    #[test]
    fn parse_answer_rejects_low_confidence_and_stray_categories() {
        assert_eq!(Classifier::parse_answer(r#"{"category":"Pantry","confidence":0.49}"#), None);
        assert_eq!(
            Classifier::parse_answer(r#"{"category":"Non-Food","confidence":0.99}"#),
            None,
            "manual-only categories must never be auto-assigned"
        );
        assert_eq!(Classifier::parse_answer("not json"), None);
    }

    #[test]
    fn cache_key_normalizes_case_and_whitespace() {
        assert_eq!(cache_key("  Brown-Green Lentils "), "brown-green lentils");
    }

    #[tokio::test]
    async fn classify_handles_unreachable_endpoint_as_none() {
        let classifier = Classifier::from_parts(
            Some("key".into()),
            Some("typesafe-ai/jev".into()),
            Some("http://127.0.0.1:9/v1/chat/completions".into()),
        )
        .unwrap();
        assert_eq!(classifier.classify("onion").await, None);
    }
}
