//! `zorix settings`: show and change the settings of the program that are kept in the database ([`AppSettings`]).

use std::path::Path;

use zorix_core::{AppSettings, Clock, RetentionMode, RetentionReport, SettingsStore, SystemClock, TenantId};
use zorix_store::{DataDirLock, SqliteRetention, SqliteSettingsStore, database_path};

use crate::app::CliError;
use crate::cli::{RetentionCommand, RetentionRunArgs, SettingsCommand, SettingsSetArgs};
use crate::follow::{Exit, Printer};

/// Run one `settings` command. Changing a setting holds the data-folder lock, like every command that writes.
pub async fn settings(
    home: &Path,
    tenant: &TenantId,
    printer: Printer,
    command: SettingsCommand,
) -> Result<Exit, CliError> {
    match command {
        SettingsCommand::Show => {
            let store = SqliteSettingsStore::open(&database_path(home))?;
            let current = store.settings(tenant).await?;
            if printer.json {
                println!("{}", serde_json::to_string(&current).map_err(|e| CliError(e.to_string()))?);
            } else {
                for (key, value) in lines(&current) {
                    println!("{key:<20} {value}");
                }
            }
            Ok(Exit::Done)
        }
        SettingsCommand::Set(args) => {
            let _lock = DataDirLock::acquire(home)?;
            let store = SqliteSettingsStore::open(&database_path(home))?;
            let changed = apply(store.settings(tenant).await?, &args)?;
            store.put_settings(tenant, &changed).await?;
            let shown = lines(&changed).into_iter().find(|(key, _)| *key == args.key).map(|(_, value)| value);
            eprintln!("{} is now {}.", args.key, shown.unwrap_or_default());
            Ok(Exit::Done)
        }
    }
}

/// Run `zorix retention`.
pub async fn retention(
    home: &Path,
    tenant: &TenantId,
    printer: Printer,
    command: RetentionCommand,
) -> Result<Exit, CliError> {
    let (dry_run, write) = match command {
        RetentionCommand::Show => (true, false),
        RetentionCommand::Run(RetentionRunArgs { dry_run }) => (dry_run, !dry_run),
    };
    // Changing anything holds the data-folder lock; looking does not.
    let _lock = if write { Some(DataDirLock::acquire(home)?) } else { None };
    let database = database_path(home);
    let settings = SqliteSettingsStore::open(&database)?.load(tenant)?;
    let store = SqliteRetention::open(&database)?;
    let Some(report) = store.pass(tenant, &settings, SystemClock.now(), dry_run).await? else {
        if printer.json {
            println!("{}", serde_json::json!({"enabled": false}));
        } else {
            eprintln!(
                "Retention is off: runs are kept for ever. Turn it on with `zorix settings set retention.days 90`."
            );
        }
        return Ok(Exit::Done);
    };
    if printer.json {
        println!("{}", serde_json::json!({"enabled": true, "dry_run": dry_run, "report": report}));
    } else {
        println!("{}", describe(&report, &settings, dry_run));
    }
    Ok(Exit::Done)
}

fn describe(report: &RetentionReport, settings: &AppSettings, dry_run: bool) -> String {
    let days = settings.retention.days.unwrap_or(0);
    let verb = match (dry_run, settings.retention.mode) {
        (true, RetentionMode::Compact) => "would compact",
        (true, RetentionMode::Delete) => "would delete",
        (false, RetentionMode::Compact) => "compacted",
        (false, RetentionMode::Delete) => "deleted",
    };
    if report.runs == 0 {
        return format!("No run is older than {days} days and finished, so there is nothing to do.");
    }
    let mut text = format!(
        "{} {verb} {} run(s) older than {days} days: {} event(s), {} step(s), {} approval(s) and {} recall(s), about {:.1} MB.",
        if dry_run { "It" } else { "ZORIX" },
        report.runs,
        report.events,
        report.tool_calls,
        report.approvals,
        report.snapshots,
        report.bytes as f64 / 1_048_576.0
    );
    if report.kept_because_referenced > 0 {
        text.push_str(&format!(
            "\n{} run(s) are referred to by a task, a workflow run or a report, so they are compacted, not deleted.",
            report.kept_because_referenced
        ));
    }
    text
}

fn on_off(text: &str, key: &str) -> Result<bool, CliError> {
    match text.to_ascii_lowercase().as_str() {
        "on" | "true" | "yes" => Ok(true),
        "off" | "false" | "no" => Ok(false),
        _ => Err(CliError(format!("{key} takes on or off, not “{text}”."))),
    }
}

fn is_off(text: &str) -> bool {
    matches!(text.to_ascii_lowercase().as_str(), "off" | "none" | "never")
}

/// `current` with one setting changed.
fn apply(mut current: AppSettings, args: &SettingsSetArgs) -> Result<AppSettings, CliError> {
    let value = args.value.trim();
    match args.key.as_str() {
        "context.summaries" => current.context.summaries = on_off(value, &args.key)?,
        "knowledge.ocr" => current.knowledge.ocr = on_off(value, &args.key)?,
        "retention.days" => {
            current.retention.days = if is_off(value) {
                None
            } else {
                Some(value.parse::<u32>().map_err(|_| {
                    CliError("retention.days takes a number of days, or off to keep runs for ever.".into())
                })?)
            };
        }
        "retention.mode" => {
            current.retention.mode = match value.to_ascii_lowercase().as_str() {
                "compact" => RetentionMode::Compact,
                "delete" => RetentionMode::Delete,
                _ => return Err(CliError("retention.mode is compact or delete.".into())),
            };
        }
        "telemetry.endpoint" => {
            current.telemetry.endpoint = if is_off(value) { None } else { Some(value.to_string()) };
        }
        other => {
            return Err(CliError(format!(
                "There is no setting “{other}”. The keys are context.summaries, retention.days, retention.mode, telemetry.endpoint and knowledge.ocr."
            )));
        }
    }
    current.validate()?;
    Ok(current)
}

fn lines(settings: &AppSettings) -> Vec<(&'static str, String)> {
    let on = |value: bool| if value { "on" } else { "off" }.to_string();
    vec![
        ("context.summaries", on(settings.context.summaries)),
        ("retention.days", settings.retention.days.map_or_else(|| "off".to_string(), |d| d.to_string())),
        (
            "retention.mode",
            match settings.retention.mode {
                RetentionMode::Compact => "compact",
                RetentionMode::Delete => "delete",
            }
            .to_string(),
        ),
        ("telemetry.endpoint", settings.telemetry.endpoint.clone().unwrap_or_else(|| "off".into())),
        ("knowledge.ocr", on(settings.knowledge.ocr)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(key: &str, value: &str) -> SettingsSetArgs {
        SettingsSetArgs { key: key.into(), value: value.into() }
    }

    #[test]
    fn each_key_changes_its_setting_and_a_bad_value_or_key_says_what_to_use() {
        let start = AppSettings::default();
        assert_eq!(apply(start.clone(), &set("retention.days", "90")).unwrap().retention.days, Some(90));
        assert_eq!(apply(start.clone(), &set("retention.days", "off")).unwrap().retention.days, None);
        assert_eq!(
            apply(start.clone(), &set("retention.mode", "delete")).unwrap().retention.mode,
            RetentionMode::Delete
        );
        assert!(apply(start.clone(), &set("knowledge.ocr", "on")).unwrap().knowledge.ocr);
        assert!(!apply(start.clone(), &set("context.summaries", "off")).unwrap().context.summaries);
        let with_endpoint = apply(start.clone(), &set("telemetry.endpoint", "http://127.0.0.1:4318")).unwrap();
        assert_eq!(with_endpoint.telemetry.endpoint.as_deref(), Some("http://127.0.0.1:4318"));
        for (key, value) in [
            ("retention.days", "0"),
            ("retention.days", "soon"),
            ("retention.mode", "shred"),
            ("knowledge.ocr", "maybe"),
            ("telemetry.endpoint", "collector.example.com"),
            ("nothing.here", "on"),
        ] {
            assert!(apply(start.clone(), &set(key, value)).is_err(), "{key} {value}");
        }
    }
}
