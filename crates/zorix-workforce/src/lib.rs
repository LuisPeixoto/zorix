//! The ZORIX workforce (ADR-0038, ADR-0039): workers over agents, in departments, with a level of autonomy and a weekly budget that
//! shape the runs they start, and a virtual manager that plans the week and writes the report.
//!
//! [`WorkforceService`] holds the rules; [`org_tools`] gives a manager the `org.team` tool; [`WorkforceTicker`] keeps plans, reports,
//! schedules and budget notices moving while the desktop app or `zorix serve` is awake. The service implements
//! [`zorix_projects::RunShaper`], so a task's run starts under its worker's level and limits.

pub mod rules;
pub mod runtime;
pub mod service;
pub mod tools;

pub use service::{
    ActivityLine, Figures, NewWorker, Notice, OpenTask, Overview, SettingsFields, WorkerDetail, WorkerFields,
    WorkerView, WorkforceService, YouView,
};
pub use tools::{WorkforceTicker, org_tools};
