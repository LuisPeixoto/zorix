//! One JSON-RPC message from an MCP server: any text, no panic.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = zorix_mcp::wire::parse(text);
});
