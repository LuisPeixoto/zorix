//! The rules of the workforce that need no store: how a worker's level and budgets shape a run, what a week is worth, and the words
//! of a manager's prompts (ADR-0038, ADR-0039). Everything here is pure, so it is tested without a model or a database.

use zorix_core::{Autonomy, AutonomyLevel, OrgSettings, Task, TaskState, Timestamp, Who, Worker};

/// How near a weekly budget has to be before it is worth a notice: four fifths.
pub const NEAR_NUMERATOR: u64 = 4;
/// The denominator of [`NEAR_NUMERATOR`].
pub const NEAR_DENOMINATOR: u64 = 5;

/// What a run gets from its worker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Shaped {
    /// The level and what always asks.
    pub autonomy: Autonomy,
    /// The most active time, in milliseconds.
    pub max_active_ms: u64,
    /// The most it may cost, in millionths of a dollar.
    pub max_cost_micros: Option<u64>,
    /// A sentence for the run's labels when the level was lowered.
    pub note: Option<String>,
}

fn smaller(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// What is left of a budget this week, or none when there is no budget.
pub fn left(budget: Option<u64>, spent: u64) -> Option<u64> {
    budget.map(|b| b.saturating_sub(spent))
}

/// The level, time and cost a worker's run starts with.
///
/// The worker's weekly budget and the organisation's both only narrow: the run may cost no more than what is left of either (and than
/// what the caller already allowed). When either is spent the run starts at Ask, so nothing changes without a person, and says why.
pub fn shape_run(
    worker: &Worker,
    settings: &OrgSettings,
    worker_spent: u64,
    org_spent: u64,
    asked_cost: Option<u64>,
    asked_active_ms: Option<u64>,
) -> Shaped {
    let worker_left = left(worker.weekly_budget_micros, worker_spent);
    let org_left = left(settings.weekly_budget_micros, org_spent);
    let spent = worker_left == Some(0) || org_left == Some(0);
    let mut level = worker.autonomy;
    let mut note = None;
    if spent && level > AutonomyLevel::Ask {
        level = AutonomyLevel::Ask;
        let whose = if worker_left == Some(0) {
            "its weekly budget is spent"
        } else {
            "the organisation's weekly budget is spent"
        };
        note = Some(format!("{} starts at Ask because {whose}, so it asks before it changes anything.", worker.agent));
    }
    // A spent budget lowers the level instead of stopping the run, so it adds no cost cap of zero.
    let cap = smaller(smaller(asked_cost, worker_left.filter(|n| *n > 0)), org_left.filter(|n| *n > 0));
    Shaped {
        autonomy: Autonomy { level, still_ask: worker.still_ask.clone() },
        max_active_ms: asked_active_ms.map_or(worker.task_time_ms, |a| a.min(worker.task_time_ms)),
        max_cost_micros: cap,
        note,
    }
}

/// Whether `spent` has reached four fifths of `budget`.
pub fn is_near(budget: u64, spent: u64) -> bool {
    budget > 0 && spent.saturating_mul(NEAR_DENOMINATOR) >= budget.saturating_mul(NEAR_NUMERATOR)
}

/// Open tasks an agent has: assigned to it by name and not Done.
pub fn open_for<'a>(tasks: &'a [Task], agent: &zorix_core::AgentSlug) -> impl Iterator<Item = &'a Task> {
    let agent = agent.clone();
    tasks.iter().filter(move |t| {
        t.state != TaskState::Done && matches!(&t.assignee, Some(Who::Agent { slug }) if *slug == agent)
    })
}

/// Tasks an agent finished at or after `since` that had a due date, as `(on time, all)`. A task is on time when it went to Done on or
/// before its due day, in the day-text of the same zone.
pub fn on_time_counts(
    tasks: &[Task],
    agent: &zorix_core::AgentSlug,
    since: Timestamp,
    day_of: impl Fn(Timestamp) -> String,
) -> (u32, u32) {
    let mut on_time = 0;
    let mut all = 0;
    for task in tasks {
        let mine = matches!(&task.assignee, Some(Who::Agent { slug }) if slug == agent);
        let (Some(done), Some(due)) = (task.done_at, task.due.as_ref()) else { continue };
        if !mine || task.state != TaskState::Done || done < since {
            continue;
        }
        all += 1;
        if day_of(done).as_str() <= due.as_str() {
            on_time += 1;
        }
    }
    (on_time, all)
}

/// The prompt that asks a manager to plan the week.
pub fn plan_prompt(manager: &Worker, week: &str, projects: &[String], ask: Option<&str>) -> String {
    let mut text = format!(
        "You are {role} ({agent}). Plan the week that starts on {week}.\n\n\
         1. Call org.team to see who reports to you, what each has open, what each spent this week and how free each one is.\n",
        role = manager.role,
        agent = manager.agent
    );
    if projects.is_empty() {
        text.push_str("2. You manage no project, so you cannot change tasks. Say what you would do and stop.\n");
    } else {
        text.push_str(&format!(
            "2. For each project you manage ({}), call tasks.list. Make the tasks that are missing with tasks.create, give each one an \
             assignee and a due date with tasks.update, and move a task to In progress with tasks.move only when the week should start it.\n",
            projects.join(", ")
        ));
    }
    text.push_str(
        "3. Give work only to people who report to you, in amounts they can do inside their budget and their time for a task. Do not \
         plan more than the week can hold.\n\
         4. Every change you make may ask a person first; that is expected.\n\
         5. Finish with a short plan in Markdown: what each person has this week and why. Under 200 words.",
    );
    if let Some(ask) = ask.map(str::trim).filter(|a| !a.is_empty()) {
        text.push_str(&format!("\n\nThe person who asked adds: {ask}"));
    }
    text
}

/// The prompt that asks a manager for the report that closes the week.
pub fn report_prompt(manager: &Worker, week: &str, projects: &[String], ask: Option<&str>) -> String {
    let mut text = format!(
        "You are {role} ({agent}). Write the report for the week that started on {week}.\n\n\
         1. Call org.team to see who reports to you, what each has open, what each spent this week and how they did.\n",
        role = manager.role,
        agent = manager.agent
    );
    if !projects.is_empty() {
        text.push_str(&format!(
            "2. Call tasks.list for each project you manage ({}) to see what moved this week.\n",
            projects.join(", ")
        ));
    }
    text.push_str(
        "3. Do not change any task. This is a report, not a plan.\n\
         4. Answer in Markdown, under 250 words: what got done, what is late or waiting for a person, what was spent against the budgets, \
         and what you suggest for next week. Say plainly when there is nothing to report.",
    );
    if let Some(ask) = ask.map(str::trim).filter(|a| !a.is_empty()) {
        text.push_str(&format!("\n\nThe person who asked adds: {ask}"));
    }
    text
}

#[cfg(test)]
mod tests {
    use zorix_core::{AgentSlug, Date, ProjectId, TaskKey, TenantId};

    use super::*;

    fn worker(level: AutonomyLevel, budget: Option<u64>) -> Worker {
        let mut w = Worker::new(
            TenantId::local(),
            AgentSlug::new("forge").unwrap(),
            "Engineering lead",
            "engineering",
            Timestamp(1),
        );
        w.autonomy = level;
        w.weekly_budget_micros = budget;
        w.still_ask = vec!["repo:acme/web:write".into()];
        w
    }

    fn settings(budget: Option<u64>) -> OrgSettings {
        OrgSettings { weekly_budget_micros: budget, ..OrgSettings::default() }
    }

    #[test]
    fn a_run_gets_its_workers_level_time_and_what_always_asks() {
        let s = shape_run(&worker(AutonomyLevel::Notify, None), &settings(None), 0, 0, None, None);
        assert_eq!(s.autonomy.level, AutonomyLevel::Notify);
        assert_eq!(s.autonomy.still_ask, vec!["repo:acme/web:write".to_string()]);
        assert_eq!(s.max_active_ms, 2 * 60 * 60 * 1_000);
        assert_eq!((s.max_cost_micros, s.note), (None, None));
    }

    #[test]
    fn what_is_left_of_either_budget_caps_the_cost_and_so_does_what_the_caller_allowed() {
        let w = worker(AutonomyLevel::Budgeted, Some(25_000_000));
        let s = shape_run(&w, &settings(Some(120_000_000)), 19_000_000, 110_000_000, None, None);
        assert_eq!(s.max_cost_micros, Some(6_000_000), "the worker has $6 left");
        let s = shape_run(&w, &settings(Some(120_000_000)), 1_000_000, 118_500_000, None, None);
        assert_eq!(s.max_cost_micros, Some(1_500_000), "the organisation has $1.50 left");
        let s = shape_run(&w, &settings(None), 0, 0, Some(1_000_000), Some(60_000));
        assert_eq!(
            (s.max_cost_micros, s.max_active_ms),
            (Some(1_000_000), 60_000),
            "the smaller of what was asked and what is left"
        );
    }

    #[test]
    fn a_spent_week_starts_the_run_at_ask_and_says_why() {
        let w = worker(AutonomyLevel::Full, Some(25_000_000));
        let s = shape_run(&w, &settings(None), 25_000_000, 25_000_000, None, None);
        assert_eq!(s.autonomy.level, AutonomyLevel::Ask);
        assert!(s.note.as_deref().unwrap().contains("its weekly budget is spent"));
        assert_eq!(s.max_cost_micros, None, "a spent budget lowers the level; it does not cap the run at zero");
        let s = shape_run(&worker(AutonomyLevel::Notify, None), &settings(Some(10_000_000)), 0, 10_000_000, None, None);
        assert_eq!(s.autonomy.level, AutonomyLevel::Ask);
        assert!(s.note.unwrap().contains("organisation's weekly budget is spent"));
        // A worker at Suggest or Ask is not changed by a spent budget.
        let s = shape_run(&worker(AutonomyLevel::Suggest, Some(1)), &settings(None), 5, 5, None, None);
        assert_eq!((s.autonomy.level, s.note), (AutonomyLevel::Suggest, None));
    }

    #[test]
    fn four_fifths_of_a_budget_is_near() {
        assert!(!is_near(100, 79));
        assert!(is_near(100, 80));
        assert!(!is_near(0, 5));
    }

    fn task(project: ProjectId, n: u32, state: TaskState, who: &str, due: Option<&str>, done_day: Option<i64>) -> Task {
        let key = TaskKey::new(zorix_core::ProjectKey::new("OPS").unwrap(), n).unwrap();
        let mut t = Task::new(TenantId::local(), project, key, format!("t{n}"), Timestamp(1));
        t.state = state;
        t.assignee = Some(Who::Agent { slug: AgentSlug::new(who).unwrap() });
        t.due = due.map(|d| Date::new(d).unwrap());
        t.done_at = done_day.map(|d| Timestamp(d * 86_400_000));
        t
    }

    fn day(at: Timestamp) -> String {
        // The test clock counts days from 1970-01-01 in UTC: day N is 2026-11-01 + (N - 20_758).
        let n = at.millis().div_euclid(86_400_000) - 20_758;
        format!("2026-11-{:02}", 1 + n)
    }

    #[test]
    fn open_tasks_are_the_ones_not_done_that_the_agent_has() {
        let p = ProjectId::new();
        let tasks = vec![
            task(p, 1, TaskState::InProgress, "forge", None, None),
            task(p, 2, TaskState::Done, "forge", None, None),
            task(p, 3, TaskState::Backlog, "atlas", None, None),
        ];
        let forge = AgentSlug::new("forge").unwrap();
        assert_eq!(open_for(&tasks, &forge).count(), 1);
    }

    #[test]
    fn a_task_is_on_time_when_done_on_or_before_its_due_day_and_only_recent_dated_ones_count() {
        let p = ProjectId::new();
        let forge = AgentSlug::new("forge").unwrap();
        let tasks = vec![
            task(p, 1, TaskState::Done, "forge", Some("2026-11-05"), Some(20_758 + 4)), // done on the 5th: on time
            task(p, 2, TaskState::Done, "forge", Some("2026-11-05"), Some(20_758 + 5)), // done on the 6th: late
            task(p, 3, TaskState::Done, "forge", None, Some(20_758 + 4)),               // no due date: not counted
            task(p, 4, TaskState::Done, "atlas", Some("2026-11-05"), Some(20_758 + 4)), // somebody else's
            task(p, 5, TaskState::Done, "forge", Some("2026-11-05"), Some(20_758 - 40)), // too long ago
        ];
        assert_eq!(on_time_counts(&tasks, &forge, Timestamp((20_758 - 30) * 86_400_000), day), (1, 2));
    }

    #[test]
    fn the_plan_and_report_prompts_name_the_week_the_tools_and_the_limits() {
        let w = worker(AutonomyLevel::Budgeted, None);
        let plan = plan_prompt(&w, "2026-11-16", &["OPS".into(), "WEB".into()], Some("Focus on the migration."));
        assert!(plan.contains("2026-11-16") && plan.contains("org.team") && plan.contains("OPS, WEB"));
        assert!(plan.contains("Focus on the migration."));
        let none = plan_prompt(&w, "2026-11-16", &[], None);
        assert!(none.contains("manage no project"));
        let report = report_prompt(&w, "2026-11-16", &["OPS".into()], None);
        assert!(report.contains("Do not change any task"));
    }
}
