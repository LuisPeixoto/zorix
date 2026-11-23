//! A workflow file someone sends: any text, no panic, and what parses and validates is written out and read again to the same file.
#![no_main]
use libfuzzer_sys::fuzz_target;
use zorix_workflows::service::WorkflowFile;

fuzz_target!(|text: &str| {
    if let Ok(file) = WorkflowFile::parse(text) {
        let _ = file.workflow.validate();
        let again = WorkflowFile::parse(&file.render()).expect("what was written reads back");
        assert_eq!(again, file);
    }
});
