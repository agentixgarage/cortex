# Cortex Roadmap

Public roadmap for Cortex. Anyone can pick up an unclaimed item — open an issue first so we can coordinate.

**Status legend:** ✅ shipped · 🚧 in progress · ⏳ planned · 💡 idea (needs design)

---

## v1.1 — shipped

- ✅ Three-pass entity extraction (regex → LLM refine → LLM relations)
- ✅ Smart Spaces with hierarchical sub-clustering + LLM naming
- ✅ Entity knowledge graph + `/ownership` page
- ✅ Chat with your docs — RAG streaming with inline citations
- ✅ Adaptive ontology (corpus-seeded bootstrap + Pass 3 feedback)
- ✅ ruvllm local LLM provider (Metal / GGUF / mmap)
- ✅ Hyperbolic hierarchical search (ruvector-hyperbolic-hnsw)
- ✅ Recency-weighted ranking
- ✅ Conversation-aware retrieval (multi-turn context)
- ✅ Year & entity query filtering
- ✅ Currency-aware answers
- ✅ Codex OAuth model list corrected (gpt-4o family + o1 family)

---

## v1.2 — core accuracy + trust

**Focus:** every answer must be right, sourced, and complete. Every doc must be indexable.

| # | Feature | Description | Effort |
|---|---|---|---|
| 1 | ✅ Chat suggestions | After each answer, show up to 3 clickable follow-up chips. Click → auto-send. Deterministic (template-based on year/doc-count patterns in the citations, no extra LLM call — zero added latency or rate-limit risk). | 4 h |
| 2 | ✅ Onboarding form | "About You" step (optional, skippable): name, aliases, family members + relations, countries, currencies. Persisted to `user_profile.json`. Injected into every RAG chat system prompt as a "confirmed by the user" context block — fixes cases like "generate my family tree" where the LLM previously refused to state relationships it could only guess at from shared surnames. Editable later in Settings (follow-up). | 1 d |
| 3 | ✅ Daily quiz card | One card/day on Dashboard: "Is {A} the same {type} as {B}?" [Yes/No/Skip]. Candidates are pure token-Jaccard pairs (same entity type, share ≥1 name token, NOT already auto-mergeable) — deterministic, no embeddings/LLM needed. **Plus** a "Take more quizzes" panel under Insights (batch of 10) for users who want to actively help. All Yes/No answers logged to `quiz_feedback.json` (the "how well are we doing" measurement). **Scope note:** answers are logged but do not yet execute an entity merge — `EntityStore` has no pairwise merge method today (only corpus-wide `run_full_alias_merge` + `split_alias`); wiring "Yes" → actual merge is a fast-follow, tracked as its own item below. | 1 d |
| 3b | ⏳ Wire quiz "Yes" answers to entity merge | Add `EntityStore::merge_canonicals(id_a, id_b)` (rewrites `alias_index`, `doc_index`, indexed doc metadata) and call it from `submit_quiz_answer` when `confirmed=true`. Currently answers are logged only. | 1 d |
| 4 | ✅ Tesseract OCR for image files | `png`/`jpg`/`jpeg`/`tiff` files (photos of documents, scanned receipts saved as images) now run through system libtesseract → text → the same fastembed pipeline every other doc uses, no new embedding code needed. Requires `brew install tesseract` (macOS) / `apt install libtesseract-dev libleptonica-dev` (Linux) at build time — documented in README prereqs. | 4 h |
| 4b | ⏳ Scanned-PDF rasterization + OCR | Image-only PDFs (scanned pages saved as PDF, no text layer) still return near-empty text from pdf-extract. Fix: rasterize each page to an image first, then OCR (reuses #4's `ocr_image` path unchanged). Needs a PDF-render dependency — **recommend `pdfium-render`** (Google's PDFium, BSD-3-Clause permissive license) over MuPDF (AGPL — incompatible with MIT) or Poppler (GPL — licensing risk). PDFium ships as a small prebuilt binary per platform (~10-20MB); cross-platform Tauri bundling needs its own careful pass. | 1-2 d |
| 4c | ⏳ Dynamic OpenAI/Codex model list | Hardcoded model default has broken 3 times in one session (`gpt-5`, then `gpt-4o-mini`, then had to be corrected to `gpt-5.6-terra` after research — OpenAI deprecates slugs on a roughly monthly cadence per community reports). Real fix: fetch the live model list from `api.openai.com/v1/models` (API-key path, confirmed endpoint) at connect time and cache it; for `openai-codex` (ChatGPT OAuth), the Codex Responses API has no confirmed public list-models endpoint — worth checking whether the `codex` CLI's own model-selection UI hits an endpoint we could reuse, otherwise keep a curated fallback list but stop treating it as load-bearing. Eliminates an entire class of "model not supported" bugs. | 1 d |
| 5 | ⏳ CLIP visual embeddings | fastembed ImageEmbedding for photos, floor plans, receipt scans. Separate `images_512` collection. Enables "find similar-looking receipts". | 1 d |
| 6 | ⏳ Query classifier | Route by intent (aggregate / lookup / list / compare / recency). Customize retrieval + prompt per type. | 1 d |
| 7 | ⏳ Currency-aware aggregator | Parse `₹1,23,456.78`, `$1,234.56`, `€1.234,56` from any locale. Group + total per currency. Multi-country. | 1 d |
| 8 | ⏳ Doc-scoped chat | "Chat with this doc" button on `/document/:id`. Retrieval locked to one file. Deep-dive on contracts, leases. | 4 h |
| 9 | ⏳ Answer thumbs feedback | 👍/👎 on every chat answer + optional "what was wrong?" chip. Log to `feedback.json`. Weekly summary. Measurable quality. | 4 h |
| 10 | ⏳ User dictionary page | Settings → "here's what we think we know about you": inferred name, family, addresses, entities. Editable, mergeable. Ontology transparency. | 1 d |

---

## v1.3 — views + productivity

**Focus:** turn indexed docs into daily-use views. Ledger, vault, timeline, alerts.

| # | Feature | Description |
|---|---|---|
| 11 | ⏳ Ledger view | All extracted monetary transactions on a timeline. Per-currency, per-entity, per-doc-type. Filter, sort, export. The "money truth" of your corpus. |
| 12 | ⏳ Vault view | All IDs (passports, licenses, cards) in a card grid. Expiry countdown. Copy-to-clipboard with reveal. |
| 13 | ⏳ Timeline view | Docs on a temporal axis. Group by year/month/topic. "What was going on in Q4 2019?" |
| 14 | ⏳ Expiry alerts | Insurance renewal, passport expiry, warranty end, contract end. Native OS notifications (Tauri notification plugin). |
| 15 | ⏳ Export | Chat conversation → PDF. Timeline/ledger → CSV. Send to accountant/lawyer. |
| 16 | ⏳ Chat suggestion memory | Long-term: learn which chip-types the user clicks. Personalize suggestion generation. |

---

## v1.4 — collaboration + polish

| # | Feature | Description |
|---|---|---|
| 17 | 💡 Family shared vault | Multiple family members share a scoped corpus (mutual passports, joint property docs). E2E encrypted. |
| 18 | 💡 Import wizard | One-click import from Google Drive / Dropbox / iCloud Drive (via native APIs, no cloud roundtrip). |
| 19 | 💡 Predicate consolidation UI | "bought / purchased / acquired — same predicate?" User approves merges. |
| 20 | 💡 SONA feedback loop | Search click-through re-ranks future queries. Personalized ranking. |

---

## v2.0 — advanced

| # | Feature | Description |
|---|---|---|
| 21 | 💡 MicroLoRA on-device fine-tuning | After 1K+ docs, fine-tune local ruvllm on user's own extractions. Better than any prompt. |
| 22 | 💡 Multi-device sync | E2E encrypted, no cloud middleman. Options: rqlite, iroh, age + git. Mobile + desktop parity. |
| 23 | 💡 Voice input | Dictate a query, whisper.cpp local. Hands-free. |
| 24 | 💡 Structured extraction to spreadsheet | User picks a doc type → extractor writes fields to a spreadsheet (CSV/Sheets). "All my receipts → CSV." |
| 25 | 💡 Community extraction rulesets | Share extraction rules for domains (US tax, UK NHS, DE insurance). Signed + reviewed. Network effect. |
| 26 | 💡 Multi-hop Cypher-style graph queries | `Alex.owns.located_in.Metroville` — chain relations. Uses ruvector-graph. |
| 27 | 💡 Mobile companion app | View-only + capture (scan a receipt, sync back). React Native + Tauri Mobile. |

---

## Guiding principles

Every roadmap item must satisfy:

1. **Local-first** — no cloud dependency. Optional cloud LLMs allowed for compute; user data stays on device.
2. **Explainable** — every answer must cite its source.
3. **Privacy-first** — [see CLAUDE.md privacy rule](CLAUDE.md). No real PII in code, tests, prompts, or issues.
4. **Additive** — new features must not break existing workflows. Feature flags for anything risky.
5. **Testable** — TDD. New behavior needs a failing test first.
6. **Measurable** — where possible, ship with a metric (feedback thumbs, query success rate, indexing latency).

---

## How to contribute

1. Comment on the issue tracking the feature (or file one if none exists).
2. Read [CLAUDE.md](CLAUDE.md) — especially the privacy rule and the coding discipline section.
3. Fork, branch (`feat/v1.2-chat-suggestions`), TDD, PR.
4. Small PRs preferred. One concern per PR.

Reach out in Discussions before starting anything large.
