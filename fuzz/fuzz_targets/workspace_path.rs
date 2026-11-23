//! A path a model asks a file tool for: any text, no panic, and an accepted path never climbs out or names a reserved folder.
#![no_main]
use libfuzzer_sys::fuzz_target;
use zorix_tools::workspace::RelativePath;

fuzz_target!(|text: &str| {
    if let Ok(path) = RelativePath::parse(text) {
        let resource = path.resource();
        let inside = resource.strip_prefix("workspace:").expect("a resource names the workspace");
        // A name may hold dots (`a..b`); a part that is only `..` climbs.
        assert!(inside.split('/').all(|part| part != ".."), "{text:?} became {resource:?}");
        assert!(!inside.contains('\\') && !inside.contains(':'), "{text:?} became {resource:?}");
        assert!(!inside.to_ascii_lowercase().starts_with(".zorix"), "{text:?} reaches the reserved folder");
    }
});
