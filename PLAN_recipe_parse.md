# Recipe Web Importer for Bouedig

## Goal

Add a robust webpage-to-recipe importer to Bouedig.

The importer should accept a recipe webpage URL, fetch the page, extract a structured recipe, normalize it into Bouedig's existing `RecipeInput` model, and return a preview that the client can review before saving.

Do not build a new microservice or redesign the existing recipe domain.

The existing shared model is already the target representation:

* `RecipeInput`
* `Ingredient`
* `InstructionStep`
* ingredient `section`
* instruction `section`
* `yield`
* `notes`
* `source`

The existing `POST /api/recipes` endpoint remains responsible for persistence and validation. The importer should initially be a separate extraction/preview path.

## Existing architecture to preserve

Bouedig is a Cargo workspace containing:

* `shared`
* `backend`
* `frontend-web`
* `frontend-android`
* `e2e-tests`

The backend is Axum + SQLite and currently has most backend logic in `backend/src/lib.rs`. Do not refactor the whole backend as part of this feature.

Add a focused `recipe_import` module and keep the router integration small.

The backend currently exposes:

* `GET /api/recipes`
* `POST /api/recipes`
* `POST /api/recipes/photo`
* `GET /api/recipes/{id}`
* `PUT /api/recipes/{id}`
* `DELETE /api/recipes/{id}`

Add:

`POST /api/recipes/import`

Request:

```json
{
  "url": "https://example.com/recipe"
}
```

Response should be a preview structure, not a database record.

Example:

```json
{
  "recipe": {
    "name": "Chocolate Cake",
    "sections": [],
    "ingredients": [
      {
        "quantity": 200,
        "unit": "g",
        "name": "flour",
        "prep": null,
        "section": null
      }
    ],
    "instructions": [
      {
        "text": "Preheat the oven to 180°C.",
        "section": null
      }
    ],
    "instruction_sections": [],
    "notes": "",
    "yield": "8 servings",
    "source": "https://example.com/recipe"
  },
  "method": "json_ld",
  "confidence": 0.95,
  "warnings": []
}
```

The client can then let the user edit the result and submit the normal recipe creation request.

Do not automatically write imported recipes into SQLite in the first implementation.

---

# Phase 1 — extraction core

Create:

```text
backend/src/recipe_import/
├── mod.rs
├── fetch.rs
├── json_ld.rs
├── html.rs
├── normalize.rs
├── score.rs
└── types.rs
```

Keep responsibilities separate.

## `types.rs`

Define importer-specific intermediate types.

Do not deserialize external Schema.org data directly into `shared::RecipeInput`.

Create an intermediate `SchemaRecipe` / extraction representation because real JSON-LD is inconsistent.

Important fields:

* name
* description
* image
* author
* recipeIngredient
* recipeInstructions
* recipeYield
* prepTime
* cookTime
* totalTime
* nutrition
* source URL

Use `Option<Value>` where an external Schema.org field can have multiple real-world shapes.

Do not make external webpage structures part of `shared`.

---

# Phase 2 — fetching

Implement a reusable fetcher around the existing workspace `reqwest` dependency.

Use:

* one reusable `reqwest::Client`
* redirect support
* reasonable timeout
* bounded response size
* a descriptive User-Agent
* URL validation

The fetcher should return:

```rust
pub struct FetchedPage {
    pub final_url: Url,
    pub html: String,
}
```

Do not create a new HTTP client for every request.

Reject obviously unsupported URLs and avoid fetching arbitrary local/private network addresses.

At minimum, do not allow importing:

* `file://`
* localhost
* loopback addresses
* link-local/private network targets

The importer is a server-side URL fetcher, so SSRF protection is required.

Do not bypass bot protection, CAPTCHAs, or access controls.

---

# Phase 3 — JSON-LD parser

This is the primary extraction strategy.

Use `scraper` to find:

```html
<script type="application/ld+json">
```

Parse the contents with `serde_json::Value`.

Do not assume the JSON is a single object.

Support all of these patterns:

```json
{
  "@type": "Recipe"
}
```

```json
{
  "@graph": [
    {
      "@type": "WebSite"
    },
    {
      "@type": "Recipe"
    }
  ]
}
```

```json
[
  {
    "@type": "BreadcrumbList"
  },
  {
    "@type": "Recipe"
  }
]
```

Also support:

```json
"@type": ["Recipe", "Thing"]
```

Implement a recursive JSON traversal which finds every object whose `@type` contains `Recipe`.

Do not rely on fixed object depth.

If multiple Recipe objects exist:

1. prefer the one with the most complete recipe fields
2. prefer one with ingredients and instructions
3. prefer one whose name resembles the HTML page title
4. otherwise use the first sufficiently complete recipe

Do not blindly choose the first `Recipe` object.

---

# Phase 4 — instruction normalization

Support the common Schema.org shapes:

### String

```json
"recipeInstructions": [
  "Preheat the oven.",
  "Mix everything together."
]
```

### HowToStep

```json
"recipeInstructions": [
  {
    "@type": "HowToStep",
    "text": "Preheat the oven."
  }
]
```

### HowToSection

```json
"recipeInstructions": [
  {
    "@type": "HowToSection",
    "name": "For the sauce",
    "itemListElement": [...]
  }
]
```

Preserve order.

For `HowToSection`, map the section name into Bouedig's existing:

```rust
InstructionStep {
    text,
    section,
}
```

and maintain `instruction_sections`.

Do not flatten away section information.

---

# Phase 5 — ingredient normalization

The importer should first extract raw ingredient strings.

Example:

```text
"1 1/2 cups all-purpose flour"
"2 eggs, beaten"
"salt, to taste"
```

Then pass them through a separate normalization function.

The parser should populate Bouedig's existing model:

```rust
Ingredient {
    quantity,
    unit,
    name,
    prep,
    section,
}
```

Important:

* preserve the original textual meaning
* do not invent quantities
* allow `quantity = None`
* allow `unit = None`
* put preparation text such as `finely chopped` into `prep`
* preserve section names

Do not add a sophisticated ingredient NLP dependency in phase one unless it is actually necessary.

Implement a modest deterministic parser first:

* Unicode fractions
* ASCII fractions
* mixed numbers
* decimals
* quantity ranges
* common cooking units
* `to taste`
* common preparation suffixes

When uncertain, keep the whole ingredient as `name` rather than corrupting it.

For example:

```text
"salt, to taste"
```

must not fail because it has no numeric quantity.

---

# Phase 6 — recipe metadata normalization

Map external recipe metadata into Bouedig's existing fields.

### Name

Required.

If Schema.org does not provide a name:

1. try `<h1>`
2. try `<title>`
3. otherwise fail extraction

### Yield

Map `recipeYield` into:

```rust
RecipeInput::yield_amount
```

Handle both string and array forms.

### Source

Always store the final canonical URL in:

```rust
RecipeInput::source
```

Do not use the page author's name as the source.

### Notes

Do not aggressively populate notes.

Only use clearly recipe-specific description text when appropriate.

Do not dump the entire webpage into `notes`.

---

# Phase 7 — HTML fallback

Only add generic HTML extraction when Schema.org is:

* missing
* malformed
* incomplete

Use `scraper`.

First support semantic attributes:

```css
[itemprop="recipeIngredient"]
[itemprop="recipeInstructions"]
[itemprop="name"]
[itemprop="recipeYield"]
```

Then add conservative structural heuristics.

Do not build a giant CSS-selector scraper.

The HTML parser should primarily recover:

* ingredient grouping
* missing ingredient lines
* missing instruction sections
* title
* metadata missing from JSON-LD

Keep generic HTML extraction conservative.

---

# Phase 8 — merge rather than choose one source

Do not treat JSON-LD and HTML as mutually exclusive.

The desired process is:

```text
JSON-LD
   +
visible HTML
   ↓
normalized recipe
```

For example:

* JSON-LD provides ingredient quantities
* HTML provides section headings
* JSON-LD provides instructions
* HTML provides missing metadata

Do not overwrite high-quality structured data with weaker HTML guesses.

---

# Phase 9 — confidence scoring

Create:

```rust
pub struct ExtractionScore {
    pub confidence: f32,
    pub warnings: Vec<String>,
}
```

Confidence should be based on observable extraction quality, not arbitrary optimism.

Consider:

* recipe name present
* ingredients present
* instructions present
* reasonable ingredient count
* reasonable instruction count
* valid positive quantities
* yield present
* JSON-LD source
* HTML agreement with JSON-LD
* missing required information

Examples of warnings:

```text
"recipe has no instructions"
"recipe has no ingredients"
"some ingredient quantities could not be parsed"
"multiple Recipe objects were found"
"recipe was recovered from HTML instead of structured data"
```

Confidence is informational. Do not block an otherwise usable recipe solely because the score is not high.

---

# Phase 10 — API integration

Add an importer request type, for example:

```rust
#[derive(Debug, Deserialize)]
pub struct RecipeImportRequest {
    pub url: String,
}
```

Add a preview response type.

Keep these importer-specific API types in `backend` initially unless the frontend needs them. Do not modify `shared` until the shared client genuinely needs the types.

Add:

```rust
POST /api/recipes/import
```

The handler should:

1. validate the URL
2. fetch HTML
3. parse JSON-LD
4. fall back to HTML extraction
5. normalize
6. score
7. return preview

It should not create a database row.

Reuse the existing `validate_recipe_input()` logic where appropriate.

---

# Phase 11 — do not add browser rendering yet

Do NOT add `thirtyfour`, Chrome, Selenium, or browser infrastructure in the initial implementation.

Reason:

Bouedig's current `Justfile` and E2E setup already manage a Firefox/geckodriver process, and adding another browser runtime would complicate development and CI. The current project deliberately keeps its E2E browser setup explicit.

Start with ordinary HTTP.

After the importer has fixtures and telemetry showing that JavaScript-rendered recipe pages are a meaningful failure category, introduce a browser fallback behind an abstraction such as:

```rust
#[async_trait]
pub trait PageFetcher {
    async fn fetch(&self, url: &Url) -> Result<FetchedPage>;
}
```

Then have:

```text
HttpFetcher
BrowserFetcher
```

The normal path stays cheap.

---

# Phase 12 — tests are a major part of this feature

Do not rely on live websites in unit tests.

Create:

```text
backend/tests/recipe_import/
├── json_ld_basic.html
├── json_ld_graph.html
├── json_ld_array.html
├── how_to_steps.html
├── how_to_sections.html
├── grouped_ingredients.html
├── malformed_json_ld.html
├── no_json_ld.html
└── incomplete_recipe.html
```

Test the parser against static fixtures.

Minimum tests:

1. single Recipe JSON-LD
2. Recipe inside `@graph`
3. array of JSON-LD objects
4. `@type` as array
5. multiple Recipe objects
6. HowToStep
7. HowToSection
8. Unicode fractions
9. missing quantity
10. grouped ingredients
11. empty Recipe
12. malformed JSON-LD
13. title fallback
14. HTML-only fallback
15. source URL preservation
16. invalid URL
17. SSRF/private-address rejection
18. confidence/warning generation

Use the existing backend's in-memory SQLite test pattern for API-level tests. The current backend already has test helpers using `sqlite::memory:` and `build_router`, so extend that style rather than creating a new test harness.

---

# Phase 13 — API integration test

Add a backend test equivalent to:

```text
POST /api/recipes/import
        ↓
preview returned
        ↓
extract RecipeInput from response
        ↓
POST /api/recipes
        ↓
GET /api/recipes/{id}
        ↓
verify sections, ingredients, instructions and metadata
```

This verifies that the importer produces data compatible with the existing persistence layer.

Do not modify grocery-list behavior.

The existing tests explicitly establish that creating a recipe must not implicitly modify the grocery list. Preserve this behavior.

---

# Phase 14 — frontend integration

Only after the backend importer is stable.

Add an "Import from URL" action to the web client.

UX:

```text
URL input
   ↓
Import
   ↓
loading
   ↓
Recipe preview/edit screen
   ↓
user can correct fields
   ↓
Save Recipe
```

Never silently save an imported webpage directly.

The preview should expose warnings when extraction was incomplete.

Use the existing recipe editor's representation instead of introducing a second recipe editing model.

Android can reuse the same API later.

---

# Phase 15 — dependencies

Add only what is needed initially.

Workspace:

```toml
scraper = "..."
url = "..."
```

Backend:

```toml
scraper.workspace = true
url.workspace = true
```

Do not add a browser dependency yet.

Continue using the existing:

* `reqwest`
* `serde`
* `serde_json`
* `tokio`
* `anyhow`
* `tracing`
* `axum`

The workspace already centralizes these common dependencies in the root `Cargo.toml`; follow that convention.

---

# Phase 16 — logging and diagnostics

Use the existing `tracing` infrastructure.

Log:

* hostname
* extraction method
* success/failure
* confidence
* elapsed time

Do not log:

* full webpage HTML
* potentially huge JSON-LD
* arbitrary user secrets
* request bodies unnecessarily

Example:

```text
recipe import succeeded
host=www.example.com
method=json_ld
confidence=0.96
elapsed_ms=84
```

When extraction fails, log enough information to diagnose the failure without dumping the whole document.

---

# Phase 17 — keep the implementation boundaries clean

Do not:

* put the entire importer into `backend/src/lib.rs`
* add site-specific selectors before generic parsing exists
* use an LLM for every webpage
* automatically save uncertain imports
* add a browser runtime in phase one
* change the database schema just to store temporary extraction metadata
* duplicate the existing `RecipeInput` model
* modify grocery-list behavior
* redesign the existing recipe persistence code

The target dependency direction should be:

```text
HTTP/API handler
      ↓
recipe_import::pipeline
      ↓
fetch
      ↓
JSON-LD / HTML extraction
      ↓
normalization
      ↓
shared::RecipeInput
```

Persistence stays where it is.

---

# Phase 18 — optional future LLM fallback

Do not implement this in the first PR.

Design the importer so that a later stage can do:

```text
HTTP fetch
    ↓
JSON-LD
    ↓
HTML
    ↓
Browser-rendered HTML
    ↓
LLM fallback
```

The LLM should only receive a compact recipe candidate, not arbitrary raw HTML whenever possible.

Its output must be validated into `RecipeInput`.

Never allow the LLM to bypass normal recipe validation.

---

# Definition of done

The first implementation is complete when:

* `POST /api/recipes/import` exists
* ordinary recipe websites with JSON-LD can be imported
* `@graph` and array JSON-LD work
* recipe instructions are normalized
* ingredient sections survive extraction
* Unicode fractions do not break parsing
* missing quantities are supported
* malformed/missing JSON-LD falls back to HTML
* the source URL is preserved
* invalid/private URLs are rejected
* extraction confidence and warnings are returned
* no recipe is persisted automatically
* existing recipe creation/update behavior remains unchanged
* grocery-list behavior remains unchanged
* fixture tests cover the important JSON-LD variants
* API tests exercise import → save → retrieve
* `just check` passes
* existing E2E tests continue to pass

## Recommended implementation order

1. Create importer types.
2. Implement URL validation + HTTP fetcher.
3. Implement generic JSON-LD traversal.
4. Implement Schema.org Recipe normalization.
5. Implement instruction parsing.
6. Implement ingredient normalization.
7. Implement confidence scoring.
8. Add fixture tests.
9. Add HTML fallback.
10. Add `POST /api/recipes/import`.
11. Add API integration tests.
12. Add the web preview/import UI.
13. Measure real failures.
14. Only then consider browser rendering and/or LLM fallback.

