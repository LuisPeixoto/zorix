//! The small pure rules of projects: where a card lands in a lane, what a task asks its agent, and what a link points at.

use serde::Serialize;
use zorix_core::{Project, Task, TaskState};

/// The gap left between ranks, so a card can be placed between two others without moving them.
pub const RANK_STEP: i64 = 1_000;

/// The rank for a card placed at `index` among `ranks` (sorted, without the card itself), or none when two neighbours have no
/// room between them and the lane must be numbered again.
pub fn rank_at(ranks: &[i64], index: usize) -> Option<i64> {
    match (index.checked_sub(1).and_then(|i| ranks.get(i)), ranks.get(index)) {
        (None, None) => Some(RANK_STEP),
        (None, Some(first)) => Some(first - RANK_STEP),
        (Some(last), None) => Some(last + RANK_STEP),
        (Some(before), Some(after)) => (after - before >= 2).then(|| before + (after - before) / 2),
    }
}

/// Ranks for a lane of `count` cards, evenly spaced.
pub fn renumber(count: usize) -> Vec<i64> {
    (1..=count as i64).map(|n| n * RANK_STEP).collect()
}

/// Cut text to a number of characters, with an ellipsis.
pub fn clip(text: &str, max: usize) -> String {
    let text = text.trim();
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// A title for a run's final answer: its first line.
pub fn answer_title(key: &str, text: &str) -> String {
    let line = text.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty());
    match line {
        Some(line) => clip(line, 80),
        None => format!("Answer for {key}"),
    }
}

/// What a task asks the agent or team that has it.
pub fn task_prompt(project: &Project, task: &Task, siblings: &[Task], note: Option<&str>) -> String {
    let mut out =
        format!("You have the task {}: {}\nProject: {} ({}).\n", task.key, task.title, project.name, project.key);
    if !project.description.trim().is_empty() {
        out.push_str(&format!("About the project: {}\n", clip(&project.description, 600)));
    }
    if !task.description.trim().is_empty() {
        out.push_str(&format!("\n{}\n", task.description.trim()));
    }
    if let Some(due) = &task.due {
        out.push_str(&format!("\nIt is due on {due}.\n"));
    }
    let needs: Vec<String> = task
        .depends_on
        .iter()
        .filter_map(|key| siblings.iter().find(|t| &t.key == key))
        .map(|t| format!("- {}: {}", t.key, t.title))
        .collect();
    if !needs.is_empty() {
        out.push_str(&format!("\nIt builds on tasks that are done:\n{}\n", needs.join("\n")));
    }
    if !task.links.is_empty() {
        let links: Vec<String> = task.links.iter().map(|l| format!("- {}", l.url)).collect();
        out.push_str(&format!("\nLinks:\n{}\n", links.join("\n")));
    }
    if let Some(note) = note.map(str::trim).filter(|n| !n.is_empty()) {
        out.push_str(&format!("\nA person looked at your earlier work and asked for this change: {note}\n"));
    }
    out.push_str(
        "\nWhen you can, report how far you are with task.report_progress, and put what you produce with task.add_deliverable. \
         Your final answer becomes the deliverable a person reviews, so make it complete.",
    );
    out
}

/// What the manager is asked when nobody says more.
pub fn manager_prompt(project: &Project, ask: Option<&str>) -> String {
    let ask = ask.map(str::trim).filter(|a| !a.is_empty());
    let mut out = format!(
        "You manage the project {} ({}). Read its tasks with tasks.list and tasks.get.\n",
        project.name, project.key
    );
    match ask {
        Some(ask) => out.push_str(&format!("\n{ask}\n")),
        None => out.push_str(
            "\nPlan it: say what is late or blocked, what should be added, and who should have what. Change the project with tasks.create, \
             tasks.update, tasks.move and tasks.comment. A person approves every change.\n",
        ),
    }
    out.push_str("\nFinish with a short summary of what you did and what is left for the person.");
    out
}

/// What a link on a task points at, when ZORIX can tell.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LinkKind {
    /// A pull request on GitHub.
    GithubPull {
        /// `owner/repo`.
        repo: String,
        /// Its number.
        number: u64,
    },
    /// A merge request on GitLab.
    GitlabPull {
        /// `group/project`.
        repo: String,
        /// Its number.
        number: u64,
    },
    /// Anything else.
    Other,
}

impl LinkKind {
    /// Read an address.
    pub fn of(url: &str) -> LinkKind {
        let Ok(parsed) = url::Url::parse(url) else {
            return LinkKind::Other;
        };
        let host = parsed.host_str().unwrap_or_default();
        let parts: Vec<&str> =
            parsed.path_segments().map(|p| p.filter(|s| !s.is_empty()).collect()).unwrap_or_default();
        if host == "github.com"
            && parts.len() >= 4
            && parts[2] == "pull"
            && let Ok(number) = parts[3].parse()
        {
            return LinkKind::GithubPull { repo: format!("{}/{}", parts[0], parts[1]), number };
        }
        if host.contains("gitlab")
            && let Some(at) = parts.iter().position(|p| *p == "-")
            && parts.get(at + 1) == Some(&"merge_requests")
            && at >= 2
            && let Some(number) = parts.get(at + 2).and_then(|n| n.parse().ok())
        {
            return LinkKind::GitlabPull { repo: parts[..at].join("/"), number };
        }
        LinkKind::Other
    }
}

/// How many tasks of a project are in a lane (for the project card).
pub fn count(tasks: &[Task], state: TaskState) -> usize {
    tasks.iter().filter(|t| t.state == state).count()
}

#[cfg(test)]
mod tests {
    use zorix_core::{ProjectId, ProjectKey, TaskKey, TenantId, Timestamp};

    use super::*;

    #[test]
    fn a_card_lands_between_its_neighbours_and_the_lane_is_numbered_again_when_there_is_no_room() {
        assert_eq!(rank_at(&[], 0), Some(RANK_STEP));
        assert_eq!(rank_at(&[1000, 2000], 0), Some(0));
        assert_eq!(rank_at(&[1000, 2000], 1), Some(1500));
        assert_eq!(rank_at(&[1000, 2000], 2), Some(3000));
        assert_eq!(rank_at(&[1000, 1001], 1), None, "no integer fits between 1000 and 1001");
        assert_eq!(renumber(3), vec![1000, 2000, 3000]);
    }

    #[test]
    fn links_are_read_for_pull_requests_on_github_and_gitlab() {
        assert_eq!(
            LinkKind::of("https://github.com/acme/api/pull/482"),
            LinkKind::GithubPull { repo: "acme/api".into(), number: 482 }
        );
        assert_eq!(
            LinkKind::of("https://github.com/acme/api/pull/482/files"),
            LinkKind::GithubPull { repo: "acme/api".into(), number: 482 }
        );
        assert_eq!(
            LinkKind::of("https://gitlab.com/group/sub/proj/-/merge_requests/7"),
            LinkKind::GitlabPull { repo: "group/sub/proj".into(), number: 7 }
        );
        assert_eq!(LinkKind::of("https://github.com/acme/api/issues/3"), LinkKind::Other);
        assert_eq!(LinkKind::of("not a link"), LinkKind::Other);
    }

    #[test]
    fn the_prompt_names_the_task_and_carries_the_note_and_what_it_builds_on() {
        let project = Project::new(TenantId::local(), ProjectKey::new("OPS").unwrap(), "Q4 Operations", Timestamp(1));
        let key = |n| TaskKey::new(project.key.clone(), n).unwrap();
        let mut done = Task::new(TenantId::local(), project.id, key(1), "Index the folder", Timestamp(1));
        done.state = TaskState::Done;
        let mut task = Task::new(TenantId::local(), ProjectId::new(), key(2), "Write the runbook", Timestamp(1));
        task.description = "Cover certificate renewals.".into();
        task.depends_on = vec![key(1)];
        let text = task_prompt(&project, &task, &[done], Some("Add the Friday renewal."));
        for needle in [
            "OPS-2: Write the runbook",
            "Q4 Operations",
            "Cover certificate renewals.",
            "OPS-1: Index the folder",
            "Add the Friday renewal.",
            "task.report_progress",
        ] {
            assert!(text.contains(needle), "{needle}");
        }
        assert!(!task_prompt(&project, &task, &[], None).contains("asked for this change"));
    }

    #[test]
    fn an_answer_is_titled_by_its_first_line() {
        assert_eq!(answer_title("OPS-1", "\n## Renewal runbook\nBody"), "Renewal runbook");
        assert_eq!(answer_title("OPS-1", "  "), "Answer for OPS-1");
        assert!(answer_title("OPS-1", &"x".repeat(200)).chars().count() <= 80);
    }
}
