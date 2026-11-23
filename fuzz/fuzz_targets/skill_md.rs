//! A skill's `SKILL.md`: any text, with and without a folder name, no panic.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|text: &str| {
    let _ = zorix_agents::skills::parse_skill_md(text, None);
    let _ = zorix_agents::skills::parse_skill_md(text, Some("a-skill"));
});
