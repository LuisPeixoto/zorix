//! What changed between two versions of an agent, line by line, over their Markdown form.

use similar::{ChangeTag, TextDiff};
use zorix_core::AgentSpec;

use crate::format;

/// One line of a diff.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    /// Same, added or removed.
    pub kind: Change,
    /// The line, without its newline.
    pub text: String,
}

/// What happened to a line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    /// In both.
    Same,
    /// Only in the newer.
    Added,
    /// Only in the older.
    Removed,
}

/// The lines that differ between two versions, with a little unchanged context kept between them by the caller if wanted.
pub fn diff(old: &AgentSpec, new: &AgentSpec) -> Vec<DiffLine> {
    let (a, b) = (format::render(old, None), format::render(new, None));
    TextDiff::from_lines(&a, &b)
        .iter_all_changes()
        .map(|change| DiffLine {
            kind: match change.tag() {
                ChangeTag::Equal => Change::Same,
                ChangeTag::Insert => Change::Added,
                ChangeTag::Delete => Change::Removed,
            },
            text: change.value().trim_end_matches('\n').to_string(),
        })
        .collect()
}

/// How many lines were added and removed.
pub fn summarize(lines: &[DiffLine]) -> (usize, usize) {
    let count = |kind| lines.iter().filter(|l| l.kind == kind).count();
    (count(Change::Added), count(Change::Removed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use zorix_core::AgentSlug;

    fn spec(instructions: &str) -> AgentSpec {
        let mut spec = AgentSpec::new(AgentSlug::new("a").unwrap(), "A", "demo");
        spec.instructions = instructions.into();
        spec
    }

    #[test]
    fn the_same_agent_has_no_changes_and_an_edit_shows_exactly_what_changed() {
        let a = spec("Be brief.");
        assert_eq!(summarize(&diff(&a, &a)), (0, 0));
        let mut b = spec("Be brief.\nBe kind.");
        b.tools = vec!["math.*".into()];
        let lines = diff(&a, &b);
        let added: Vec<&str> = lines.iter().filter(|l| l.kind == Change::Added).map(|l| l.text.as_str()).collect();
        assert!(added.contains(&"Be kind.") && added.iter().any(|l| l.contains("math.*")), "{added:?}");
        assert!(lines.iter().filter(|l| l.kind == Change::Removed).any(|l| l.text == "tools: []"));
    }
}
