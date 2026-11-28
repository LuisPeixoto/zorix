# ADR-0047: An index for the vectors of documents

Status: accepted (Phase 11)

## Context

A search by meaning compared the question with the vector of every piece the person could see, streaming them all out of SQLite: at 50,000 pieces that is 190 ms
and tens of megabytes, at the old limit of 200,000 pieces about four times as much, and all of it while holding the store's only connection, so every other
memory call waited. A collection that large is a real one (a company's handbook, a code base).

## Decision

- **A cache, never the truth.** The vectors stay in the database. An index of **one-bit codes** (the sign of each number of a vector: 384 dimensions are 48
  bytes) is kept in memory and in a file next to the database (`index/documents-<hash>.zvi`, one per tenant and model; the name is a hash, so nothing from
  outside is part of a path). It is only for pieces of documents. Facts and past work (thousands, not hundreds of thousands) are still compared exactly.
- **Rank by codes, order by vectors.** A search ranks every visible piece by the number of bits its code shares with the question (a few instructions each),
  takes `max(1,000; 50 per result; one piece in a hundred)` candidates, reads only those vectors from the database and gives the **exact** order among them.
  The cost no longer depends on how many vectors there are, only on how many candidates are reread. The scan holds no connection; the database is touched for the folder list and the
  rerank only.
- **Small collections are exact.** Under 20,000 visible pieces nothing changes: the exact search is as fast and has no error. The threshold is a setting of the store, for tests.
- **A counter says when it is stale.** Triggers on the pieces and the vectors raise `vector_generation` (schema 15) on every insert and delete, whatever code did
  it. An index made at another number is not used: it is read again from the vectors (a file made at the current number is read instead, so a restart does not reread everything).
  While one search is making it, another is answered exactly instead of waiting.
- **A damaged file is ignored.** The file has a version, the size of its vectors, a count, its counter and a checksum; its size must be exactly what the header says
  before anything is allocated; a change to any byte or a cut file is refused and the index is made again from the vectors. Tests change bytes and cut the file.
- **Scope is checked on the folders, not baked in.** Each piece is stored with its folder; a search is given the folders visible to the run (global, its project, its
  agent) that are turned on, so scopes, tenants and a folder turned off need no rebuild.
- **The limit rises to 500,000 pieces per tenant,** as far as it was measured.

## Measured

Clustered vectors of 128 dimensions (200 clusters, so a question has true neighbours), on an Intel 3.4 GHz Mac with an SSD:

| Pieces | Search before | Search now | The top ten that is kept | The first search, which makes the index | Index file |
|---|---|---|---|---|---|
| 50,000 | 188 ms | 7 ms | 0.96 | 0.3 s | 0.9 MB |
| 200,000 | about 750 ms | 19 ms | 0.96 | 1.5 s | 3.7 MB |
| 490,000 | about 1.8 s | 49 ms | 0.985 | 4.4 s | 9 MB |

## Consequences

- An answer by meaning can miss a piece that the exact search would have ranked in the top ten: about one in twenty to one in fifty of the true top ten on this data, where
  the neighbourhoods are large and noisy. Real text embeddings are usually easier. The keyword search runs beside it and the two are fused, so a piece that is a good
  keyword match is not lost.
- While documents are being indexed, each change makes the next search read the vectors again (about 1.5 s per 200,000 pieces). That is the first search after a sync, not every
  search.
- The first search after a start reads a small file instead of every vector; if the file is missing it reads every vector once.
- Counting the pieces that still need a vector (once per sync) still reads every piece: 1.1 s at 200,000 and 3.2 s at 490,000.
