//! The server-sent-events reader that every streaming provider and MCP over HTTP use: any bytes, cut into chunks anywhere, no panic, and
//! the same events whatever the cuts.
#![no_main]
use libfuzzer_sys::fuzz_target;
use zorix_providers::sse::SseParser;

fuzz_target!(|input: (&[u8], Vec<u8>)| {
    let (bytes, cuts) = input;
    let whole = {
        let mut parser = SseParser::new();
        parser.feed(bytes).map(|events| events.len()).ok()
    };
    // The same bytes in pieces whose sizes come from `cuts`.
    let mut parser = SseParser::new();
    let mut total = 0usize;
    let mut failed = false;
    let mut rest = bytes;
    for cut in cuts.iter().map(|c| usize::from(*c) % 17 + 1).chain(std::iter::repeat(64)) {
        if rest.is_empty() {
            break;
        }
        let (head, tail) = rest.split_at(cut.min(rest.len()));
        match parser.feed(head) {
            Ok(events) => total += events.len(),
            Err(_) => {
                failed = true;
                break;
            }
        }
        rest = tail;
    }
    // Cutting the stream anywhere must not change how many events it holds, nor whether a line was too long.
    if let Some(whole) = whole {
        assert!(!failed && whole == total, "cuts changed the result: {whole} events whole, {total} in pieces");
    } else {
        assert!(failed, "a line too long whole must be too long in pieces");
    }
});
