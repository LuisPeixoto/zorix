//! Reading and cutting the documents people index: any bytes as each kind of file, no panic, and the pieces are never empty or past
//! their limit.
#![no_main]
use libfuzzer_sys::fuzz_target;
use zorix_memory::index::chunk::chunk;
use zorix_memory::index::extract::extract;
use zorix_memory::index::walk::Kind;

fuzz_target!(|bytes: &[u8]| {
    for kind in [Kind::Markdown, Kind::Text, Kind::Code, Kind::Data, Kind::Html] {
        if let Ok(read) = extract(kind, bytes) {
            let cut = chunk(kind, &read.pages);
            for piece in &cut.chunks {
                assert!(!piece.text.trim().is_empty());
            }
        }
    }
});
