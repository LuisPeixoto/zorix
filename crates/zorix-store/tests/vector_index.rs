//! The index of document vectors (ADR-0047): with it a search returns the same pieces as the exact one when the candidates cover the collection, it keeps
//! scope and tenants apart, it follows every change to the documents, it survives a restart through its file and a damaged file is made again, and on
//! clustered data its candidates keep nearly all of the true neighbours.

use std::path::PathBuf;

use zorix_core::{
    Corpus, DocState, DocStatus, DocumentWrite, KnowledgeRootId, MemoryStore, NewChunk, NewRoot, Scope, TenantId,
    Timestamp, Visibility, text_hash,
};
use zorix_store::SqliteMemoryStore;

const NOW: Timestamp = Timestamp(1_000_000);
const MODEL: &str = "m";
const DIMS: usize = 64;

/// A small deterministic generator, so the data is the same on every run.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn unit(&mut self) -> f32 {
        (self.next() % 20_001) as f32 / 10_000.0 - 1.0
    }
}

fn normalised(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-6);
    v.iter_mut().for_each(|x| *x /= n);
    v
}

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).unwrap()
}

async fn root(store: &SqliteMemoryStore, who: &str, scope: Scope, path: &str) -> KnowledgeRootId {
    store
        .add_root(
            &tenant(who),
            NewRoot {
                scope,
                path: path.into(),
                include: vec![],
                exclude: vec![],
                max_file_bytes: 1_000_000,
                auto_sync: false,
            },
            NOW,
        )
        .await
        .unwrap()
        .id
}

/// A file of pieces with the given vectors.
async fn file(store: &SqliteMemoryStore, root: KnowledgeRootId, rel: &str, pieces: &[(String, Vec<f32>)]) {
    let chunks: Vec<NewChunk> = pieces
        .iter()
        .map(|(text, _)| NewChunk {
            text: text.clone(),
            text_hash: text_hash(text),
            heading: None,
            line_start: Some(1),
            line_end: Some(2),
            page: None,
            tokens: 10,
        })
        .collect();
    store
        .write_document(DocumentWrite {
            root,
            rel_path: rel.into(),
            kind: "markdown".into(),
            state: DocState { size: 1, mtime_ns: 1, content_hash: text_hash(rel), status: DocStatus::Indexed },
            error: None,
            chunks,
            indexed_at: NOW,
        })
        .await
        .unwrap();
    store.put_vectors(MODEL, pieces.iter().map(|(t, v)| (text_hash(t), v.clone())).collect()).await.unwrap();
}

/// Pieces of text with their vectors.
type Pieces = Vec<(String, Vec<f32>)>;

/// `clusters` groups of `per_cluster` pieces: each is its cluster's centre plus noise.
fn clustered(rng: &mut Lcg, clusters: usize, per_cluster: usize, label: &str) -> (Vec<Vec<f32>>, Pieces) {
    let centres: Vec<Vec<f32>> = (0..clusters).map(|_| normalised((0..DIMS).map(|_| rng.unit()).collect())).collect();
    let mut pieces = Vec::new();
    for (c, centre) in centres.iter().enumerate() {
        for k in 0..per_cluster {
            let v = normalised(centre.iter().map(|x| x + 0.35 * rng.unit()).collect());
            pieces.push((format!("{label} cluster {c} piece {k}"), v));
        }
    }
    (centres, pieces)
}

fn ids(hits: Vec<zorix_core::VectorHit>) -> Vec<String> {
    hits.into_iter().map(|h| h.id).collect()
}

fn dir_of(path: &std::path::Path) -> PathBuf {
    path.parent().unwrap().join("index")
}

#[tokio::test]
async fn with_candidates_to_spare_the_index_gives_the_exact_answer() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteMemoryStore::open(&dir.path().join("zorix.sqlite")).unwrap();
    let mut rng = Lcg(1);
    let (centres, pieces) = clustered(&mut rng, 6, 20, "a");
    let r = root(&store, "local", Scope::Global, "/docs").await;
    file(&store, r, "a.md", &pieces).await;
    let visible = Visibility::tenant(tenant("local"));
    for centre in &centres {
        store.use_index_from(usize::MAX);
        let exact = ids(store.vector_search(&visible, Corpus::Knowledge, MODEL, centre, 10).await.unwrap());
        store.use_index_from(1);
        let indexed = ids(store.vector_search(&visible, Corpus::Knowledge, MODEL, centre, 10).await.unwrap());
        assert_eq!(indexed, exact, "120 pieces are fewer than the 200 candidates, so nothing can be missed");
    }
}

#[tokio::test]
async fn scope_tenant_and_a_turned_off_folder_are_honoured_through_the_index() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteMemoryStore::open(&dir.path().join("zorix.sqlite")).unwrap();
    store.use_index_from(1);
    let mut rng = Lcg(2);
    let (centres, _) = clustered(&mut rng, 1, 1, "x");
    let shared = |label: &str| -> Vec<(String, Vec<f32>)> {
        (0..30)
            .map(|k| {
                (
                    format!("{label} {k}"),
                    normalised(centres[0].iter().enumerate().map(|(i, x)| x + 0.02 * ((i + k) % 5) as f32).collect()),
                )
            })
            .collect()
    };
    let global = root(&store, "local", Scope::Global, "/global").await;
    let one = root(&store, "local", Scope::Project("/work/one".into()), "/one").await;
    let two = root(&store, "local", Scope::Project("/work/two".into()), "/two").await;
    let other = root(&store, "other", Scope::Global, "/other").await;
    file(&store, global, "g.md", &shared("global")).await;
    file(&store, one, "o.md", &shared("one")).await;
    file(&store, two, "t.md", &shared("two")).await;
    file(&store, other, "x.md", &shared("other")).await;

    let texts = |hits: Vec<zorix_core::VectorHit>, store: &SqliteMemoryStore, who: &str| {
        let ids: Vec<i64> = hits.iter().map(|h| h.id.parse().unwrap()).collect();
        let store = store.clone();
        let who = tenant(who);
        async move { store.chunks(&who, &ids).await.unwrap().into_iter().map(|c| c.text).collect::<Vec<_>>() }
    };
    let in_one = Visibility { project: Some("/work/one".into()), ..Visibility::tenant(tenant("local")) };
    let hits = store.vector_search(&in_one, Corpus::Knowledge, MODEL, &centres[0], 200).await.unwrap();
    let found = texts(hits, &store, "local").await;
    assert!(
        found.iter().any(|t| t.starts_with("global")) && found.iter().any(|t| t.starts_with("one")),
        "its own and the global ones"
    );
    assert!(
        !found.iter().any(|t| t.starts_with("two") || t.starts_with("other")),
        "another project's, or another tenant's: {found:?}"
    );
    let plain = Visibility::tenant(tenant("local"));
    let hits = store.vector_search(&plain, Corpus::Knowledge, MODEL, &centres[0], 200).await.unwrap();
    let found = texts(hits, &store, "local").await;
    assert!(found.iter().all(|t| t.starts_with("global")), "no project, only the global ones: {found:?}");
    let strangers = Visibility::tenant(tenant("nobody"));
    assert!(store.vector_search(&strangers, Corpus::Knowledge, MODEL, &centres[0], 10).await.unwrap().is_empty());

    // A folder that is turned off is not searched, though its pieces stay in the index.
    let mut folder = store.all_roots(&tenant("local")).await.unwrap().into_iter().find(|r| r.id == global).unwrap();
    folder.enabled = false;
    store.update_root(&tenant("local"), folder).await.unwrap();
    let hits = store.vector_search(&plain, Corpus::Knowledge, MODEL, &centres[0], 200).await.unwrap();
    assert!(hits.is_empty(), "the global folder is off");
    let hits = store.vector_search(&in_one, Corpus::Knowledge, MODEL, &centres[0], 200).await.unwrap();
    let found = texts(hits, &store, "local").await;
    assert!(found.iter().all(|t| t.starts_with("one")), "only the project's folder is left: {found:?}");
}

#[tokio::test]
async fn the_index_follows_what_is_added_rewritten_and_removed() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteMemoryStore::open(&dir.path().join("zorix.sqlite")).unwrap();
    store.use_index_from(1);
    let mut rng = Lcg(3);
    let (centres, pieces) = clustered(&mut rng, 2, 15, "base");
    let r = root(&store, "local", Scope::Global, "/docs").await;
    file(&store, r, "base.md", &pieces).await;
    let visible = Visibility::tenant(tenant("local"));
    let search = |q: Vec<f32>, store: SqliteMemoryStore, v: Visibility| async move {
        let hits = store.vector_search(&v, Corpus::Knowledge, MODEL, &q, 5).await.unwrap();
        let ids: Vec<i64> = hits.iter().map(|h| h.id.parse().unwrap()).collect();
        store.chunks(&v.tenant, &ids).await.unwrap().into_iter().map(|c| c.text).collect::<Vec<_>>()
    };
    assert!(
        search(centres[0].clone(), store.clone(), visible.clone())
            .await
            .iter()
            .all(|t| t.starts_with("base cluster 0"))
    );

    // A new file with a piece exactly at the first centre: found at once, first.
    file(&store, r, "new.md", &[("the newest piece".to_string(), centres[0].clone())]).await;
    assert_eq!(search(centres[0].clone(), store.clone(), visible.clone()).await[0], "the newest piece");
    // The file is rewritten without it: gone.
    file(&store, r, "new.md", &[("something else".to_string(), normalised(vec![1.0; DIMS]))]).await;
    assert!(
        !search(centres[0].clone(), store.clone(), visible.clone()).await.contains(&"the newest piece".to_string())
    );
    // The folder is removed: nothing is found, and nothing of it lingers in the index.
    assert!(store.remove_root(&tenant("local"), r).await.unwrap());
    assert!(search(centres[0].clone(), store.clone(), visible.clone()).await.is_empty());
}

#[tokio::test]
async fn the_index_is_kept_in_a_file_that_a_restart_reads_and_a_damaged_one_is_made_again() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let mut rng = Lcg(4);
    let (centres, pieces) = clustered(&mut rng, 4, 25, "doc");
    let visible = Visibility::tenant(tenant("local"));
    let first = {
        let store = SqliteMemoryStore::open(&path).unwrap();
        store.use_index_from(1);
        let r = root(&store, "local", Scope::Global, "/docs").await;
        file(&store, r, "d.md", &pieces).await;
        ids(store.vector_search(&visible, Corpus::Knowledge, MODEL, &centres[1], 8).await.unwrap())
    };
    let files: Vec<_> = std::fs::read_dir(dir_of(&path)).unwrap().map(|e| e.unwrap().path()).collect();
    assert_eq!(files.len(), 1, "one index file: {files:?}");
    assert!(files[0].extension().is_some_and(|e| e == "zvi"));
    let size = std::fs::metadata(&files[0]).unwrap().len();
    assert!(size > 100 && size < 10_000, "it is small: {size}");

    // A restart reads it.
    let reopened = SqliteMemoryStore::open(&path).unwrap();
    reopened.use_index_from(1);
    assert_eq!(ids(reopened.vector_search(&visible, Corpus::Knowledge, MODEL, &centres[1], 8).await.unwrap()), first);

    // A damaged file, and a cut one, are ignored and written again; the answer does not change.
    for damage in [0u8, 1] {
        let mut bytes = std::fs::read(&files[0]).unwrap();
        if damage == 0 {
            let at = bytes.len() / 2;
            bytes[at] ^= 0xff;
        } else {
            bytes.truncate(bytes.len() / 3);
        }
        std::fs::write(&files[0], &bytes).unwrap();
        let again = SqliteMemoryStore::open(&path).unwrap();
        again.use_index_from(1);
        assert_eq!(ids(again.vector_search(&visible, Corpus::Knowledge, MODEL, &centres[1], 8).await.unwrap()), first);
        assert!(zorix_store::vindex::KnowledgeIndex::load(&files[0]).is_some(), "the file was made again");
    }
}

#[tokio::test]
async fn on_clustered_data_the_candidates_keep_nearly_all_the_true_neighbours() {
    let dir = tempfile::tempdir().unwrap();
    let store = SqliteMemoryStore::open(&dir.path().join("zorix.sqlite")).unwrap();
    let mut rng = Lcg(5);
    let (centres, pieces) = clustered(&mut rng, 50, 60, "c");
    let r = root(&store, "local", Scope::Global, "/docs").await;
    for (n, part) in pieces.chunks(500).enumerate() {
        file(&store, r, &format!("part-{n}.md"), part).await;
    }
    let visible = Visibility::tenant(tenant("local"));
    let (mut kept, mut total) = (0usize, 0usize);
    for centre in centres.iter().take(25) {
        // A question near the cluster: its centre, nudged.
        let question = normalised(centre.iter().map(|x| x + 0.1 * rng.unit()).collect());
        store.use_index_from(usize::MAX);
        let exact = ids(store.vector_search(&visible, Corpus::Knowledge, MODEL, &question, 10).await.unwrap());
        store.use_index_from(1);
        let indexed = ids(store.vector_search(&visible, Corpus::Knowledge, MODEL, &question, 10).await.unwrap());
        kept += exact.iter().filter(|id| indexed.contains(id)).count();
        total += exact.len();
    }
    let recall = kept as f64 / total as f64;
    assert!(recall >= 0.95, "recall@10 was {recall:.3} over 3,000 pieces with 200 candidates");
}
