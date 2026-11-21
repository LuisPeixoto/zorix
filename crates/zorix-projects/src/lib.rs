//! ZORIX projects (ADR-0036, ADR-0037): tasks that start ordinary runs, a manager that plans through tools that always ask, the
//! deliverables of a task and the history of a project.
//!
//! [`ProjectService`] holds the rules; [`project_tools`] gives agents the `tasks.*` and `task.*` tools; [`ProjectTicker`] keeps tasks
//! moving while the desktop app or `zorix serve` is awake.

pub mod links;
pub mod rules;
pub mod service;
pub mod tools;

pub use links::{HubPullReader, PullReader, PullState};
pub use rules::LinkKind;
pub use service::{
    Board, Counts, DeliverableRow, LinkState, ManagerRun, Move, NewProject, NewTask, ProjectCard, ProjectFields,
    ProjectService, RUN_GOING, TaskDetail, TaskFields, TaskView,
};
pub use tools::{ProjectTicker, project_tools};
