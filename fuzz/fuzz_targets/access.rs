//! The access patterns people write for agents (`read:docs/*`, `repo:acme/web:write`): any text, no panic.
#![no_main]
use libfuzzer_sys::fuzz_target;

fuzz_target!(|entries: Vec<String>| {
    let _ = zorix_core::access::compile_access(&entries);
    for entry in &entries {
        let _ = zorix_core::permission::validate_tool_pattern(entry);
        let _ = zorix_core::permission::validate_workspace_pattern(entry);
        let _ = zorix_core::permission::validate_host_pattern(entry);
    }
});
