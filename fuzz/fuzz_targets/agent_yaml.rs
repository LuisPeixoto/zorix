//! The hand-written YAML reader of the agent file: any text, no panic and no hang.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = zorix_agents::yaml::parse(text);
});
