//! The config files of other MCP clients that ZORIX imports servers from: any text, for each client, no panic.
#![no_main]
use libfuzzer_sys::fuzz_target;
use zorix_mcp::import::{Source, parse};

fuzz_target!(|text: &str| {
    for source in [Source::Claude, Source::Vscode, Source::Cursor] {
        let _ = parse(source, text);
    }
});
