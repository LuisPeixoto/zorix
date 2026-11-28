//! Knowledge: indexing folders of documents so their text can be searched and cited (ADR-0017).

pub mod chunk;
pub mod extract;
#[cfg(feature = "ocr")]
pub mod ocr;
#[cfg(feature = "pdf")]
pub mod pdf;
pub mod sync;
pub mod walk;
pub mod watch;

pub use sync::{Indexer, Phase, Progress, SyncReport};

/// Something that reads the text of a picture of a page: the OCR engine, or a stand-in in a test.
pub trait PageReader: Send + Sync {
    /// The text in a picture of `width` by `height` pixels, three bytes (red, green, blue) each.
    fn read(&self, width: u32, height: u32, rgb: &[u8]) -> Result<String, String>;
}
