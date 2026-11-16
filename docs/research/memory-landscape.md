# Memory and retrieval landscape

What ZORIX needs to remember, how that can be found again, and the building blocks chosen for Phase 3. Researched in
November 2026; items marked *(verify)* came from secondary sources or could not be confirmed from a primary page and are
re-checked when the code that depends on them is written.

## What "memory" means here

| Kind | Question it answers | Lives for | Written by |
|---|---|---|---|
| Conversation | What did we just say? | One conversation | The runtime (prompts and answers of earlier executions) |
| Long-term | What does this person or project want me to know? | Until changed or forgotten | A person, or the model through an approved tool |
| Knowledge | What do these documents say? | Until the files change | The indexer, from folders a person chose |
| Operational | What did we already try and decide? | Until deleted | The runtime, from finished executions |

Other agent systems mostly keep one pool of "memories" and retrieve by similarity. ZORIX keeps the four apart because
they differ in trust (an approved fact is not a web page), in freshness (a document changes under you) and in how a person
controls them (edit a fact, remove a folder).

## Search: words, meaning, or both

| | Keyword (BM25) | Embeddings | Hybrid |
|---|---|---|---|
| Finds exact names, ids, error text, code symbols | yes | poorly | yes |
| Finds a paraphrase | no | yes | yes |
| Needs a model or network | no | yes | optional |
| Cost to index | very low | a model call per chunk | both |
| Failure mode | misses synonyms | returns something for every query | needs a gate |

**Chosen: hybrid, with keyword search as the floor.** Everything works with no embeddings model; embeddings add recall when one
is configured. Results are merged with **reciprocal rank fusion** (`score = Σ 1/(60 + rank)`), which needs no score
calibration between the two lists. Because fusion always returns *something*, a **relevance gate** keeps a hit only when it
is in both lists, or strongly in one (see ADR-0014).

## SQLite FTS5

The bundled SQLite (`rusqlite` with `bundled`, `libsqlite3-sys` 0.40, SQLite 3.54) is compiled with `SQLITE_ENABLE_FTS5`, so
full-text search needs no new dependency; a test in the store crate proves it on every platform. Used as external-content
tables kept in sync by triggers, one table per corpus so BM25 statistics do not mix. Tokenizer `unicode61` with
`remove_diacritics 2` and `_` as a token character (identifiers). Queries are built from sanitised terms (quoted, joined
with `OR`); raw text never reaches `MATCH`, because FTS5 query syntax (`NEAR`, `*`, `-`, column filters) is a footgun.

## Embedding APIs

| | Ollama | OpenAI and compatible | Gemini |
|---|---|---|---|
| Endpoint | `POST /api/embed` | `POST /v1/embeddings` | `POST /v1beta/models/{m}:batchEmbedContents` |
| Input | `input`: string or array | `input`: string or array, at most 2048 items, 8192 tokens each | `requests[]`, one per text |
| Size control | `dimensions` (models with Matryoshka training) | `dimensions` (`text-embedding-3-*`) | `outputDimensionality` |
| Query vs document | none; model-specific prefixes (nomic: `search_query:` / `search_document:`) | none | `taskType` (`RETRIEVAL_QUERY`, `RETRIEVAL_DOCUMENT`) |
| Too-long input | `truncate` (default true) | error | error *(verify)* |
| Normalised | returns unit vectors | unit vectors | only the full 3072 size is pre-normalised; others must be normalised by the client |
| Usage | `prompt_eval_count` | `usage.prompt_tokens` | none *(verify)* |
| Batch limits | none documented | 2048 items; a total-token cap per request *(verify)* | keep batches small (≲100); an ordering bug with very large batches is reported *(verify)* |
| Anthropic | has **no** embeddings API | | |

Consequences for the adapters:

- Always **L2-normalise** on the client, whatever the provider says, so a dot product is a cosine.
- The vector's size is read from the first answer and stored with it; a vector of another size is never compared.
- Use one batch size per provider (Ollama 32, OpenAI 64, Gemini 50) and check that the answer has one vector per input.
- Anthropic answers `Unsupported`; the router does not fall back to another provider, because vectors of different models
  cannot be compared.

Local model candidates: `all-minilm` (small, fast, English-leaning) and `nomic-embed-text` (longer input, needs prefixes).
The real size and capability list come from `/api/show` (`capabilities` contains `embedding`), and the live test
checks them instead of trusting a table.

## Vector storage

| Option | Verdict |
|---|---|
| **Plain `BLOB` of f32, brute-force dot product in Rust** | **Chosen.** A desktop corpus is thousands to tens of thousands of chunks; 20k × 768 floats ≈ 61 MB, scanned in about 0.1 s from the page cache, and retrieval runs once per execution |
| `sqlite-vec` extension | Adds a native extension to load on three platforms; worthwhile past ~200k vectors. The scan sits behind one store method, so it can replace the scan later |
| A vector database (Qdrant, LanceDB, Chroma) | A second process or large dependency for a problem this size does not have |
| int8 quantisation | A later optimisation behind the same method |

Vectors are keyed by `(model, text_hash)`: identical text is never embedded twice, and moving or renaming a file re-embeds
nothing.

## Building blocks for knowledge

| Need | Candidates | Chosen |
|---|---|---|
| Walk folders respecting `.gitignore` | `ignore` (ripgrep's walker), `walkdir` + manual rules | **`ignore`**: gitignore, global ignore, custom ignore file, no followed links |
| HTML to text | `html2text`, `scraper` + manual | **`html2text`** (pure Rust) |
| PDF text | `pdf-extract` (on `lopdf`), `pdf_oxide`, `lopdf` directly, shelling out to `pdftotext` | **`pdf-extract`** behind a `pdf` feature; isolated and bounded because PDF parsers can panic or hang. `pdf_oxide` is the fallback if its text quality proves better *(verify against fixtures)* |
| Hashing | `sha2` (already a dependency) | **`sha2`** |
| Tokens | none | none: sizes come from the calibrated estimate |

Extraction is text only; scanned PDFs without a text layer are reported as "no text found". OCR is out of scope.

## Token counting

Provider tokenizers are not all public (Claude's is not), and an exact local tokenizer per model would bloat the binary and
still be wrong for some. Every call already reports real usage, and the runtime already records both the estimate
(`execution.model.requested`) and the actual (`execution.model.responded`). **Chosen: calibrate the characters ÷ 4
estimate per model** with an exponentially weighted ratio (ADR-0016). It improves by itself, works offline and adds no
dependency.

## What other systems do, and what ZORIX takes

| System | Idea | ZORIX |
|---|---|---|
| Chat assistants with "memory" | The model decides what to remember | Taken, but **a person approves every write**: the model can only propose |
| Coding agents with project files (`CLAUDE.md`-style) | Project-scoped instructions | Project scope for facts, keyed by the workspace folder |
| RAG frameworks | Chunk, embed, retrieve top-k | Taken, with keyword search as the floor and a relevance gate |
| Agent runtimes with a scratchpad | Memory re-read every step | Rejected: it breaks prompt caching and native-turn replay. Memory is retrieved **once per execution** and frozen (ADR-0015) |

## Sources

- OpenAI API reference, Embeddings: https://developers.openai.com/api/reference/resources/embeddings
- Ollama API, `/api/embed`: https://docs.ollama.com/api/embed
- Gemini API, embeddings: https://ai.google.dev/api/embeddings
- SQLite FTS5: https://sqlite.org/fts5.html
- Reciprocal rank fusion (Cormack, Clarke, Büttcher, 2009)
- The `ignore` crate: https://docs.rs/ignore
