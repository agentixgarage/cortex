//! User profile: optional, user-entered context (name/aliases, family,
//! countries, currencies) collected in onboarding's "About You" step.
//! Seeds RAG chat prompts so Cortex can answer questions like "generate
//! my family tree" or "how much did I spend in {currency}" without
//! guessing from ambiguous document content alone.

pub mod store;
