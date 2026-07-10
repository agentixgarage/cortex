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
| 2 | ⏳ Onboarding form | 10 opt-in questions (name aliases, family, countries, currencies, key assets) seed entity graph before first doc. Massive day-1 accuracy boost. | 1 d |
| 3 | ⏳ Daily quiz card | One card/day in Dashboard: "Is Alex Doe the same as A Doe?" [Yes/No/Later]. Feeds alias merging + Pass 2 correction. Non-intrusive. **Plus** a "Take more quizzes" tab under Insights for users who want to actively train the model — batch of 10 at a time, alias merges + entity subclass confirmations + relation confirmations + doc-topic confirmations. All answers logged for feedback loop. | 1 d |
| 4 | ⏳ Tesseract OCR pipeline | Rasterize image-PDFs → tesseract-rs → text → same embedding path. Unblocks passports, scanned receipts, ID cards. | 2 d |
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
