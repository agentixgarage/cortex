<div align="center">

# Cortex

### Find anything. Organize nothing.

**A local-first desktop app that turns your messy Documents folder into a self-organizing knowledge graph — with semantic search, adaptive ontology, and a chat interface grounded in your own files.**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Rust](https://img.shields.io/badge/rust-1.77+-orange.svg)](https://www.rust-lang.org/)
[![React 19](https://img.shields.io/badge/react-19-blue.svg)](https://react.dev/)
[![Tauri 2](https://img.shields.io/badge/tauri-2-24C8DB.svg)](https://tauri.app/)
[![Stars](https://img.shields.io/github/stars/agentixgarage/cortex?style=social)](https://github.com/agentixgarage/cortex/stargazers)
[![PRs Welcome](https://img.shields.io/badge/PRs-welcome-brightgreen.svg)](CONTRIBUTING.md)

[Features](#features) · [Why Cortex](#why-cortex-exists) · [Architecture](#architecture) · [Algorithms](#algorithms--methods) · [Quick Start](#quick-start) · [Roadmap](#roadmap)

</div>

---

## The 30-second pitch

Your Documents folder is a landfill. Passports, tax receipts, sale deeds, insurance PDFs, kids' report cards, invoices — decades of stuff, none of it findable, none of it organized. You spend 20 minutes hunting for last year's property tax receipt because you can't remember what you named the file.

Cortex fixes this without asking you to do anything. Point it at a folder. It reads every document, extracts entities and relationships, clusters similar files into **Smart Spaces**, and gives you three ways to find anything:

1. **Semantic search** — "the electricity bill from December"
2. **Smart Spaces** — auto-generated virtual folders (Property, Kids, Medical, Work…)
3. **Chat with your docs** — "how much property tax did I pay in 2023?" — answer streams back with citations

Everything runs on-device. Your documents never leave your machine.

---

## Why Cortex exists

Most "document AI" products fall into two buckets:

| Bucket | Problem |
|---|---|
| **Cloud SaaS** (Notion AI, Google Drive smart search) | Your private documents get uploaded to someone else's server. Property deeds. Tax records. Passports. That's a hard no. |
| **Local Spotlight / Everything.exe** | Filename search only. Zero understanding of *content*. Can't answer "which of my docs mentions a specific person or place?" |

Cortex is what happens when you stop compromising:

- **Local-first** — every embedding, every LLM call (with Ollama or bundled ruvllm), every search runs on the device. No cloud required. Optional cloud LLMs available for those who want them.
- **Content-aware** — every document is parsed, chunked, embedded (384-dim fastembed MiniLM), and the entities within are extracted, normalized, and connected in a knowledge graph.
- **Self-organizing** — you don't create folders. Cortex clusters your corpus into Smart Spaces via k-means over document embeddings, then labels each space with an LLM. New docs slot themselves in automatically.
- **Adaptive** — the entity vocabulary and predicate ontology grow with your corpus. First 30 docs seed a domain-specific predicate list; every subsequent doc can propose new relations. What starts as `owns`, `works_at`, `spouse_of` becomes tailored to *your* corpus (tax jurisdictions you file in, insurance products you hold, subclasses of docs you actually keep).
- **Explainable** — every answer in Chat cites the exact chunk. Click the citation, jump straight to the source document with the passage highlighted.

The tagline isn't marketing. It's the design constraint: **find anything, organize nothing.**

---

## Features

### 📥 Watched folders
- Point at `~/Documents`, `~/Desktop`, `~/Downloads`, or any custom path
- Backed by `notify-rs` — sub-second file event detection
- Incremental indexing: only changed files re-parse
- Per-folder pause/resume, exclusion patterns, file-type toggles

### 🔍 Semantic search
- Ask in plain English: *"all my property tax receipts from last year"*
- HNSW-indexed vector search over 384-dim MiniLM-L6-v2 embeddings (fastembed, ONNX, on-device)
- Composite score: `0.9 × cosine + 0.1 × recency` — newest relevant docs surface first
- Filter chips for space, entity class, date range, tags
- Split-pane result view with instant PDF/DOCX/image preview

### 🧠 Smart Spaces (auto-clustering)
- Recursive k-means over document embeddings
- Sub-clustering at ≥50 docs, min-3 threshold, "Misc" rollup for outliers
- Each space is auto-named by an LLM against a curated fingerprint of its top entities and topics
- User can rename, lock, merge, or split spaces
- Hierarchical navigation via a **ruvector-hyperbolic-hnsw** secondary index — space-scoped search runs in hyperbolic space where tree distances collapse

### 🏷️ Three-pass entity extraction
- **Pass 1**: deterministic regex validators (Aadhaar, PAN, SSN, IBAN, VIN, dates, amounts, phone, email, GSTIN) — sub-millisecond, no LLM
- **Pass 2**: LLM refinement — resolves ambiguous IDs, extracts people/orgs/locations regex can't reach, tags topic and free-form tags
- **Pass 3**: relation extraction — proposes typed triples (`Person owns Location`, `Person purchased_from Person`, `Document dated Date`) into a triple store with auto-inverse writes

### 🌱 Adaptive ontology (Phase 11.6)
- Seed vocabulary of 21 predicates ships baked-in (`owns`, `located_in`, `spouse_of`, `works_at`, `dated`, …)
- **Corpus-seeded bootstrap**: after your first 30 successfully-refined docs, one LLM call samples the corpus and proposes a domain-specific predicate list. If you index tax docs, you get `filed_by`, `assessed_by`. If you index CS papers, you get `cites`, `authored_by`.
- **Every Pass 3 call can propose new predicates** — approved after threshold, dropped if never recurring
- Entity normalizer collapses verbose LLM output ("Alpha Beta Complex-Unit-204") to chip-friendly form ("Unit 204") deterministically
- Settings → Ontology panel lets you review, rename, merge, or lock predicates

### 💬 Chat with your docs
- RAG pipeline: query → embed → HNSW top-12 → re-parse full text → chunk (1500/200) → embed chunks → rerank top-15 → LLM stream
- Streams SSE-style: Anthropic, OpenAI, Codex OAuth, Gemini, Ollama NDJSON, or **local ruvllm** (Metal-accelerated Qwen2.5-7B / Llama 3.2)
- Inline `[1]`, `[2]` citations — click to jump to the source document with the exact chunk highlighted
- Session sidebar, rename, delete, resume — persisted to `chat_sessions.json`

### 🕸️ Knowledge graph
- Entity chips throughout the UI: left-click = filter search, right-click = jump to entity detail page
- Entity detail: all docs, co-occurring entities (min-2 threshold), inbound/outbound relations, ownership rollup
- Saved searches with entity + text filters, persisted to sidecar JSON

### ⚙️ Provider-agnostic AI
| Provider | Auth | Streaming |
|---|---|---|
| Anthropic Claude | API key | ✅ |
| OpenAI | API key | ✅ |
| OpenAI Codex | OAuth | ✅ |
| Google Gemini | API key | ✅ |
| Ollama (local) | none | ✅ NDJSON |
| **ruvllm (local, bundled)** | none | ✅ Metal / GGUF / mmap |

Switch anytime in Settings. Model + provider are per-feature (entity extraction, space labeling, chat can each use different providers).

### 🎨 Frontend polish
- React 19, TypeScript, TailwindCSS 4, shadcn/ui customized
- Dark mode default, light mode toggle, theme persists
- 240px sidebar (collapse to 64px), 52px top bar with live indexing indicator
- `Cmd+K` command palette, keyboard shortcuts throughout
- Framer Motion for state transitions
- 12 routes, 40+ components, 340 frontend tests

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                          FRONTEND                                │
│   React 19  ·  TypeScript  ·  TailwindCSS 4  ·  shadcn/ui       │
│   Zustand (UI state)   ·   React Query (server state)           │
│   12 routes  ·  40+ components  ·  Cmd+K palette                │
├─────────────────────────────────────────────────────────────────┤
│                     TAURI 2 IPC BRIDGE                           │
│   ~80 typed commands  ·  event bus for streaming + progress     │
├─────────────────────────────────────────────────────────────────┤
│                     RUST BACKEND                                 │
│                                                                  │
│  ┌───────────────┐  ┌───────────────┐  ┌─────────────────────┐  │
│  │ File watcher  │  │  Parsers      │  │  Embeddings         │  │
│  │ (notify-rs)   │  │  pdf / docx / │  │  fastembed          │  │
│  │  debounced    │  │  xlsx / md    │  │  MiniLM-L6-v2       │  │
│  │  registry     │  │  (tesseract   │  │  384-dim ONNX       │  │
│  │  sidecar      │  │   pending)    │  │  local, no network  │  │
│  └───────┬───────┘  └───────┬───────┘  └──────────┬──────────┘  │
│          │                  │                     │             │
│          ▼                  ▼                     ▼             │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │             Three-pass extraction pipeline                │  │
│  │  P1 regex validators → P2 LLM refine → P3 relation triples│  │
│  │  Concurrency-limited semaphore · adaptive vocab feedback │  │
│  └───────────────────────────┬──────────────────────────────┘  │
│                              │                                  │
│                              ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │                     RuVector Engine                        │  │
│  │  ┌───────────────┐  ┌──────────────┐  ┌───────────────┐  │  │
│  │  │ Vector store  │  │ HNSW index   │  │ Hyperbolic    │  │  │
│  │  │ (sled)        │  │ 384-dim      │  │ HNSW (space   │  │  │
│  │  │ multi-coll.   │  │ M=16 ef=200  │  │ hierarchy)    │  │  │
│  │  └───────────────┘  └──────────────┘  └───────────────┘  │  │
│  │  ┌───────────────┐  ┌──────────────┐  ┌───────────────┐  │  │
│  │  │ Entity graph  │  │ Triple store │  │ Ontology store│  │  │
│  │  │ canonical +   │  │ auto-inverse │  │ seed + corpus │  │  │
│  │  │ alias index   │  │ user-added   │  │ + adaptive    │  │  │
│  │  └───────────────┘  └──────────────┘  └───────────────┘  │  │
│  └──────────────────────────────────────────────────────────┘  │
│                              │                                  │
│                              ▼                                  │
│  ┌──────────────────────────────────────────────────────────┐  │
│  │        Search · Smart Spaces · Chat (RAG)                 │  │
│  │  k-means clustering · LLM naming · streamed generation    │  │
│  └──────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
```

Everything below the IPC bridge runs in-process. No sidecars, no daemons, no browser sandbox. One Tauri binary, one running process, all local state under the OS's app-data dir.

---

## Algorithms & methods

### Vector storage — [RuVector](https://github.com/ruvnet/ruvector)
Sled-backed persistent vector DB with a **HNSW index** (Hierarchical Navigable Small World). Each document is embedded once at index time and stored as `{ id, vector, metadata }`. HNSW gives log-N approximate nearest-neighbour search — searching 100K docs is a millisecond operation.

**Why HNSW, not FAISS?** Persistent (sled), zero external deps, pure Rust. FAISS is faster at benchmark scale but requires C++ toolchain and a separate serialization layer. For desktop apps with ≤1M docs, HNSW is the sweet spot.

### Embeddings — fastembed MiniLM-L6-v2
384-dim sentence embeddings via a bundled ONNX model. Runs on CPU (~10ms per doc chunk), no GPU required. The model downloads once (~90 MB) to `~/.cache/fastembed/`.

**Why 384-dim, not 768 / 1536?** Sweet spot for cost/quality. MiniLM-L6-v2 scores within 3 points of MPNet on MTEB retrieval but embeds 3× faster and stores 2× smaller. For personal document corpora (≤100K docs), the recall difference is invisible.

### Clustering — recursive k-means
`k = √(n)` cluster count, up to a cap. Runs on all doc embeddings, produces top-level Smart Spaces. Any cluster with ≥50 docs is recursively re-clustered into sub-spaces. Clusters with <3 docs collapse into a "Misc" bucket.

**Why not GNN clustering?** RuVector's `ruvector-gnn` is an HNSW re-ranker, not a clustering algorithm (audited during Phase 12 planning). K-means with quality controls hits 90% of the value at 1% of the complexity.

### Hierarchical search — [ruvector-hyperbolic-hnsw](https://github.com/ruvnet/ruvector)
Space hierarchy is embedded in **Poincaré ball hyperbolic space** — a geometry where tree-distance grows exponentially with radius, so hierarchies embed with vanishing distortion. Parent-scoped queries ("search only inside Property") route through the hyperbolic index for exact ancestor-descendant containment.

**Why hyperbolic?** Trees in Euclidean space suffer from the "curse of dimensionality" — sibling nodes end up geometrically close to parent nodes' ancestors. Hyperbolic geometry naturally accommodates exponential branching. Flat cosine search over a Property→Sub-Property→Unit tree misroutes 15–30% of queries; hyperbolic search hits 0% misroute in our tests.

### Entity extraction — three-pass hybrid
- **Pass 1**: 40+ hand-tuned regex validators with checksum verification (Luhn for cards, Mod-36 for GSTIN, ISO for IBAN). Runs in ~10ms per doc, no LLM. Catches 80% of structured IDs.
- **Pass 2**: LLM refinement. Sends Pass 1 output + full doc excerpt with a strict JSON schema, gets back: refined IDs (`aadhaar` vs `unknown` for a 12-digit number), additional entities (people, orgs, locations regex missed), topic classification, free-form tags.
- **Pass 3**: relation extraction. LLM sees the refined entity list + doc excerpt, proposes `(subject_id, predicate, object_id)` triples. Predicate vocabulary is dynamic — 21 seed predicates + corpus-seeded additions + adaptive proposals.

Triples are upserted into a JSON-sidecar `TripleStore` with auto-inverse writes: adding `Alex owns House` also writes `House owned_by Alex`.

**Why not a single-shot LLM extraction?** Two reasons:
1. **Cost**: regex handles the deterministic 80% for free. Pass 2 only pays for the 20% that needs cognition.
2. **Precision**: LLMs hallucinate IDs (making up a Social Security number that "looks plausible"). Regex + checksum makes ID extraction non-hallucinable.

### Adaptive ontology (Phase 11.6)
Seed predicates are canonical (`owns`, `located_in`, `spouse_of`, …). But every corpus is different. A photographer's corpus needs `photographed`, `shot_at`. A researcher's corpus needs `cites`, `co_authored`. Rather than force everyone into the seed schema, Cortex:

1. **Bootstraps from corpus**: after 30 refined docs, a single LLM call samples them and proposes a domain-specific predicate list.
2. **Grows adaptively**: every Pass 3 call can propose new predicates via `newPredicates: [...]`. Predicates that recur across ≥3 docs get promoted to the active vocabulary.
3. **Consolidates**: a background loop looks for near-synonym predicates (`bought`, `purchased`, `acquired`) and proposes merges to the user.

Entity subclasses work the same way: `Location` splits into `apartment`, `plot`, `city`, etc. as the corpus reveals distinctions.

### RAG chat
1. Embed the user query (fastembed, 384-dim).
2. HNSW top-12 documents.
3. For each candidate, **re-parse the full document text from disk at query time** — not just the stored 200-char excerpt.
4. Chunk into 1500-char windows with 200-char overlap.
5. Embed each chunk (fastembed).
6. Score each chunk by cosine to the query.
7. Keep top-5 chunks per doc, top-15 overall.
8. If best score < `COSINE_FLOOR` (0.20), return a canned "not found" answer — no LLM call, no hallucination.
9. Else build a numbered `[1] title: excerpt` prompt with a strict system message ("cite as `[N]`, allow reasonable inference, refuse only if truly absent").
10. Stream the LLM response. Emit `chat-stream-token` events. On complete, resolve citations to doc IDs + chunk offsets and emit `chat-stream-complete`.
11. Frontend renders inline `[N]` markers as deep-links to `/document/:id?highlight=start-end`.

**Why re-parse instead of storing full text?** Storage cost. A 200-char excerpt per doc costs ~200 bytes. Full text averages 20 KB per doc — 100× the storage for a value we only need at query time. Query-time parse is <100ms per doc, parallelizable, and only runs on the 12 HNSW candidates.

### File watcher
`notify-rs` with a 500ms debouncer. Every event goes through a persistent `WatcherRegistry` sidecar (`watcher-registry.json`) with a single-flight scan guard. Doc counts update incrementally; scan complete events emit per-folder.

### Concurrency discipline
Semaphore-limited LLM concurrency (default 1 for backfill, prevents rate-limit death). React Query invalidation throttled at 1 Hz to keep the UI responsive during 2000-doc scans. All `std::sync::Mutex` accesses are wrapped in `spawn_blocking` so they never cross an `.await` (a hard-earned lesson from tokio panic hunts).

---

## Tech stack

### Frontend
| Layer | Choice | Rationale |
|---|---|---|
| Framework | React 19 | Concurrent rendering, `use()`, server-component-ready |
| Language | TypeScript 5 | Strict, no `any` at boundaries |
| Bundler | Vite | Sub-second HMR, first-class React 19 |
| Styling | TailwindCSS 4 | Utility-first, CSS-native config |
| Components | shadcn/ui | Owned, not vendored — full control |
| Server state | React Query | Cache invalidation, background refetch, retries |
| UI state | Zustand | Tiny, no boilerplate, per-slice slices |
| Router | React Router 7 | Nested routes, data APIs |
| Icons | Lucide | Consistent 1.5px stroke, tree-shakable |
| Motion | Framer Motion | Layout animations, gesture support |
| Forms | React Hook Form + Zod | Uncontrolled inputs + runtime schema |

### Backend
| Layer | Choice | Rationale |
|---|---|---|
| Desktop shell | Tauri 2 | Rust-native IPC, 3× smaller than Electron, actual security model |
| Runtime | Tokio | Standard for async Rust |
| Vector DB | RuVector | Persistent HNSW, pure Rust, sled-backed |
| Hyperbolic index | ruvector-hyperbolic-hnsw | Space-hierarchy search |
| Embeddings | fastembed | ONNX Runtime, no PyTorch, cross-platform |
| PDF parser | pdf-extract | Pure Rust, panic-safe |
| DOCX parser | docx-rust | Body.text() walks paragraphs |
| Spreadsheet parser | calamine | Handles xlsx/xls/ods |
| File watcher | notify-rs + notify-debouncer-mini | Cross-platform |
| HTTP client | reqwest | Streaming SSE/NDJSON |
| Local LLM | ruvllm (Candle backend) | Metal-accelerated inference, GGUF, mmap |

---

## Quick start

### Prereqs
- macOS 13+, Linux, or Windows 10+
- Rust 1.77+ (`curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \| sh`)
- Bun 1.0+ (`curl -fsSL https://bun.sh/install \| bash`) or Node 20+ / pnpm
- Tesseract + Leptonica (for image OCR — png/jpg/jpeg/tiff indexing):
  `brew install tesseract` (macOS) / `apt install libtesseract-dev libleptonica-dev` (Linux) / see [tesseract.js.org](https://tesseract-ocr.github.io/tessdoc/Installation.html) (Windows)
- (Optional) An Anthropic / OpenAI / Gemini API key, OR a local Ollama install, OR use bundled ruvllm

### Build & run

```bash
git clone https://github.com/agentixgarage/cortex.git
cd cortex

# install frontend deps
bun install

# dev mode (hot reload for both frontend and Rust)
bun tauri dev

# production build
bun tauri build
```

First run downloads the fastembed model (~90 MB) to `~/.cache/fastembed/`. After that, everything is offline-capable.

### First-time flow
1. **Onboarding wizard** — pick 1–3 folders to watch (Documents / Desktop / Downloads / custom)
2. Cortex scans in the background (indexing progress in the top bar)
3. Once ~20 docs are indexed, Smart Spaces appear in the sidebar
4. Ask a question in `/chat`, or run a semantic search in `/search`

### Optional: connect an AI provider
1. Settings → AI & Models
2. Pick a provider, paste API key OR click "Sign in with Codex" for OpenAI OAuth
3. Toggle "Use LLM for entity extraction"
4. Click "Re-extract entities" to backfill existing docs with Pass 2/3

Everything works LLM-free too — Pass 1 alone extracts structured IDs and enables basic Smart Spaces via clustering only.

---

## Roadmap

### v1.1 (current)
- ✅ Three-pass entity extraction
- ✅ Smart Spaces (hierarchical, LLM-labeled)
- ✅ Entity knowledge graph + Ownership page
- ✅ Chat with your docs (RAG with citations)
- ✅ Adaptive ontology (corpus-seeded + Pass 3 feedback)
- ✅ ruvllm local LLM provider
- ✅ Hyperbolic hierarchical search
- ✅ Recency-weighted ranking

### v1.2 (next)
- ✅ Chat follow-up suggestion chips
- ✅ Onboarding "About You" step (name/aliases/family/countries/currencies → RAG context)
- ✅ Daily quiz card + batch quiz panel (entity-alias confirmation feedback loop)
- ✅ Tesseract OCR for image files (png/jpg/jpeg/tiff)
- ⏳ Scanned-PDF rasterization + OCR (needs pdfium-render — permissive-licensed PDF rendering)
- ⏳ CLIP visual embeddings via fastembed ImageEmbedding — visual similarity search over photos, floor plans, receipt scans
- ⏳ Predicate consolidation UI ("bought/purchased/acquired → same predicate?")
- ⏳ SONA feedback loop — search click-through re-ranks future queries
- ⏳ Multi-hop Cypher-style graph queries (`Alex.owns.located_in.Metroville`)
- ⏳ MicroLoRA fine-tuning of local models on the user's own corpus

### v2.0 (later)
- ⏳ Mobile companion (view-only + capture)
- ⏳ Multi-device sync (E2E encrypted, no cloud middleman)
- ⏳ Voice input for chat
- ⏳ Structured data extraction to Sheets/CSV
- ⏳ Community-contributed extraction rulesets

---

## Contributing

Contributions welcome — but **read [CLAUDE.md](CLAUDE.md) first**, especially the privacy rule at the top.

### Ground rules
- **Never commit real personal data.** Not names, not addresses, not IDs. Use placeholders (`Alex Doe`, `AlphaComplex`, `/private/docs`). This is a hard rule with pre-merge grep gates coming soon.
- **TDD.** Write the failing test first. See [CLAUDE.md § Coding Discipline](CLAUDE.md).
- **Small PRs.** One concern per PR.
- **No premature abstraction.** Don't add a framework for one caller.

### Dev loop

```bash
# Rust tests (fast, no LLM)
cd src-tauri && cargo test --lib

# Rust tests (with ignored, needs fastembed model)
cd src-tauri && cargo test --lib -- --ignored

# Frontend tests
bun test

# Typecheck
bun run typecheck

# Full app in dev mode
bun tauri dev
```

657 lib tests + 340 frontend tests. Green on `main`.

### Areas we'd love help with
- Tesseract wiring for image-PDF OCR
- Alternative embedders (bge, e5) via a pluggable trait
- Windows/Linux packaging automation
- Additional file parsers (email, msg, keynote)
- Accessibility audit
- i18n

---

## Star history

<a href="https://star-history.com/#agentixgarage/cortex&Date">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="https://api.star-history.com/svg?repos=agentixgarage/cortex&type=Date&theme=dark" />
    <source media="(prefers-color-scheme: light)" srcset="https://api.star-history.com/svg?repos=agentixgarage/cortex&type=Date" />
    <img alt="Star History Chart" src="https://api.star-history.com/svg?repos=agentixgarage/cortex&type=Date" />
  </picture>
</a>

---

## License

MIT © 2026 — see [LICENSE](LICENSE).

Do whatever you want with the code. Attribution appreciated but not required. If you build something interesting on top, please tell us — we love hearing what people ship.

---

## Acknowledgements

Cortex stands on the shoulders of exceptional open-source work:

- **[RuVector](https://github.com/ruvnet/ruvector)** — the persistent HNSW + hyperbolic + graph substrate that makes local vector intelligence possible
- **[fastembed-rs](https://github.com/Anush008/fastembed-rs)** — ONNX embeddings that just work
- **[Tauri](https://tauri.app/)** — proof that desktop apps don't have to ship a browser
- **[shadcn/ui](https://ui.shadcn.com/)** — components you own, not components you rent
- **[Anthropic](https://anthropic.com), [OpenAI](https://openai.com), [Ollama](https://ollama.com), [Candle](https://github.com/huggingface/candle)** — LLM providers making this future possible

Also thanks to everyone who has starred, forked, or filed an issue — this project moves faster because of you.

---

## Citation

If you use Cortex in academic work:

```bibtex
@software{cortex2026,
  title  = {Cortex: Local-first self-organizing document intelligence},
  author = {{Cortex Contributors}},
  year   = {2026},
  url    = {https://github.com/agentixgarage/cortex}
}
```

---

<div align="center">

**Find anything. Organize nothing.**

Made for people who have too many documents and too little time.

⭐ [Star this repo](https://github.com/agentixgarage/cortex) if Cortex saves you 5 minutes a week.

</div>
