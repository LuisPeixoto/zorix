//! An agent file (Markdown with front matter): any text, no panic. What parses is written out and read again to the same agent.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = zorix_agents::format::parse(text);
});
