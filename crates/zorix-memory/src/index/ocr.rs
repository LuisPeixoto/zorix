//! Reading the text of a scanned PDF on this computer (ADR-0048).
//!
//! A scan is a PDF whose pages are pictures. The picture of each page is taken out of the file (a JPEG, or raw samples packed with Flate), turned into
//! pixels and read by an OCR engine whose two small models run here: nothing is sent anywhere. Everything about it is bounded, because a PDF and an image
//! are both hostile input: the number of pages, the size of a picture in pixels (before anything is allocated for it), the time for the whole file, and what the
//! engine may write. A picture that cannot be read (a compression that is not supported, a colour space ZORIX does not know) is said in words and the page is left out.

use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use lopdf::{Document, Object};
use ocrs::{ImageSource, OcrEngine, OcrEngineParams};
use rten::Model;
use zune_core::bytestream::ZCursor;
use zune_core::colorspace::ColorSpace;
use zune_core::options::DecoderOptions;
use zune_jpeg::JpegDecoder;

use super::PageReader;
use super::extract::{Extracted, Page, Unreadable};

/// The most pages of one PDF that are read.
pub const MAX_PAGES: usize = 100;
/// The largest picture that is decoded, in pixels (a page scanned at 600 dpi is about 35 million).
pub const MAX_PIXELS: u64 = 40_000_000;
/// The most characters kept from one page.
pub const MAX_PAGE_CHARS: usize = 60_000;

/// The OCR engine, with its models loaded from files.
pub struct OcrReader {
    engine: Mutex<OcrEngine>,
}

impl OcrReader {
    /// Load the two models. The files must be the ones ZORIX downloaded (their hash is checked when they are fetched, not here).
    pub fn from_files(detection: &Path, recognition: &Path) -> Result<Self, String> {
        let load = |path: &Path| {
            Model::load_file(path).map_err(|e| format!("the model {} could not be read: {e}", path.display()))
        };
        let engine = OcrEngine::new(OcrEngineParams {
            detection_model: Some(load(detection)?),
            recognition_model: Some(load(recognition)?),
            ..OcrEngineParams::default()
        })
        .map_err(|e| format!("the OCR engine could not start: {e}"))?;
        Ok(Self { engine: Mutex::new(engine) })
    }
}

impl PageReader for OcrReader {
    fn read(&self, width: u32, height: u32, rgb: &[u8]) -> Result<String, String> {
        let engine = self.engine.lock().map_err(|_| "the OCR engine stopped working".to_string())?;
        let source =
            ImageSource::from_bytes(rgb, (width, height)).map_err(|e| format!("the picture could not be used: {e}"))?;
        let input = engine.prepare_input(source).map_err(|e| format!("the picture could not be prepared: {e}"))?;
        engine.get_text(&input).map_err(|e| format!("the text could not be read: {e}"))
    }
}

/// The pixels of a picture in a PDF, three bytes each, or why it cannot be read.
fn pixels(doc: &Document, image: &lopdf::xobject::PdfImage<'_>) -> Result<(u32, u32, Vec<u8>), String> {
    let (width, height) = (u64::try_from(image.width).unwrap_or(0), u64::try_from(image.height).unwrap_or(0));
    if width == 0 || height == 0 || width.saturating_mul(height) > MAX_PIXELS {
        return Err(format!("the picture is {width} by {height} pixels, more than the {MAX_PIXELS} that are read"));
    }
    let filters: Vec<&str> = image.filters.as_deref().unwrap_or(&[]).iter().map(String::as_str).collect();
    // A picture that is a JPEG.
    if matches!(filters.as_slice(), ["DCTDecode"] | ["DCT"]) {
        let options = DecoderOptions::default()
            .jpeg_set_out_colorspace(ColorSpace::RGB)
            .set_max_width(width as usize)
            .set_max_height(height as usize);
        let mut decoder = JpegDecoder::new_with_options(ZCursor::new(image.content), options);
        let data = decoder.decode().map_err(|e| format!("the JPEG could not be decoded: {e:?}"))?;
        let (w, h) = decoder.dimensions().ok_or("the JPEG has no size")?;
        return if data.len() == w * h * 3 {
            Ok((w as u32, h as u32, data))
        } else {
            Err("the JPEG is not made of three colours".into())
        };
    }
    // Raw samples, packed with Flate or not at all.
    if !filters.iter().all(|f| matches!(*f, "FlateDecode" | "Fl")) {
        return Err(format!("the picture is compressed with {}, which is not read", filters.join(" and ")));
    }
    let stream = doc
        .get_object(image.id)
        .and_then(Object::as_stream)
        .map_err(|e| format!("the picture could not be found: {e}"))?;
    let raw = if filters.is_empty() {
        stream.content.clone()
    } else {
        stream.decompressed_content().map_err(|e| format!("the picture could not be unpacked: {e}"))?
    };
    let bits = image.bits_per_component.unwrap_or(8);
    let (w, h) = (width as usize, height as usize);
    let space = image.color_space.as_deref().unwrap_or("DeviceGray");
    // A `Decode [1 0]` array turns black and white around.
    let inverted = stream
        .dict
        .get(b"Decode")
        .ok()
        .and_then(|d| d.as_array().ok())
        .is_some_and(|a| a.first().and_then(|v| v.as_float().ok()).is_some_and(|v| v >= 1.0));
    let gray: Vec<u8> = match (space, bits) {
        ("DeviceGray" | "CalGray", 8) if raw.len() >= w * h => {
            raw[..w * h].iter().map(|&v| if inverted { 255 - v } else { v }).collect()
        }
        ("DeviceGray" | "CalGray", 1) => {
            let row = w.div_ceil(8);
            if raw.len() < row * h {
                return Err("the picture has fewer samples than its size says".into());
            }
            let mut out = Vec::with_capacity(w * h);
            for y in 0..h {
                for x in 0..w {
                    let bit = raw[y * row + x / 8] >> (7 - x % 8) & 1;
                    out.push(if (bit == 1) != inverted { 255 } else { 0 });
                }
            }
            out
        }
        ("DeviceRGB" | "CalRGB", 8) if raw.len() >= w * h * 3 => {
            return Ok((w as u32, h as u32, raw[..w * h * 3].to_vec()));
        }
        _ => return Err(format!("the picture is {space} with {bits} bits, which is not read")),
    };
    let mut rgb = Vec::with_capacity(w * h * 3);
    for value in gray {
        rgb.extend_from_slice(&[value, value, value]);
    }
    Ok((w as u32, h as u32, rgb))
}

/// Read the pages of a scanned PDF. Pages with no picture, or one that cannot be read, are left out; when no page gave any text the reason of the
/// first one that failed is the answer.
pub fn read_scan(bytes: &[u8], reader: &dyn PageReader, budget: Duration) -> Result<Extracted, Unreadable> {
    let began = Instant::now();
    let doc = Document::load_mem(bytes).map_err(|e| Unreadable::Fail(format!("the PDF could not be read: {e}")))?;
    let pages = doc.get_pages();
    if pages.len() > MAX_PAGES {
        return Err(Unreadable::Skip(format!(
            "the scan has {} pages, more than the {MAX_PAGES} that are read",
            pages.len()
        )));
    }
    let mut out = Vec::new();
    let mut first_problem: Option<String> = None;
    for (number, id) in pages {
        if began.elapsed() > budget {
            return Err(Unreadable::Skip(format!("the scan took longer than {} seconds to read", budget.as_secs())));
        }
        let images = match doc.get_page_images(id) {
            Ok(images) => images,
            Err(_) => continue,
        };
        // The largest picture on the page is the scan.
        let Some(image) = images.iter().max_by_key(|i| i.width.saturating_mul(i.height)) else { continue };
        let read = pixels(&doc, image).and_then(|(w, h, rgb)| reader.read(w, h, &rgb));
        match read {
            Ok(text) if !text.trim().is_empty() => {
                let text = if text.chars().count() > MAX_PAGE_CHARS {
                    text.chars().take(MAX_PAGE_CHARS).collect()
                } else {
                    text
                };
                out.push(Page { number: Some(number), text });
            }
            Ok(_) => {}
            Err(problem) => {
                first_problem.get_or_insert(format!("page {number}: {problem}"));
            }
        }
    }
    if out.is_empty() {
        return Err(Unreadable::Skip(match first_problem {
            Some(problem) => format!("the scan could not be read ({problem})"),
            None => "the scan has no text that could be read".into(),
        }));
    }
    Ok(Extracted { pages: out })
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use lopdf::{Stream, dictionary};

    use super::*;

    /// A stand-in for the engine: it remembers what it was given and answers with a fixed text.
    struct Recorder {
        seen: Mutex<Vec<(u32, u32, Vec<u8>)>>,
        text: String,
    }

    impl Recorder {
        fn saying(text: &str) -> Self {
            Self { seen: Mutex::new(Vec::new()), text: text.into() }
        }
        fn calls(&self) -> usize {
            self.seen.lock().unwrap().len()
        }
    }

    impl PageReader for Recorder {
        fn read(&self, width: u32, height: u32, rgb: &[u8]) -> Result<String, String> {
            self.seen.lock().unwrap().push((width, height, rgb.to_vec()));
            Ok(self.text.clone())
        }
    }

    /// One picture on a page, as the PDF stores it.
    #[derive(Clone)]
    struct Picture {
        width: i64,
        height: i64,
        bits: i64,
        space: &'static str,
        filter: Option<&'static str>,
        inverted: bool,
        data: Vec<u8>,
    }

    impl Picture {
        fn gray(width: i64, height: i64, data: Vec<u8>) -> Self {
            Self { width, height, bits: 8, space: "DeviceGray", filter: None, inverted: false, data }
        }
    }

    /// A PDF with one page per entry: a picture, or an empty page.
    fn pdf(pages: &[Option<Picture>]) -> Vec<u8> {
        let mut doc = Document::with_version("1.5");
        let pages_id = doc.new_object_id();
        let mut kids = Vec::new();
        for picture in pages {
            let mut resources = lopdf::Dictionary::new();
            if let Some(p) = picture {
                let mut dict = dictionary! {
                    "Type" => "XObject", "Subtype" => "Image", "Width" => p.width, "Height" => p.height,
                    "ColorSpace" => p.space, "BitsPerComponent" => p.bits,
                };
                // Flate is done by the PDF library itself, so the stream really is packed.
                let packed = p.filter == Some("FlateDecode");
                if let Some(filter) = p.filter.filter(|_| !packed) {
                    dict.set("Filter", filter);
                }
                if p.inverted {
                    dict.set("Decode", vec![Object::Integer(1), Object::Integer(0)]);
                }
                let mut stream = Stream::new(dict, p.data.clone());
                if packed {
                    stream.compress().unwrap();
                }
                let image = doc.add_object(stream);
                resources.set("XObject", dictionary! { "Im0" => image });
            }
            let content = doc.add_object(Stream::new(dictionary! {}, b"q 100 0 0 100 0 0 cm /Im0 Do Q".to_vec()));
            let page = doc.add_object(dictionary! {
                "Type" => "Page", "Parent" => pages_id, "Contents" => content, "Resources" => resources,
                "MediaBox" => vec![0.into(), 0.into(), 100.into(), 100.into()],
            });
            kids.push(Object::Reference(page));
        }
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! { "Type" => "Pages", "Kids" => kids, "Count" => pages.len() as i64 }),
        );
        let catalog = doc.add_object(dictionary! { "Type" => "Catalog", "Pages" => pages_id });
        doc.trailer.set("Root", catalog);
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn read(pdf: &[u8], reader: &Recorder) -> Result<Extracted, Unreadable> {
        read_scan(pdf, reader, Duration::from_secs(30))
    }

    fn skipped(result: Result<Extracted, Unreadable>) -> String {
        match result {
            Err(Unreadable::Skip(why)) => why,
            Err(Unreadable::Fail(why)) => panic!("failed, not skipped: {why}"),
            Ok(_) => panic!("it was read"),
        }
    }

    #[test]
    fn a_gray_picture_is_given_to_the_reader_as_three_colours() {
        let reader = Recorder::saying("hello");
        let out = read(&pdf(&[Some(Picture::gray(2, 2, vec![0, 255, 128, 7]))]), &reader).unwrap();
        assert_eq!(out.pages.len(), 1);
        assert_eq!((out.pages[0].number, out.pages[0].text.as_str()), (Some(1), "hello"));
        let seen = reader.seen.lock().unwrap();
        assert_eq!(seen[0], (2, 2, vec![0, 0, 0, 255, 255, 255, 128, 128, 128, 7, 7, 7]));
    }

    #[test]
    fn a_picture_packed_with_flate_is_unpacked() {
        // Rows of one grey each, so the packing really makes it smaller (the library leaves a stream alone when it would not).
        let data: Vec<u8> = (0..256).map(|i| (i / 16) as u8 * 10).collect();
        let reader = Recorder::saying("x");
        let picture = Picture { filter: Some("FlateDecode"), ..Picture::gray(16, 16, data.clone()) };
        let bytes = pdf(&[Some(picture)]);
        let doc = Document::load_mem(&bytes).unwrap();
        let packed = doc.objects.values().any(|o| {
            o.as_stream().is_ok_and(|s| {
                s.dict.get(b"Subtype").is_ok_and(|v| v.as_name().is_ok_and(|n| n == b"Image"))
                    && s.dict.get(b"Filter").is_ok_and(|f| f.as_name().is_ok_and(|n| n == b"FlateDecode"))
            })
        });
        assert!(packed, "the stream was packed");
        read(&bytes, &reader).unwrap();
        let seen = reader.seen.lock().unwrap();
        let reds: Vec<u8> = seen[0].2.chunks(3).map(|p| p[0]).collect();
        assert_eq!(reds, data);
    }

    #[test]
    fn a_flate_bomb_cannot_grow_past_what_the_size_allows() {
        // A picture that says 8 by 8 but unpacks to far more is cut to its size and read as that.
        let reader = Recorder::saying("x");
        let picture = Picture { filter: Some("FlateDecode"), ..Picture::gray(8, 8, vec![7; 4_000_000]) };
        read(&pdf(&[Some(picture)]), &reader).unwrap();
        assert_eq!(reader.seen.lock().unwrap()[0].2.len(), 8 * 8 * 3);
    }

    #[test]
    fn colour_pictures_keep_their_colours() {
        let reader = Recorder::saying("x");
        let picture = Picture { space: "DeviceRGB", ..Picture::gray(2, 1, vec![10, 20, 30, 40, 50, 60]) };
        read(&pdf(&[Some(picture)]), &reader).unwrap();
        assert_eq!(reader.seen.lock().unwrap()[0], (2, 1, vec![10, 20, 30, 40, 50, 60]));
    }

    #[test]
    fn one_bit_pictures_are_unpacked_and_the_decode_array_turns_them_around() {
        // Eight pixels, 1 0 1 0 1 0 1 0, and a ninth row byte is not needed: width 8 is one byte per row.
        let plain = Picture { bits: 1, ..Picture::gray(8, 1, vec![0b1010_1010]) };
        let reader = Recorder::saying("x");
        read(&pdf(&[Some(plain.clone())]), &reader).unwrap();
        let first: Vec<u8> = reader.seen.lock().unwrap()[0].2.chunks(3).map(|p| p[0]).collect();
        assert_eq!(first, vec![255, 0, 255, 0, 255, 0, 255, 0]);

        let reader = Recorder::saying("x");
        read(&pdf(&[Some(Picture { inverted: true, ..plain })]), &reader).unwrap();
        let turned: Vec<u8> = reader.seen.lock().unwrap()[0].2.chunks(3).map(|p| p[0]).collect();
        assert_eq!(turned, vec![0, 255, 0, 255, 0, 255, 0, 255]);
    }

    #[test]
    fn a_row_that_ends_in_the_middle_of_a_byte_is_padded() {
        // Width 3: the row is one byte and only its top three bits count.
        let reader = Recorder::saying("x");
        let picture = Picture { bits: 1, ..Picture::gray(3, 2, vec![0b1010_0000, 0b0100_0000]) };
        read(&pdf(&[Some(picture)]), &reader).unwrap();
        let reds: Vec<u8> = reader.seen.lock().unwrap()[0].2.chunks(3).map(|p| p[0]).collect();
        assert_eq!(reds, vec![255, 0, 255, 0, 255, 0]);
    }

    #[test]
    fn a_jpeg_is_decoded() {
        let jpeg = include_bytes!("../../tests/fixtures/page.jpg").to_vec();
        let picture = Picture { space: "DeviceRGB", filter: Some("DCTDecode"), ..Picture::gray(32, 16, jpeg) };
        let reader = Recorder::saying("x");
        read(&pdf(&[Some(picture)]), &reader).unwrap();
        let seen = reader.seen.lock().unwrap();
        let (w, h, rgb) = &seen[0];
        assert_eq!((*w, *h, rgb.len()), (32, 16, 32 * 16 * 3));
        let at = |x: usize, y: usize| rgb[(y * 32 + x) * 3];
        assert!(at(6, 6) < 60, "the square is dark: {}", at(6, 6));
        assert!(at(0, 0) > 200, "the page is light: {}", at(0, 0));
    }

    #[test]
    fn a_jpeg_that_is_cut_off_is_left_out_with_a_reason_and_does_not_panic() {
        let mut jpeg = include_bytes!("../../tests/fixtures/page.jpg").to_vec();
        jpeg.truncate(jpeg.len() / 3);
        let picture = Picture { space: "DeviceRGB", filter: Some("DCTDecode"), ..Picture::gray(32, 16, jpeg) };
        let reader = Recorder::saying("x");
        let why = skipped(read(&pdf(&[Some(picture)]), &reader));
        assert!(why.starts_with(zorix_core::SCAN_UNREAD_NOTE) && why.contains("page 1"), "{why}");
        assert_eq!(reader.calls(), 0);
    }

    #[test]
    fn a_picture_that_says_it_is_huge_is_refused_before_anything_is_allocated() {
        let reader = Recorder::saying("x");
        let picture = Picture::gray(100_000, 100_000, vec![0; 16]);
        let why = skipped(read(&pdf(&[Some(picture)]), &reader));
        assert!(why.contains("100000 by 100000") && why.starts_with("the scan"), "{why}");
        assert_eq!(reader.calls(), 0);
    }

    #[test]
    fn a_picture_with_fewer_samples_than_its_size_says_is_refused() {
        let reader = Recorder::saying("x");
        let why = skipped(read(&pdf(&[Some(Picture::gray(50, 50, vec![0; 10]))]), &reader));
        assert!(why.contains("page 1"), "{why}");
        assert_eq!(reader.calls(), 0);
    }

    #[test]
    fn a_compression_that_is_not_supported_is_named() {
        let reader = Recorder::saying("x");
        let picture = Picture { bits: 1, filter: Some("CCITTFaxDecode"), ..Picture::gray(8, 1, vec![0]) };
        let why = skipped(read(&pdf(&[Some(picture)]), &reader));
        assert!(why.contains("CCITTFaxDecode") && why.contains("not read"), "{why}");
    }

    #[test]
    fn a_colour_space_that_is_not_known_is_named() {
        let reader = Recorder::saying("x");
        let picture = Picture { space: "DeviceCMYK", ..Picture::gray(1, 1, vec![0, 0, 0, 0]) };
        let why = skipped(read(&pdf(&[Some(picture)]), &reader));
        assert!(why.contains("DeviceCMYK"), "{why}");
    }

    #[test]
    fn pages_without_a_picture_are_passed_over_and_the_page_numbers_are_kept() {
        let reader = Recorder::saying("second page");
        let pages = [None, Some(Picture::gray(1, 1, vec![9])), None];
        let out = read(&pdf(&pages), &reader).unwrap();
        assert_eq!(out.pages.len(), 1);
        assert_eq!(out.pages[0].number, Some(2));
    }

    #[test]
    fn a_scan_with_nothing_to_read_says_so() {
        let reader = Recorder::saying("   \n ");
        let why = skipped(read(&pdf(&[Some(Picture::gray(1, 1, vec![9]))]), &reader));
        assert_eq!(why, "the scan has no text that could be read");
        let why = skipped(read(&pdf(&[None]), &reader));
        assert_eq!(why, "the scan has no text that could be read");
    }

    #[test]
    fn one_bad_page_does_not_stop_the_others() {
        let reader = Recorder::saying("fine");
        let pages = [Some(Picture::gray(50, 50, vec![0; 3])), Some(Picture::gray(1, 1, vec![9]))];
        let out = read(&pdf(&pages), &reader).unwrap();
        assert_eq!(out.pages.iter().map(|p| p.number).collect::<Vec<_>>(), vec![Some(2)]);
    }

    #[test]
    fn a_pdf_with_too_many_pages_is_left_out() {
        let pages = vec![None; MAX_PAGES + 1];
        let reader = Recorder::saying("x");
        let why = skipped(read(&pdf(&pages), &reader));
        assert!(why.starts_with("the scan has 101 pages"), "{why}");
        assert_eq!(reader.calls(), 0);
    }

    #[test]
    fn the_time_for_a_file_is_limited() {
        let reader = Recorder::saying("x");
        let bytes = pdf(&[Some(Picture::gray(1, 1, vec![9])), Some(Picture::gray(1, 1, vec![9]))]);
        let why = skipped(read_scan(&bytes, &reader, Duration::ZERO));
        assert!(why.contains("took longer"), "{why}");
        assert_eq!(reader.calls(), 0);
    }

    #[test]
    fn the_text_of_one_page_is_capped() {
        let reader = Recorder::saying(&"a".repeat(MAX_PAGE_CHARS + 500));
        let out = read(&pdf(&[Some(Picture::gray(1, 1, vec![9]))]), &reader).unwrap();
        assert_eq!(out.pages[0].text.chars().count(), MAX_PAGE_CHARS);
    }

    #[test]
    fn bytes_that_are_not_a_pdf_are_a_failure_and_not_a_panic() {
        let reader = Recorder::saying("x");
        assert!(matches!(read(b"%PDF-1.4 nothing here", &reader), Err(Unreadable::Fail(_))));
        assert!(matches!(read(&[], &reader), Err(Unreadable::Fail(_))));
    }
}
