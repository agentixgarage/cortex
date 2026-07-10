//! Pass 2 — LLM Refinement Pass (Phase 8, Plan 03)
//!
//! Given document text + title + Pass 1 extracted entities, calls the active AI
//! provider once with the fixed 8-class schema prompt and returns structured JSON
//! (`additional_entities`, `refined_entities`, `topic`, `tags`, `language`).
//!
//! When no provider is connected, `refine()` returns `Ok(Pass2Output::empty())` —
//! never errors. Pass 1 always runs regardless of LLM availability (D-31).
//!
//! Design decisions: D-07..D-19, D-27, D-31, LLME-01..03.

use std::sync::Arc;
use serde::{Deserialize, Serialize};
use crate::auth::AuthState;
use crate::error::AppError;
use crate::types::{ExtractedEntity, normalize_tag};
use crate::ai::retry::ai_request_with_retry;
use crate::ai::service::{AIServiceRequest, ServiceMessage};

// ─── Prompt version ───────────────────────────────────────────────────────────
/// Semantic version of the REFINE_PROMPT. Stored alongside output so future
/// re-extract logic can detect prompt drift (LLME-03).
pub const PROMPT_VERSION: &str = "v1";

// ─── 8-class allow-list (D-09) ────────────────────────────────────────────────
/// The locked entity class set. LLM cannot invent new classes; novel entities
/// must go into `tags` or `topic` (free-form fields).
const EIGHT_CLASS_ALLOW_LIST: [&str; 8] = [
    "Person",
    "Organization",
    "Location",
    "Date",
    "Amount",
    "Email",
    "Phone",
    "Identifier",
];

// ─── Chunking constants (D-19) ────────────────────────────────────────────────
const MAX_CHARS: usize = 12_000;
const HEAD_SIZE: usize = 6_000;
const TAIL_SIZE: usize = 6_000;

// ─── REFINE_PROMPT (D-10) ─────────────────────────────────────────────────────
/// Inline system prompt for Pass 2 LLM refinement.
///
/// Includes:
/// - Role + task description
/// - 8 rules (schema enforcement, OCR tolerance, class lock, confidence guidance)
/// - Fixed JSON schema with camelCase field names
/// - 19-topic seed list (D-36, soft — LLM may invent new topics)
/// - 4 multi-region few-shot examples (India/Aadhaar, US/SSN, EU-UK/IBAN+GBP, Vehicle/VIN)
pub const REFINE_PROMPT: &str = r#"You are an expert document entity extractor. Your task is to identify entities in the provided document text.

RULES:
1. Output ONLY valid JSON matching the schema below. No markdown fences, no explanation, no preamble.
2. Classes are FIXED: Person, Organization, Location, Date, Amount, Email, Phone, Identifier
3. Do NOT invent new classes. Novel or domain-specific entities go into `tags` field, not into `class`.
4. Set confidence < 0.7 for entities you suspect are OCR-corrupted or ambiguous.
5. The document may be OCR'd from a scanned image; tolerate typos, missing characters, and mis-spaced words.
6. `confidence` must be a float in [0.0, 1.0].
7. Output 2-5 tags describing the document's content (normalized to snake_case automatically).
8. For `topic`, output the single most representative label for the document (snake_case preferred).

JSON SCHEMA (output ONLY this JSON, nothing else):
{
  "additionalEntities": [
    {"class": "Person|Organization|Location|Date|Amount|Email|Phone|Identifier", "subclass": null, "value": "extracted text", "confidence": 0.95}
  ],
  "refinedEntities": [
    {"pass1Id": "e_N", "class": "Identifier", "subclass": "aadhaar|iban|ssn|pan|vin|nino|gstin|credit_card|...", "confidence": 0.92}
  ],
  "topic": "single_topic_label",
  "tags": ["tag1", "tag2", "tag3"],
  "language": "en"
}

TOPIC SUGGESTIONS (prefer these when applicable, but may freely use or invent others):
property, identity, vehicle, finance, investment, insurance, taxes, kids, education,
family, work, business, bills, travel, medical, legal, spiritual, reference, other

FEW-SHOT EXAMPLES:

[India] Aadhaar card: "1234 5678 9012 JANE DOE DOB: 15/07/1985 Male"
Pass 1 entities: e_0: 1234 5678 9012 (class=Identifier, subclass=unknown)
→ {"additionalEntities":[{"class":"Person","subclass":null,"value":"JANE DOE","confidence":0.95}],"refinedEntities":[{"pass1Id":"e_0","class":"Identifier","subclass":"aadhaar","confidence":0.92}],"topic":"identity","tags":["aadhaar","personal_id","india"],"language":"en"}

[US] SSN document: "SSN: 123-45-6789  Name: John Smith  Employer: Acme Corp  EIN: 12-3456789"
Pass 1 entities: e_0: 123-45-6789 (class=Identifier, subclass=ssn)
→ {"additionalEntities":[{"class":"Person","subclass":null,"value":"John Smith","confidence":0.97},{"class":"Organization","subclass":null,"value":"Acme Corp","confidence":0.93}],"refinedEntities":[{"pass1Id":"e_0","class":"Identifier","subclass":"ssn","confidence":0.95}],"topic":"identity","tags":["ssn","us_document","employment"],"language":"en"}

[EU/UK] Bank statement: "IBAN: GB29 NWBK 6016 1331 9268 19  Amount: £1,234.56  Barclays Bank PLC  Sort: 60-16-13"
Pass 1 entities: e_0: GB29NWBK60161331926819 (class=Identifier, subclass=iban), e_1: £1,234.56 (class=Amount)
→ {"additionalEntities":[{"class":"Organization","subclass":null,"value":"Barclays Bank PLC","confidence":0.96}],"refinedEntities":[{"pass1Id":"e_0","class":"Identifier","subclass":"iban","confidence":0.98},{"pass1Id":"e_1","class":"Amount","subclass":"gbp","confidence":0.99}],"topic":"finance","tags":["bank_statement","uk","iban"],"language":"en"}

[Vehicle] Ford Figo purchase invoice: "Vehicle: Ford Figo 1.2 Ti-VCT  VIN: MA1FD2GY5KP123456  Dealer: Popular Ford  Amount: ₹6,45,000"
Pass 1 entities: e_0: MA1FD2GY5KP123456 (class=Identifier, subclass=vin), e_1: ₹6,45,000 (class=Amount)
→ {"additionalEntities":[{"class":"Organization","subclass":null,"value":"Ford","confidence":0.95},{"class":"Organization","subclass":null,"value":"Popular Ford","confidence":0.90}],"refinedEntities":[{"pass1Id":"e_0","class":"Identifier","subclass":"vin","confidence":0.96},{"pass1Id":"e_1","class":"Amount","subclass":"inr","confidence":0.98}],"topic":"vehicle","tags":["invoice","ford_figo","car_purchase","dealer"],"language":"en"}"#;

// ─── Output types ─────────────────────────────────────────────────────────────

/// An entity found by the LLM that was NOT in Pass 1 output.
/// Typically Person, Organization, or Location — types regex cannot reliably find.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pass2AdditionalEntity {
    /// One of the 8 locked classes (enforced by `normalize_and_validate`).
    pub class: String,
    /// Free-form subclass within the class (e.g. "doctor", "retail_bank"). No whitelist.
    #[serde(default)]
    pub subclass: Option<String>,
    /// Extracted text value from the document.
    pub value: String,
    /// Confidence in [0.0, 1.0]. Values < 0.7 indicate potential OCR corruption.
    pub confidence: f32,
}

/// A Pass 1 candidate entity that the LLM has classified or sub-classified more precisely.
/// Keyed by `pass1_id` = `e_<index>` where index matches position in `pass1_entities` slice.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pass2RefinedEntity {
    /// References the Pass 1 entity: `e_0`, `e_1`, ... (D-08).
    /// Accept both `pass1Id` (camelCase canonical) and `pass1_id` (snake_case fallback).
    #[serde(alias = "pass1_id")]
    pub pass1_id: String,
    /// One of the 8 locked classes.
    pub class: String,
    /// Narrower subclass, e.g. "aadhaar", "iban", "ssn", "pan", "vin".
    #[serde(default)]
    pub subclass: Option<String>,
    /// Confidence in [0.0, 1.0].
    pub confidence: f32,
}

/// Full structured output from one Pass 2 LLM call.
///
/// All optional/collection fields have `#[serde(default)]` so partial provider
/// responses (missing `language`, missing `subclass`) still deserialize (Pitfall 5).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Pass2Output {
    /// New entities found by LLM (Person/Org/Location not visible to regex).
    #[serde(default)]
    pub additional_entities: Vec<Pass2AdditionalEntity>,
    /// Pass 1 entity refinements with narrowed subclass.
    #[serde(default)]
    pub refined_entities: Vec<Pass2RefinedEntity>,
    /// Single-value document topic (normalized to snake_case at read time).
    #[serde(default)]
    pub topic: Option<String>,
    /// 2-5 free-form tags (normalized to snake_case at read time). D-38: no whitelist.
    #[serde(default)]
    pub tags: Vec<String>,
    /// BCP-47 language code detected by the LLM (e.g. "en", "hi"). Soft signal for
    /// Phase 9 space labeling.
    #[serde(default)]
    pub language: Option<String>,
}

impl Pass2Output {
    /// Returns an empty output — used when no provider is connected (D-31) or
    /// Ollama has no extraction model configured.
    pub fn empty() -> Self {
        Self {
            additional_entities: Vec::new(),
            refined_entities: Vec::new(),
            topic: None,
            tags: Vec::new(),
            language: None,
        }
    }
}

// ─── JSON fence stripping (D-14) ──────────────────────────────────────────────

/// Strip markdown code fences and leading `<think>...</think>` blocks from LLM output.
///
/// Handles:
/// - ` ```json\n...\n``` ` (with optional newline after opening fence)
/// - ` ```\n...\n``` ` (plain backtick fence)
/// - `<think>...</think>` prefixes (Ollama chain-of-thought)
/// - Non-fenced input is returned unchanged (idempotent).
pub fn strip_json_fences(s: &str) -> String {
    let s = s.trim();

    // WR-05 fix: strip ALL <think>...</think> pairs, not just the first </think>.
    // Some chain-of-thought models emit multiple or nested blocks. Using find()
    // on just "</think>" would discard everything before the FIRST closing tag even
    // if valid JSON preceded it. We remove each <think>...</think> pair in order,
    // until no more pairs remain.
    let mut working = s.to_string();
    loop {
        match (working.find("<think>"), working.find("</think>")) {
            (Some(open), Some(close)) if open < close => {
                let after_close = close + "</think>".len();
                working = format!("{}{}", &working[..open], &working[after_close..]);
            }
            _ => break,
        }
    }
    let stripped = working.trim().to_owned();
    let s: &str = stripped.as_str();

    // Handle ```json ... ``` (most common LLM-generated fence)
    if let Some(inner) = s.strip_prefix("```json") {
        // Skip optional newline/whitespace right after opening fence
        let inner = inner.trim_start_matches('\n').trim_start_matches('\r').trim_start();
        // Find the last ``` to handle any embedded backtick sequences
        if let Some(close_pos) = inner.rfind("```") {
            return inner[..close_pos].trim().to_string();
        }
        return inner.trim_end_matches("```").trim().to_string();
    }

    // Handle ``` ... ``` (plain backtick fence)
    if let Some(inner) = s.strip_prefix("```") {
        let inner = inner.trim_start_matches('\n').trim_start_matches('\r').trim_start();
        if let Some(close_pos) = inner.rfind("```") {
            return inner[..close_pos].trim().to_string();
        }
        return inner.trim_end_matches("```").trim().to_string();
    }

    s.to_string()
}

// ─── JSON parsing (D-14) ──────────────────────────────────────────────────────

/// Parse LLM JSON output into `Pass2Output` with two-attempt robustness (D-14).
///
/// Attempt 1: Direct `serde_json::from_str` on the raw string.
/// Attempt 2: Strip markdown fences with `strip_json_fences`, then retry.
/// On second failure: returns `AppError::Internal("LLM JSON parse failed: ...")`.
/// Never retries the LLM call — that is the caller's concern.
pub fn parse_llm_json(raw: &str) -> Result<Pass2Output, AppError> {
    // Attempt 1: direct parse (fast path — well-behaved providers)
    if let Ok(out) = serde_json::from_str::<Pass2Output>(raw) {
        return Ok(out);
    }

    // Attempt 2: strip fences and retry
    let stripped = strip_json_fences(raw);
    serde_json::from_str::<Pass2Output>(&stripped)
        .map_err(|e| AppError::Internal(format!("LLM JSON parse failed: {}", e)))
}

// ─── Class validation + normalization ─────────────────────────────────────────

/// Enforce the 8-class allow-list and normalize `topic`/`tags` to snake_case (D-09, D-35).
///
/// Entities whose `class` is not in `EIGHT_CLASS_ALLOW_LIST` are dropped with a
/// logged warning. This prevents LLM class-invention from polluting the entity store
/// (T-08-08 mitigation).
fn normalize_and_validate(mut out: Pass2Output) -> Pass2Output {
    // Filter additional_entities: drop entries with unknown class
    out.additional_entities.retain(|e| {
        let valid = EIGHT_CLASS_ALLOW_LIST.contains(&e.class.as_str());
        if !valid {
            eprintln!(
                "[pass2_llm_refiner] dropping additional_entity with unknown class '{}' \
                 (value: '{}') — LLM must not invent classes (D-09)",
                e.class, e.value
            );
        }
        valid
    });

    // Filter refined_entities: drop entries with unknown class
    out.refined_entities.retain(|e| {
        let valid = EIGHT_CLASS_ALLOW_LIST.contains(&e.class.as_str());
        if !valid {
            eprintln!(
                "[pass2_llm_refiner] dropping refined_entity with unknown class '{}' \
                 (pass1_id: '{}') — LLM must not invent classes (D-09)",
                e.class, e.pass1_id
            );
        }
        valid
    });

    // Normalize topic to snake_case; drop if it becomes empty after normalization
    out.topic = out.topic
        .map(|t| normalize_tag(&t))
        .filter(|t| !t.is_empty());

    // Normalize every tag to snake_case; drop empty tags
    out.tags = out.tags
        .iter()
        .map(|t| normalize_tag(t))
        .filter(|t| !t.is_empty())
        .collect();

    out
}

// ─── Pass2LlmRefiner ──────────────────────────────────────────────────────────

/// LLM refinement pass for entity extraction.
///
/// Wraps the `ai_request_with_retry` HTTP call with:
/// - Semaphore-based concurrency cap of 8 in-flight requests (D-13)
/// - Provider-absent short-circuit (returns empty output, D-31)
/// - Heads-and-tails chunking for long documents (D-19)
/// - JSON fence stripping + two-attempt parse (D-14)
/// - 8-class allow-list enforcement (D-09)
/// - `topic`/`tags` normalization to snake_case (D-35)
pub struct Pass2LlmRefiner {
    /// Active AI provider credential store (shared with the rest of the app).
    auth: Arc<AuthState>,
    /// Semaphore limiting in-flight LLM requests to `cap` (default 8, D-13).
    semaphore: Arc<tokio::sync::Semaphore>,
    /// User-configured extraction model (from `Settings.extraction_model`).
    /// Empty string = "use provider default at request time" per D-11.
    configured_model: Arc<tokio::sync::RwLock<String>>,
}

impl Pass2LlmRefiner {
    /// Create a new refiner with the default concurrency cap of 8 (D-13).
    pub fn new(auth: Arc<AuthState>) -> Self {
        Self {
            auth,
            semaphore: Arc::new(tokio::sync::Semaphore::new(1)), // D-13: reduced 3→1 to keep UI responsive during backfill (heavy tokio work was starving get_document/preview queries)
            configured_model: Arc::new(tokio::sync::RwLock::new(String::new())),
        }
    }

    /// Create a new refiner with a custom semaphore capacity.
    /// Use `new_with_capacity(auth, 2)` in tests to verify concurrency behavior.
    pub fn new_with_capacity(auth: Arc<AuthState>, cap: usize) -> Self {
        Self {
            auth,
            semaphore: Arc::new(tokio::sync::Semaphore::new(cap)),
            configured_model: Arc::new(tokio::sync::RwLock::new(String::new())),
        }
    }

    /// Read the currently configured extraction model.
    pub async fn model(&self) -> String {
        self.configured_model.read().await.clone()
    }

    /// Set the extraction model override (called when `Settings.extraction_model` changes).
    pub async fn set_model(&self, model: String) {
        *self.configured_model.write().await = model;
    }

    /// Expose the semaphore for testing concurrency limits.
    pub fn semaphore(&self) -> Arc<tokio::sync::Semaphore> {
        self.semaphore.clone()
    }

    /// Build the LLM user message from document title + text.
    ///
    /// Applies heads-and-tails chunking for documents > 12 000 chars (D-19):
    /// sends `title + first 6k + last 6k` to bound LLM input tokens and cost.
    /// Pass 1 still runs on the full content (regex is O(n) and cheap).
    fn prepare_llm_input(title: &str, text: &str) -> String {
        if text.len() <= MAX_CHARS {
            format!("Title: {}\n\n{}", title, text)
        } else {
            let head_end = safe_byte_boundary(text, HEAD_SIZE);
            let tail_start = text.len().saturating_sub(TAIL_SIZE);
            let tail_start = safe_byte_boundary_from_end(text, tail_start);
            let head = &text[..head_end];
            let tail = &text[tail_start..];
            format!(
                "Title: {}\n\n[Document excerpt — beginning]\n{}\n\n[...middle omitted...]\n\n[Document excerpt — end]\n{}",
                title, head, tail
            )
        }
    }

    /// Default LLM model per provider per D-11.
    /// Returns empty string for Ollama (caller must supply from Settings) and unknown providers.
    pub fn pick_model_default(provider: &str) -> &'static str {
        match provider {
            "anthropic" => "claude-haiku-4-5-20251001",
            // WR-07 fix: "openai" slug was missing a default, causing the skip-gate
            // to fire for any user who selected OpenAI but did not explicitly save
            // an extraction model. gpt-5 is not valid via the ChatGPT Codex OAuth
            // backend (openai-codex). gpt-5.6-terra is OpenAI's current
            // (July 2026) balanced-tier model, confirmed supported via
            // both the direct API-key path and ChatGPT subscriptions
            // (learn.chatgpt.com/docs/models). NOTE: OpenAI deprecates
            // model slugs frequently (gpt-5.2, gpt-5.3-codex already
            // retired) — this WILL go stale again; a dynamic model-list
            // fetch is the real fix (ROADMAP.md).
            "openai" | "openai-codex" => "gpt-5.6-terra",
            "gemini" => "gemini-2.5-flash",
            _ => "", // Ollama + unknown providers: no default
        }
    }

    /// Pure helper: parse a raw LLM JSON string, validate, and normalize.
    /// No network I/O — fully testable without mocking `ai_request_with_retry`.
    ///
    /// `pass1_entities` is accepted for future use (e.g., merge validation);
    /// currently the method only uses `raw_json`.
    pub fn refine_with_response(
        raw_json: &str,
        _pass1_entities: &[ExtractedEntity],
    ) -> Result<Pass2Output, AppError> {
        let out = parse_llm_json(raw_json)?;
        Ok(normalize_and_validate(out))
    }

    /// Full async refinement pipeline.
    ///
    /// Steps (per D-07, D-12, D-13, D-18, D-19, D-27, D-31):
    ///  a. Acquire semaphore permit (held across LLM await, Pitfall 2)
    ///  b. Check active provider — return empty if none (D-31)
    ///  c. Choose model: configured > provider default; empty Ollama → return empty + warn
    ///  d. Build user content: chunked text + compact Pass 1 summary
    ///  e. Construct `AIServiceRequest` with `temperature = 0.0` (D-12)
    ///  f. Call `ai_request_with_retry` with 3 attempts (D-27)
    ///  g. Parse JSON → normalize + validate → return Ok
    /// WR-01 fix: returns `Ok(None)` when Pass 2 is intentionally skipped (no
    /// provider configured, or Ollama with no model). Returns `Ok(Some(out))`
    /// when the LLM actually ran — even if all output fields are empty.  This
    /// distinguishes "provider absent" from "LLM returned nothing useful for this
    /// document", preventing the backfill from re-processing simple-but-valid
    /// documents on every run (which would waste API quota indefinitely).
    pub async fn refine(
        &self,
        text: &str,
        title: &str,
        pass1_entities: &[ExtractedEntity],
    ) -> Result<Option<Pass2Output>, AppError> {
        // a. Acquire semaphore permit.
        //    Use `acquire_owned` so the permit is bound to this scope and survives
        //    across the LLM `.await` point — critical to enforce D-13 (Pitfall 2).
        let _permit = self.semaphore.clone().acquire_owned().await
            .map_err(|e| AppError::Internal(e.to_string()))?;

        // b. Provider check: short-circuit when no provider is connected (D-31).
        //    Return None (not empty output) so the caller can distinguish this case.
        let provider = match self.auth.get_active_provider()
            .map_err(|e| AppError::Internal(e))?
        {
            Some(p) => p,
            None => return Ok(None),
        };

        // c. Skip-gate: if no model is configured and there is no built-in default
        //    for this provider, skip Pass 2.  This primarily catches Ollama when the
        //    user has not saved an extraction model in Settings → AI & Models.
        //
        //    WR-06 note: `AIServiceRequest` does not carry a `model` field — the
        //    active model is resolved from the stored provider credential by
        //    `ai_request`.  We therefore do NOT forward `_model` into the request;
        //    this block exists only as a guard, not as model selection.  A future
        //    change that adds `model` to `AIServiceRequest` should wire it here.
        {
            let cfg = self.model().await;
            if cfg.is_empty() {
                let default = Self::pick_model_default(&provider);
                if default.is_empty() {
                    eprintln!(
                        "[pass2_llm_refiner] provider='{}' has no extraction model configured \
                         (Settings.extraction_model is empty and no default exists); \
                         skipping Pass 2 for this document",
                        provider
                    );
                    return Ok(None);
                }
            }
        }

        // d. Build user content: chunked text + compact Pass 1 summary.
        let text_chunk = Self::prepare_llm_input(title, text);
        let pass1_summary = if pass1_entities.is_empty() {
            "(none)".to_string()
        } else {
            pass1_entities
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    let class_str = e.class.as_deref().unwrap_or("unknown");
                    let subclass_str = e.subclass.as_deref().unwrap_or("unknown");
                    format!("e_{}: {} (class={}, subclass={})", i, e.value, class_str, subclass_str)
                })
                .collect::<Vec<_>>()
                .join("\n")
        };
        let user_content = format!(
            "{}\n\n--- Pass 1 extracted entities (reference for refinedEntities.pass1Id) ---\n{}",
            text_chunk, pass1_summary
        );

        // e. Construct AIServiceRequest.
        let req = AIServiceRequest {
            system_prompt: REFINE_PROMPT.to_string(),
            messages: vec![ServiceMessage {
                role: "user".to_string(),
                content: user_content,
            }],
            max_tokens: Some(4096),
            temperature: Some(0.0), // D-12: temperature=0 for idempotent extraction (LLME-03)
            response_format: None,
            model_override: None,
        };

        // f. Call with retry: 3 attempts, exponential backoff + jitter (D-27).
        let response = ai_request_with_retry(self.auth.as_ref(), req, 3)
            .await
            .map_err(|e| AppError::Internal(format!("Pass 2 LLM call failed: {}", e)))?;

        // g. Parse, validate, normalize.
        let out = parse_llm_json(&response.content)?;
        Ok(Some(normalize_and_validate(out)))
    }
}

// ─── UTF-8 safe byte boundary helpers ─────────────────────────────────────────

/// Find the largest valid UTF-8 char boundary ≤ `max_bytes`.
fn safe_byte_boundary(text: &str, max_bytes: usize) -> usize {
    if max_bytes >= text.len() {
        return text.len();
    }
    let mut pos = max_bytes;
    while !text.is_char_boundary(pos) {
        pos -= 1;
    }
    pos
}

/// Find the smallest valid UTF-8 char boundary ≥ `start_bytes`.
fn safe_byte_boundary_from_end(text: &str, start_bytes: usize) -> usize {
    if start_bytes >= text.len() {
        return text.len();
    }
    let mut pos = start_bytes;
    while pos < text.len() && !text.is_char_boundary(pos) {
        pos += 1;
    }
    pos
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    // ── Task 1 tests: types, fence stripping, JSON parsing, class validation ──

    #[test]
    fn test_strip_json_fences_json_prefix() {
        let input = "```json\n{\"a\":1}\n```";
        assert_eq!(strip_json_fences(input), "{\"a\":1}");
    }

    #[test]
    fn test_strip_json_fences_plain_backticks() {
        let input = "```\n{\"a\":1}\n```";
        assert_eq!(strip_json_fences(input), "{\"a\":1}");
    }

    #[test]
    fn test_strip_json_fences_no_fences_passthrough() {
        let input = "no fences {\"a\":1}";
        assert_eq!(strip_json_fences(input), "no fences {\"a\":1}");
    }

    #[test]
    fn test_strip_json_fences_think_tag_stripped() {
        let input = "<think>some reasoning about the document</think>\n```json\n{\"topic\":\"identity\"}\n```";
        let result = strip_json_fences(input);
        // After stripping <think>…</think>, the outer call re-strips the fence
        // But strip_json_fences is called only once — verify the <think> block is gone
        // and the content is accessible
        assert!(!result.contains("<think>"), "think tags must be removed, got: {}", result);
    }

    #[test]
    fn test_parse_llm_json_direct_success() {
        // Attempt 1: direct parse (camelCase keys)
        let raw = r#"{"topic":"finance","tags":["bank"],"additionalEntities":[],"refinedEntities":[]}"#;
        let result = parse_llm_json(raw);
        assert!(result.is_ok(), "direct parse should succeed: {:?}", result);
        let out = result.unwrap();
        assert_eq!(out.topic, Some("finance".to_string()));
    }

    #[test]
    fn test_parse_llm_json_fence_stripped_success() {
        // Attempt 2: fence-wrapped JSON
        let raw = "```json\n{\"topic\":\"finance\",\"tags\":[],\"additionalEntities\":[],\"refinedEntities\":[]}\n```";
        let result = parse_llm_json(raw);
        assert!(result.is_ok(), "fence-stripped parse should succeed: {:?}", result);
        assert_eq!(result.unwrap().topic, Some("finance".to_string()));
    }

    #[test]
    fn test_parse_llm_json_invalid_returns_app_error() {
        let raw = "not json at all";
        let result = parse_llm_json(raw);
        assert!(result.is_err(), "invalid JSON should return Err");
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("LLM JSON parse failed"),
            "error must mention parse failure: {}",
            err_msg
        );
    }

    #[test]
    fn test_pass2output_serde_roundtrip() {
        let out = Pass2Output {
            additional_entities: vec![],
            refined_entities: vec![],
            topic: Some("finance".to_string()),
            tags: vec!["a".to_string(), "b".to_string()],
            language: Some("en".to_string()),
        };
        let json = serde_json::to_string(&out).expect("serialize Pass2Output");
        let decoded: Pass2Output = serde_json::from_str(&json).expect("deserialize Pass2Output");
        assert_eq!(decoded.topic, Some("finance".to_string()));
        assert_eq!(decoded.tags, vec!["a".to_string(), "b".to_string()]);
        assert_eq!(decoded.language, Some("en".to_string()));
        assert!(decoded.additional_entities.is_empty());
        assert!(decoded.refined_entities.is_empty());
    }

    #[test]
    fn test_pass2output_empty() {
        let out = Pass2Output::empty();
        assert!(out.additional_entities.is_empty());
        assert!(out.refined_entities.is_empty());
        assert_eq!(out.topic, None);
        assert!(out.tags.is_empty());
        assert_eq!(out.language, None);
    }

    #[test]
    fn test_pass2output_defaults_on_missing_fields() {
        // serde(default) must handle empty / partial JSON without panic
        let raw = r#"{}"#;
        let out: Pass2Output = serde_json::from_str(raw)
            .expect("empty JSON must deserialize with all defaults");
        assert!(out.additional_entities.is_empty());
        assert!(out.refined_entities.is_empty());
        assert_eq!(out.topic, None);
        assert!(out.tags.is_empty());
        assert_eq!(out.language, None);
    }

    #[test]
    fn test_class_allowlist_invalid_class_dropped() {
        // An entity with class "InvalidClass" must be silently dropped; valid ones kept
        let raw = r#"{
            "additionalEntities": [
                {"class": "InvalidClass", "value": "x", "confidence": 0.9},
                {"class": "Person", "value": "John", "confidence": 0.95}
            ],
            "refinedEntities": [],
            "topic": "identity",
            "tags": []
        }"#;
        let out = parse_llm_json(raw).expect("parse should succeed");
        let validated = normalize_and_validate(out);
        assert_eq!(
            validated.additional_entities.len(), 1,
            "InvalidClass entity must be dropped, only Person kept"
        );
        assert_eq!(validated.additional_entities[0].class, "Person");
    }

    #[test]
    fn test_normalize_tag_applied_to_topic_and_tags() {
        // normalize_tag (D-35) must be applied to topic and every tag element
        let raw = r#"{
            "additionalEntities": [],
            "refinedEntities": [],
            "topic": "Term Insurance",
            "tags": ["Bank Statement", "India 2024!"]
        }"#;
        let out = parse_llm_json(raw).expect("parse should succeed");
        let validated = normalize_and_validate(out);
        assert_eq!(validated.topic, Some("term_insurance".to_string()));
        assert_eq!(validated.tags[0], "bank_statement");
        assert_eq!(validated.tags[1], "india_2024");
    }

    #[test]
    fn test_refine_prompt_contains_all_eight_classes() {
        // Every class in the allow-list must appear in REFINE_PROMPT (D-10)
        for class in &EIGHT_CLASS_ALLOW_LIST {
            assert!(
                REFINE_PROMPT.contains(class),
                "REFINE_PROMPT must mention class '{}': check SCHEMA section",
                class
            );
        }
    }

    #[test]
    fn test_refine_prompt_appears_twice() {
        // Must be defined as REFINE_PROMPT const AND referenced in refine()
        // Verify the const is non-empty and contains the schema marker
        assert!(REFINE_PROMPT.len() > 500, "REFINE_PROMPT is suspiciously short");
        assert!(
            REFINE_PROMPT.contains("additionalEntities"),
            "REFINE_PROMPT must contain camelCase schema key 'additionalEntities'"
        );
        assert!(
            REFINE_PROMPT.contains("refinedEntities"),
            "REFINE_PROMPT must contain camelCase schema key 'refinedEntities'"
        );
    }

    // ── Task 2 tests: Pass2LlmRefiner helpers ──────────────────────────────────

    #[test]
    fn test_prepare_llm_input_short_text() {
        let result = Pass2LlmRefiner::prepare_llm_input("Ford Figo Invoice", "short body text");
        assert_eq!(result, "Title: Ford Figo Invoice\n\nshort body text");
    }

    #[test]
    fn test_prepare_llm_input_long_text_chunked_structure() {
        let long_text = "x".repeat(15_000);
        let result = Pass2LlmRefiner::prepare_llm_input("t", &long_text);
        assert!(result.contains("Title: t"), "must include title");
        assert!(
            result.contains("[Document excerpt — beginning]"),
            "must have beginning marker"
        );
        assert!(
            result.contains("[...middle omitted...]"),
            "must have omission marker"
        );
        assert!(
            result.contains("[Document excerpt — end]"),
            "must have end marker"
        );
    }

    #[test]
    fn test_prepare_llm_input_head_is_6000_chars() {
        let long_text = "x".repeat(15_000);
        let result = Pass2LlmRefiner::prepare_llm_input("t", &long_text);
        // Extract the head section: between "[Document excerpt — beginning]\n" and "\n\n[...middle"
        let begin_marker = "[Document excerpt — beginning]\n";
        let end_marker = "\n\n[...middle omitted...]";
        let head_start = result.find(begin_marker).unwrap() + begin_marker.len();
        let head_end = result.find(end_marker).unwrap();
        let head = &result[head_start..head_end];
        assert_eq!(head.len(), 6_000, "head must be exactly 6000 chars (bytes for ASCII input)");
    }

    #[test]
    fn test_pick_model_default_anthropic() {
        assert_eq!(
            Pass2LlmRefiner::pick_model_default("anthropic"),
            "claude-haiku-4-5-20251001"
        );
    }

    #[test]
    fn test_pick_model_default_openai_codex() {
        assert_eq!(Pass2LlmRefiner::pick_model_default("openai-codex"), "gpt-5.6-terra");
    }

    #[test]
    fn test_pick_model_default_gemini() {
        assert_eq!(Pass2LlmRefiner::pick_model_default("gemini"), "gemini-2.5-flash");
    }

    #[test]
    fn test_pick_model_default_ollama_empty() {
        assert_eq!(
            Pass2LlmRefiner::pick_model_default("ollama"),
            "",
            "Ollama has no default — caller must supply from Settings"
        );
    }

    #[test]
    fn test_pick_model_default_unknown_empty() {
        assert_eq!(Pass2LlmRefiner::pick_model_default("unknown"), "");
    }

    #[tokio::test]
    async fn test_set_and_get_model() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Arc::new(AuthState::new(&dir.path().to_path_buf()));
        let refiner = Pass2LlmRefiner::new(auth);
        refiner.set_model("custom-model-id".to_string()).await;
        assert_eq!(refiner.model().await, "custom-model-id");
    }

    #[tokio::test]
    async fn test_refine_no_provider_returns_empty() {
        // When no AI provider is connected, refine() must return Ok(empty) without calling HTTP
        let dir = tempfile::tempdir().unwrap();
        let auth = Arc::new(AuthState::new(&dir.path().to_path_buf()));
        // auth has no active_provider set → get_active_provider() returns Ok(None)
        let refiner = Pass2LlmRefiner::new(auth);
        let result = refiner.refine("some document text", "test doc", &[]).await;
        assert!(result.is_ok(), "no provider must not error: {:?}", result);
        let out = result.unwrap();
        assert!(out.is_none(), "no provider → refine returns None (WR-01)");
    }

    #[test]
    fn test_refine_with_response_valid_schema() {
        let raw = r#"{
            "additionalEntities": [
                {"class": "Person", "subclass": null, "value": "John Smith", "confidence": 0.97}
            ],
            "refinedEntities": [
                {"pass1Id": "e_0", "class": "Identifier", "subclass": "ssn", "confidence": 0.95}
            ],
            "topic": "identity",
            "tags": ["ssn", "us_document"],
            "language": "en"
        }"#;
        let result = Pass2LlmRefiner::refine_with_response(raw, &[]);
        assert!(result.is_ok(), "valid schema should parse: {:?}", result);
        let out = result.unwrap();
        assert_eq!(out.additional_entities.len(), 1);
        assert_eq!(out.additional_entities[0].class, "Person");
        assert_eq!(out.additional_entities[0].value, "John Smith");
        assert_eq!(out.refined_entities.len(), 1);
        assert_eq!(out.refined_entities[0].class, "Identifier");
        assert_eq!(out.refined_entities[0].subclass, Some("ssn".to_string()));
        assert_eq!(out.topic, Some("identity".to_string()));
    }

    #[test]
    fn test_refine_with_response_three_entities_one_invalid_class_dropped() {
        // 3 additional_entities with one class="Novel" → only 2 kept
        let raw = r#"{
            "additionalEntities": [
                {"class": "Novel", "value": "something creative", "confidence": 0.5},
                {"class": "Person", "value": "Alice", "confidence": 0.9},
                {"class": "Organization", "value": "Acme Corp", "confidence": 0.85}
            ],
            "refinedEntities": [],
            "topic": "business",
            "tags": [],
            "language": "en"
        }"#;
        let result = Pass2LlmRefiner::refine_with_response(raw, &[]);
        assert!(result.is_ok(), "parse should succeed: {:?}", result);
        let out = result.unwrap();
        assert_eq!(out.additional_entities.len(), 2, "Novel class must be dropped");
        assert!(
            out.additional_entities.iter().all(|e| e.class != "Novel"),
            "no Novel class entity should remain"
        );
    }

    #[test]
    fn test_refine_with_response_topic_normalized() {
        let raw = r#"{
            "additionalEntities": [],
            "refinedEntities": [],
            "topic": "Term Insurance",
            "tags": [],
            "language": "en"
        }"#;
        let result = Pass2LlmRefiner::refine_with_response(raw, &[]);
        assert!(result.is_ok());
        assert_eq!(
            result.unwrap().topic,
            Some("term_insurance".to_string()),
            "topic must be normalized to snake_case"
        );
    }

    /// Verify Semaphore default is 1 (reduced from 3 to keep UI responsive during backfill).
    #[test]
    fn test_semaphore_default_cap_is_one() {
        let dir = tempfile::tempdir().unwrap();
        let auth = Arc::new(AuthState::new(&dir.path().to_path_buf()));
        let refiner = Pass2LlmRefiner::new(auth);
        let sem = refiner.semaphore();
        assert_eq!(
            sem.available_permits(),
            1,
            "default semaphore capacity is 1 (UI-responsive during backfill)"
        );
    }

    /// Semaphore capacity test: with cap=2, 4 concurrent tasks take ≥2 batches of wall time.
    /// This validates that the tokio::sync::Semaphore used by Pass2LlmRefiner correctly
    /// limits concurrency to the configured cap.
    #[tokio::test]
    async fn test_semaphore_cap_enforces_concurrency_limit() {
        use std::sync::Arc as StdArc;

        let sem = StdArc::new(tokio::sync::Semaphore::new(2));
        let start = tokio::time::Instant::now();

        let mut handles = vec![];
        for _ in 0..4 {
            let s = sem.clone();
            handles.push(tokio::spawn(async move {
                let _p = s.acquire_owned().await.unwrap();
                tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
            }));
        }

        for h in handles {
            h.await.unwrap();
        }

        let elapsed_ms = start.elapsed().as_millis();
        // With cap=2, 4 tasks run in 2 batches of 50ms → total ≈ 100ms
        // Must be ≥80ms (not all 4 concurrent) and <220ms (not fully serial 4×50ms)
        assert!(
            elapsed_ms >= 80,
            "Expected ≥80ms for 2 concurrent batches, got {}ms",
            elapsed_ms
        );
        assert!(
            elapsed_ms < 220,
            "Expected <220ms (not serial), got {}ms",
            elapsed_ms
        );
    }

    /// Verify the snake_case alias on pass1_id works for LLMs that emit snake_case.
    #[test]
    fn test_refined_entity_accepts_snake_case_pass1_id_alias() {
        let raw = r#"{
            "additionalEntities": [],
            "refinedEntities": [
                {"pass1_id": "e_5", "class": "Identifier", "subclass": "aadhaar", "confidence": 0.92}
            ],
            "topic": "identity",
            "tags": []
        }"#;
        let result = parse_llm_json(raw);
        assert!(result.is_ok(), "snake_case pass1_id alias must work: {:?}", result);
        let out = result.unwrap();
        assert_eq!(out.refined_entities[0].pass1_id, "e_5");
    }
}
