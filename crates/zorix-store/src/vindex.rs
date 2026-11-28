//! The index that keeps search by meaning fast over a large collection of documents (ADR-0047).
//!
//! Comparing a question with every vector of 200,000 pieces reads hundreds of megabytes of the database on each search and holds its only
//! connection while it does. The index keeps a **one-bit code** of each vector (the sign of each of its numbers: 384 dimensions are 48 bytes) in
//! memory and in a file next to the database. A search ranks every visible piece by how many bits its code shares with the question (a few
//! machine instructions each), takes the best few hundred, and compares only those with their exact vectors, read from the database. The answer
//! is the exact order among the candidates; a piece is lost only if the codes ranked it below hundreds of others that the exact comparison would
//! have put lower, which the benchmark measures.
//!
//! The index is a cache. It is rebuilt from the vectors in the database whenever they change (a counter the database keeps says so), a file that
//! is damaged or does not match is ignored and made again, and a collection too small to need it is searched exactly, as before.

use std::collections::{BinaryHeap, HashMap};
use std::io::Write;
use std::path::Path;

use rusqlite::Connection;
use zorix_core::{Error, Result};

use crate::errors::storage;

/// Collections with fewer visible pieces than this are searched exactly: an approximation is not worth its error there.
pub const EXACT_BELOW: usize = 20_000;
/// The fewest candidates taken from the codes.
pub const MIN_CANDIDATES: usize = 1_000;
/// Candidates taken for each result asked for.
pub const CANDIDATES_PER_RESULT: usize = 50;
/// And one in this many of the pieces that may be looked at: a bigger collection has bigger neighbourhoods, and ranking by one-bit codes is noisy inside one.
pub const ONE_CANDIDATE_IN: usize = 100;

const MAGIC: &[u8; 4] = b"ZVI1";

/// The codes of every piece of one tenant's documents for one embeddings model.
pub struct KnowledgeIndex {
    /// The database counter this was made at.
    pub generation: i64,
    dims: usize,
    words: usize,
    /// The folders the pieces belong to, by position.
    roots: Vec<String>,
    ids: Vec<i64>,
    root_of: Vec<u16>,
    codes: Vec<u64>,
}

/// The one-bit code of a vector: bit `i` is set when number `i` is above zero.
pub fn code_of(vector: &[f32]) -> Vec<u64> {
    let mut code = vec![0u64; vector.len().div_ceil(64)];
    for (i, value) in vector.iter().enumerate() {
        if *value > 0.0 {
            code[i / 64] |= 1 << (i % 64);
        }
    }
    code
}

fn fnv(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

impl KnowledgeIndex {
    /// How many pieces it holds.
    pub fn len(&self) -> usize {
        self.ids.len()
    }

    /// True when it holds nothing.
    pub fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    /// The size of the vectors it was made from.
    pub fn dims(&self) -> usize {
        self.dims
    }

    /// For each folder of the index, whether it is in `visible` (folder ids).
    pub fn mask(&self, visible: &[String]) -> Vec<bool> {
        self.roots.iter().map(|root| visible.iter().any(|v| v == root)).collect()
    }

    /// How many pieces belong to the folders the mask lets through.
    pub fn count_in(&self, mask: &[bool]) -> usize {
        self.root_of.iter().filter(|root| mask.get(usize::from(**root)).copied().unwrap_or(false)).count()
    }

    /// The `k` pieces of the visible folders whose codes are closest to the question's, as piece ids, closest first.
    pub fn candidates(&self, mask: &[bool], query: &[f32], k: usize) -> Vec<i64> {
        if query.len() != self.dims || k == 0 {
            return Vec::new();
        }
        let q = code_of(query);
        // A max-heap of (distance, id): the worst of the best k sits on top and is replaced when something closer comes.
        let mut heap: BinaryHeap<(u32, i64)> = BinaryHeap::with_capacity(k + 1);
        for (i, id) in self.ids.iter().enumerate() {
            if !mask.get(usize::from(self.root_of[i])).copied().unwrap_or(false) {
                continue;
            }
            let code = &self.codes[i * self.words..(i + 1) * self.words];
            let distance: u32 = code.iter().zip(&q).map(|(a, b)| (a ^ b).count_ones()).sum();
            if heap.len() < k {
                heap.push((distance, *id));
            } else if let Some(&(worst, _)) = heap.peek()
                && distance < worst
            {
                heap.pop();
                heap.push((distance, *id));
            }
        }
        let mut best = heap.into_vec();
        best.sort_unstable();
        best.into_iter().map(|(_, id)| id).collect()
    }

    /// Make the index of one tenant's document pieces that have a vector for `model`, reading every vector once.
    pub fn build(conn: &Connection, tenant: &str, model: &str, generation: i64) -> Result<Self> {
        let mut stmt = conn
            .prepare(
                "SELECT c.id, d.root_id, v.vector FROM knowledge_chunks c
                 JOIN knowledge_documents d ON d.id = c.document_id
                 JOIN knowledge_roots r ON r.id = d.root_id
                 JOIN vectors v ON v.model = ?1 AND v.text_hash = c.text_hash
                 WHERE r.tenant = ?2 AND d.status = 'indexed'
                 ORDER BY c.id",
            )
            .map_err(storage)?;
        let mut rows = stmt.query(rusqlite::params![model, tenant]).map_err(storage)?;
        let mut index = Self {
            generation,
            dims: 0,
            words: 0,
            roots: Vec::new(),
            ids: Vec::new(),
            root_of: Vec::new(),
            codes: Vec::new(),
        };
        let mut root_position: HashMap<String, u16> = HashMap::new();
        while let Some(row) = rows.next().map_err(storage)? {
            let id: i64 = row.get(0).map_err(storage)?;
            let root: String = row.get(1).map_err(storage)?;
            let bytes: Vec<u8> = row.get(2).map_err(storage)?;
            if bytes.is_empty() || !bytes.len().is_multiple_of(4) {
                continue;
            }
            let dims = bytes.len() / 4;
            if index.dims == 0 {
                index.dims = dims;
                index.words = dims.div_ceil(64);
            } else if dims != index.dims {
                continue; // a vector of another size is never compared
            }
            let position = match root_position.get(&root) {
                Some(position) => *position,
                None => {
                    let position = u16::try_from(index.roots.len())
                        .map_err(|_| Error::Internal("there are too many folders to index".into()))?;
                    index.roots.push(root.clone());
                    root_position.insert(root, position);
                    position
                }
            };
            let mut code = vec![0u64; index.words];
            for (i, chunk) in bytes.as_chunks::<4>().0.iter().enumerate() {
                if f32::from_le_bytes(*chunk) > 0.0 {
                    code[i / 64] |= 1 << (i % 64);
                }
            }
            index.ids.push(id);
            index.root_of.push(position);
            index.codes.extend_from_slice(&code);
        }
        Ok(index)
    }

    /// Write it next to the database, through a temporary file so a reader never sees half of it.
    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        let mut out: Vec<u8> = Vec::with_capacity(64 + self.ids.len() * (10 + self.words * 8));
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.dims as u32).to_le_bytes());
        out.extend_from_slice(&(self.ids.len() as u64).to_le_bytes());
        out.extend_from_slice(&self.generation.to_le_bytes());
        out.extend_from_slice(&(self.roots.len() as u32).to_le_bytes());
        for root in &self.roots {
            out.extend_from_slice(&(root.len() as u16).to_le_bytes());
            out.extend_from_slice(root.as_bytes());
        }
        for id in &self.ids {
            out.extend_from_slice(&id.to_le_bytes());
        }
        for root in &self.root_of {
            out.extend_from_slice(&root.to_le_bytes());
        }
        for word in &self.codes {
            out.extend_from_slice(&word.to_le_bytes());
        }
        let checksum = fnv(&out);
        out.extend_from_slice(&checksum.to_le_bytes());
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let temporary = path.with_extension("tmp");
        let mut file = std::fs::File::create(&temporary)?;
        file.write_all(&out)?;
        file.sync_all()?;
        std::fs::rename(&temporary, path)
    }

    /// Read it back. `None` when the file is not there, is not an index of this version, is cut or changed (its checksum says so), or its numbers do not
    /// add up: the index is made again, never trusted.
    pub fn load(path: &Path) -> Option<Self> {
        let bytes = std::fs::read(path).ok()?;
        let (body, tail) = bytes.split_at(bytes.len().checked_sub(8)?);
        if fnv(body) != u64::from_le_bytes(tail.try_into().ok()?) || body.len() < 28 || &body[..4] != MAGIC {
            return None;
        }
        let dims = u32::from_le_bytes(body[4..8].try_into().ok()?) as usize;
        let count = usize::try_from(u64::from_le_bytes(body[8..16].try_into().ok()?)).ok()?;
        let generation = i64::from_le_bytes(body[16..24].try_into().ok()?);
        let root_count = u32::from_le_bytes(body[24..28].try_into().ok()?) as usize;
        if dims == 0 || dims > 65_536 || root_count > usize::from(u16::MAX) {
            return None;
        }
        let words = dims.div_ceil(64);
        let mut at = 28usize;
        let mut roots = Vec::with_capacity(root_count);
        for _ in 0..root_count {
            let len = u16::from_le_bytes(body.get(at..at + 2)?.try_into().ok()?) as usize;
            at += 2;
            roots.push(String::from_utf8(body.get(at..at + len)?.to_vec()).ok()?);
            at += len;
        }
        // What is left must be exactly the ids, the folders and the codes: checked before anything is allocated for them.
        let rest = body.len().checked_sub(at)?;
        let per_piece = 8usize.checked_add(2)?.checked_add(words.checked_mul(8)?)?;
        if rest != count.checked_mul(per_piece)? {
            return None;
        }
        let ids: Vec<i64> =
            body[at..at + count * 8].as_chunks::<8>().0.iter().map(|c| i64::from_le_bytes(*c)).collect();
        at += count * 8;
        let root_of: Vec<u16> =
            body[at..at + count * 2].as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
        at += count * 2;
        if root_of.iter().any(|r| usize::from(*r) >= roots.len().max(1) && count > 0) {
            return None;
        }
        let codes: Vec<u64> = body[at..].as_chunks::<8>().0.iter().map(|c| u64::from_le_bytes(*c)).collect();
        Some(Self { generation, dims, words, roots, ids, root_of, codes })
    }
}

/// The name of the file for one tenant's index of one model: a hash, so no name from outside is ever part of a path.
pub fn file_name(tenant: &str, model: &str) -> String {
    format!("documents-{:016x}.zvi", fnv(format!("{tenant}\0{model}").as_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn index_of(vectors: &[(i64, u16, Vec<f32>)], roots: &[&str]) -> KnowledgeIndex {
        let dims = vectors[0].2.len();
        let words = dims.div_ceil(64);
        let mut index = KnowledgeIndex {
            generation: 7,
            dims,
            words,
            roots: roots.iter().map(|r| r.to_string()).collect(),
            ids: Vec::new(),
            root_of: Vec::new(),
            codes: Vec::new(),
        };
        for (id, root, vector) in vectors {
            index.ids.push(*id);
            index.root_of.push(*root);
            index.codes.extend(code_of(vector));
        }
        index
    }

    #[test]
    fn a_code_is_the_sign_of_each_number_and_pads_to_whole_words() {
        let code = code_of(&[1.0, -1.0, 0.5, 0.0, -0.1]);
        assert_eq!(code, vec![0b00101]);
        assert_eq!(code_of(&vec![1.0; 65]).len(), 2);
        assert_eq!(code_of(&vec![1.0; 65])[1], 1);
    }

    #[test]
    fn the_closest_codes_come_first_and_only_visible_folders_are_looked_at() {
        let near = vec![1.0, 1.0, 1.0, 1.0];
        let mid = vec![1.0, 1.0, -1.0, -1.0];
        let far = vec![-1.0, -1.0, -1.0, -1.0];
        let index =
            index_of(&[(1, 0, far.clone()), (2, 0, near.clone()), (3, 1, near.clone()), (4, 0, mid)], &["a", "b"]);
        let a_only = index.mask(&["a".to_string()]);
        assert_eq!(index.count_in(&a_only), 3);
        assert_eq!(
            index.candidates(&a_only, &near, 2),
            vec![2, 4],
            "closest first, and the other folder's piece is not offered"
        );
        let both = index.mask(&["a".to_string(), "b".to_string()]);
        assert_eq!(index.candidates(&both, &near, 10).len(), 4);
        assert!(index.candidates(&a_only, &[1.0, 1.0], 5).is_empty(), "a question of another size matches nothing");
        assert!(index.candidates(&index.mask(&[]), &near, 5).is_empty());
    }

    #[test]
    fn an_index_survives_a_file_and_a_damaged_or_odd_file_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("index").join("one.zvi");
        let index = index_of(&[(10, 0, vec![1.0; 130]), (11, 1, vec![-1.0; 130])], &["a", "b"]);
        index.save(&path).unwrap();
        let loaded = KnowledgeIndex::load(&path).expect("it reads back");
        assert_eq!((loaded.generation, loaded.dims(), loaded.len()), (7, 130, 2));
        assert_eq!(loaded.candidates(&loaded.mask(&["a".into(), "b".into()]), &vec![1.0; 130], 1), vec![10]);

        // Every change to a byte, and every cut, is noticed.
        let good = std::fs::read(&path).unwrap();
        for at in [0usize, 5, 20, 40, good.len() / 2, good.len() - 1] {
            let mut bad = good.clone();
            bad[at] ^= 0x55;
            std::fs::write(&path, &bad).unwrap();
            assert!(KnowledgeIndex::load(&path).is_none(), "a change at byte {at} was accepted");
        }
        for cut in [0usize, 3, 27, good.len() - 1] {
            std::fs::write(&path, &good[..cut]).unwrap();
            assert!(KnowledgeIndex::load(&path).is_none(), "a file cut to {cut} bytes was accepted");
        }
        assert!(KnowledgeIndex::load(&dir.path().join("missing.zvi")).is_none());
        // A header that promises far more than the file holds is refused before anything is allocated for it.
        let mut lie = good[..good.len() - 8].to_vec();
        lie[8..16].copy_from_slice(&u64::MAX.to_le_bytes());
        let checksum = fnv(&lie);
        lie.extend_from_slice(&checksum.to_le_bytes());
        std::fs::write(&path, &lie).unwrap();
        assert!(KnowledgeIndex::load(&path).is_none());
    }

    #[test]
    fn the_file_name_is_a_hash_that_depends_on_the_tenant_and_the_model_only() {
        assert_eq!(file_name("local", "m"), file_name("local", "m"));
        assert_ne!(file_name("local", "m"), file_name("other", "m"));
        assert_ne!(file_name("local", "m"), file_name("local", "n"));
        assert_ne!(file_name("a", "bc"), file_name("ab", "c"), "the pieces are not just joined");
        assert!(!file_name("../../etc", "../x").contains('/'));
    }
}
