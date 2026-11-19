//! Skills in the Agent Skills format (ADR-0025): a folder with `SKILL.md` and files, content-addressed in the library, loaded on
//! demand by an agent's run, with scripts run only in the sandbox.

pub mod frontmatter;
pub mod library;
pub mod tools;

pub use frontmatter::{SkillMeta, parse_skill_md};
pub use library::{GcReport, SkillError, SkillLibrary, content_hash};
pub use tools::skill_tools;
