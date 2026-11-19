//! The starter agents that ship inside ZORIX: a definition, the skills it names and a few evaluation cases each. They are
//! only installed when a person asks (`zorix agent add --template researcher`, or "Add from templates" in the app), and what
//! is installed is an ordinary agent that can be edited, versioned and removed.

/// One file of a starter skill.
pub struct StarterFile {
    /// The path inside the skill's folder.
    pub path: &'static str,
    /// What the file holds.
    pub bytes: &'static [u8],
}

/// A skill that comes with a starter agent.
pub struct StarterSkill {
    /// The skill's name, which is also its folder's name.
    pub name: &'static str,
    /// Its files, `SKILL.md` first.
    pub files: &'static [StarterFile],
}

/// A starter agent.
pub struct Starter {
    /// What to type after `--template`.
    pub id: &'static str,
    /// The agent file (`agent/1`), with skills named but not pinned.
    pub agent: &'static str,
    /// The skills it names.
    pub skills: &'static [StarterSkill],
    /// Evaluation cases, each one a JSON document.
    pub cases: &'static [&'static str],
}

macro_rules! file {
    ($starter:literal, $skill:literal, $path:literal) => {
        StarterFile {
            path: $path,
            bytes: include_bytes!(concat!("../starters/", $starter, "/skills/", $skill, "/", $path)),
        }
    };
}

macro_rules! case {
    ($starter:literal, $name:literal) => {
        include_str!(concat!("../starters/", $starter, "/evals/", $name, ".json"))
    };
}

static ALL: &[Starter] = &[
    Starter {
        id: "researcher",
        agent: include_str!("../starters/researcher/agent.md"),
        skills: &[StarterSkill {
            name: "citing-sources",
            files: &[
                file!("researcher", "citing-sources", "SKILL.md"),
                file!("researcher", "citing-sources", "references/checklist.md"),
            ],
        }],
        cases: &[
            case!("researcher", "cites-its-sources"),
            case!("researcher", "says-when-it-found-nothing"),
            case!("researcher", "stays-short"),
        ],
    },
    Starter {
        id: "code-reviewer",
        agent: include_str!("../starters/code-reviewer/agent.md"),
        skills: &[StarterSkill {
            name: "review-checklist",
            files: &[
                file!("code-reviewer", "review-checklist", "SKILL.md"),
                file!("code-reviewer", "review-checklist", "references/security.md"),
            ],
        }],
        cases: &[
            case!("code-reviewer", "finds-sql-injection"),
            case!("code-reviewer", "leaves-clean-code-alone"),
            case!("code-reviewer", "spots-a-swallowed-error"),
        ],
    },
    Starter {
        id: "data-analyst",
        agent: include_str!("../starters/data-analyst/agent.md"),
        skills: &[StarterSkill {
            name: "csv-profiling",
            files: &[
                file!("data-analyst", "csv-profiling", "SKILL.md"),
                file!("data-analyst", "csv-profiling", "scripts/profile.py"),
            ],
        }],
        cases: &[
            case!("data-analyst", "computes-a-mean"),
            case!("data-analyst", "profiles-pasted-csv"),
            case!("data-analyst", "does-not-change-data"),
        ],
    },
    Starter {
        id: "writer",
        agent: include_str!("../starters/writer/agent.md"),
        skills: &[StarterSkill { name: "plain-style", files: &[file!("writer", "plain-style", "SKILL.md")] }],
        cases: &[
            case!("writer", "cuts-the-filler"),
            case!("writer", "keeps-the-length-asked"),
            case!("writer", "does-not-invent-facts"),
        ],
    },
];

/// Every starter, in the order they are offered.
pub fn all() -> &'static [Starter] {
    ALL
}

/// One starter by its id.
pub fn get(id: &str) -> Option<&'static Starter> {
    ALL.iter().find(|s| s.id == id)
}

impl StarterSkill {
    /// The files as `(path, bytes)` pairs, ready for [`crate::SkillLibrary::import_files`].
    pub fn owned_files(&self) -> Vec<(String, Vec<u8>)> {
        self.files.iter().map(|f| (f.path.to_string(), f.bytes.to_vec())).collect()
    }
}
