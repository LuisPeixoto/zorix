//! Contract tests for the workforce store: settings default and persist, a worker is written with its events and a revision, a
//! second hire of one agent is refused, a stale write is not applied, reports and history are kept and filtered, and tenants never mix.

use zorix_core::project::Date;
use zorix_core::{
    AgentSlug, AutonomyLevel, ExecutionId, OrgEventDraft, OrgEventKind, OrgReport, OrgReportId, OrgSettings, OrgStore,
    ReportKind, ReportStatus, TenantId, Timestamp, Worker,
};
use zorix_store::SqliteOrgStore;

const NOW: Timestamp = Timestamp(1_000);

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).unwrap()
}

fn slug(text: &str) -> AgentSlug {
    AgentSlug::new(text).unwrap()
}

fn event(kind: OrgEventKind, worker: &str, text: &str, at: i64) -> OrgEventDraft {
    OrgEventDraft {
        kind,
        worker: Some(slug(worker)),
        text: text.into(),
        by: "you".into(),
        run: None,
        at: Timestamp(at),
    }
}

fn worker(who: &TenantId, agent: &str) -> Worker {
    Worker::new(who.clone(), slug(agent), "Role", "operations", NOW)
}

#[tokio::test]
async fn the_settings_start_as_the_defaults_and_are_kept() {
    let store = SqliteOrgStore::open_in_memory().unwrap();
    let me = tenant("local");
    let first = store.settings(&me).await.unwrap();
    assert_eq!(first, OrgSettings::default());
    assert_eq!(first.default_level, AutonomyLevel::Ask);

    let mut changed = first.clone();
    changed.weekly_budget_micros = Some(120_000_000);
    changed.departments.truncate(2);
    store.put_settings(&me, &changed, &[]).await.unwrap();
    assert_eq!(store.settings(&me).await.unwrap(), changed);
    assert_eq!(store.settings(&tenant("other")).await.unwrap(), OrgSettings::default(), "another tenant has its own");
}

#[tokio::test]
async fn a_worker_is_hired_once_and_written_with_its_events() {
    let store = SqliteOrgStore::open_in_memory().unwrap();
    let me = tenant("local");
    let atlas = worker(&me, "atlas");
    let hired = event(OrgEventKind::Hired, "atlas", "Atlas was hired", 1_000);
    assert!(store.put_worker(&me, &atlas, None, &[hired]).await.unwrap());
    let stored = store.worker(&me, &slug("atlas")).await.unwrap().unwrap();
    assert_eq!(stored.revision, 1);
    assert_eq!(store.workers(&me).await.unwrap().len(), 1);
    let again = store.put_worker(&me, &atlas, None, &[]).await;
    assert!(again.unwrap_err().to_string().contains("already a worker"));
    let history = store.events(&me, Some(&slug("atlas")), 10).await.unwrap();
    assert_eq!((history.len(), history[0].seq, history[0].kind), (1, 1, OrgEventKind::Hired));
}

#[tokio::test]
async fn a_write_goes_through_only_for_the_revision_it_read() {
    let store = SqliteOrgStore::open_in_memory().unwrap();
    let me = tenant("local");
    assert!(store.put_worker(&me, &worker(&me, "atlas"), None, &[]).await.unwrap());
    let mut mine = store.worker(&me, &slug("atlas")).await.unwrap().unwrap();
    let mut theirs = mine.clone();

    mine.role = "Manager".into();
    let kept = event(OrgEventKind::Edited, "atlas", "kept", 2_000);
    assert!(store.put_worker(&me, &mine, Some(mine.revision), &[kept]).await.unwrap());
    theirs.role = "Lost".into();
    let lost = event(OrgEventKind::Edited, "atlas", "lost", 3_000);
    assert!(!store.put_worker(&me, &theirs, Some(theirs.revision), &[lost]).await.unwrap(), "stale");

    let now = store.worker(&me, &slug("atlas")).await.unwrap().unwrap();
    assert_eq!((now.role.as_str(), now.revision), ("Manager", 2));
    let texts: Vec<String> = store.events(&me, None, 10).await.unwrap().into_iter().map(|e| e.text).collect();
    assert_eq!(texts, vec!["kept"], "the events of a refused write are not stored");
}

#[tokio::test]
async fn a_worker_can_be_let_go_and_history_is_newest_first_and_filtered() {
    let store = SqliteOrgStore::open_in_memory().unwrap();
    let me = tenant("local");
    for a in ["atlas", "forge"] {
        let e = event(OrgEventKind::Hired, a, &format!("{a} hired"), 1_000);
        store.put_worker(&me, &worker(&me, a), None, &[e]).await.unwrap();
    }
    let fired = event(OrgEventKind::Fired, "forge", "forge let go", 2_000);
    assert!(store.remove_worker(&me, &slug("forge"), &[fired]).await.unwrap());
    assert!(!store.remove_worker(&me, &slug("forge"), &[]).await.unwrap());
    assert_eq!(store.workers(&me).await.unwrap().len(), 1);
    let all: Vec<String> = store.events(&me, None, 10).await.unwrap().into_iter().map(|e| e.text).collect();
    assert_eq!(all, vec!["forge let go", "forge hired", "atlas hired"]);
    assert_eq!(store.events(&me, Some(&slug("atlas")), 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn reports_are_kept_updated_and_listed_newest_first() {
    let store = SqliteOrgStore::open_in_memory().unwrap();
    let me = tenant("local");
    let make = |kind: ReportKind, at: i64| OrgReport {
        id: OrgReportId::new(),
        tenant: me.clone(),
        worker: slug("atlas"),
        kind,
        week: Date::new("2026-11-16").unwrap(),
        run: ExecutionId::new(),
        status: ReportStatus::Running,
        body: String::new(),
        error: None,
        by: "you".into(),
        started_at: Timestamp(at),
        finished_at: None,
    };
    let plan = make(ReportKind::Plan, 1_000);
    let report = make(ReportKind::Report, 2_000);
    store.add_report(&me, &plan).await.unwrap();
    store.add_report(&me, &report).await.unwrap();
    assert_eq!(store.reports(&me, None, true, 10).await.unwrap().len(), 2);

    let mut done = plan.clone();
    done.status = ReportStatus::Done;
    done.body = "Nine tasks.".into();
    done.finished_at = Some(Timestamp(1_500));
    let ended = event(OrgEventKind::PlanDone, "atlas", "plan written", 1_500);
    store.update_report(&me, &done, &[ended]).await.unwrap();
    assert_eq!(store.report(&me, plan.id).await.unwrap().unwrap().body, "Nine tasks.");
    let running = store.reports(&me, None, true, 10).await.unwrap();
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].id, report.id);
    let all = store.reports(&me, Some(&slug("atlas")), false, 10).await.unwrap();
    assert_eq!(all.iter().map(|r| r.id).collect::<Vec<_>>(), vec![report.id, plan.id]);
    assert!(store.reports(&me, Some(&slug("forge")), false, 10).await.unwrap().is_empty());
    assert_eq!(store.events(&me, None, 10).await.unwrap().len(), 1);
}

#[tokio::test]
async fn tenants_never_see_each_others_workers_or_history() {
    let store = SqliteOrgStore::open_in_memory().unwrap();
    let (a, b) = (tenant("a"), tenant("b"));
    let e = event(OrgEventKind::Hired, "atlas", "hired", 1_000);
    store.put_worker(&a, &worker(&a, "atlas"), None, &[e]).await.unwrap();
    assert!(store.workers(&b).await.unwrap().is_empty());
    assert!(store.worker(&b, &slug("atlas")).await.unwrap().is_none());
    assert!(store.events(&b, None, 10).await.unwrap().is_empty());
    // The same agent may be a worker in another tenant.
    assert!(store.put_worker(&b, &worker(&b, "atlas"), None, &[]).await.unwrap());
}

const LEAK: &str = "sk-abcdefghijklmnopqrstuvwxyz0123456789";

/// The history and the reports of the organisation hold text a model wrote; a credential in it is replaced before it is stored
/// (finding F3 of the Phase 11 audit).
#[tokio::test]
async fn a_credential_in_an_event_or_a_report_is_replaced_before_it_is_stored() {
    let store = SqliteOrgStore::open_in_memory().unwrap();
    let me = tenant("local");
    let leaked = event(OrgEventKind::ReportFailed, "atlas", &format!("it failed with key {LEAK} in the url"), 1_000);
    store.put_worker(&me, &worker(&me, "atlas"), None, &[leaked]).await.unwrap();
    let text = store.events(&me, None, 10).await.unwrap().remove(0).text;
    assert!(!text.contains(LEAK) && text.contains("it failed with key"), "{text}");

    let report = OrgReport {
        id: OrgReportId::new(),
        tenant: me.clone(),
        worker: slug("atlas"),
        kind: ReportKind::Report,
        week: Date::new("2026-11-16").unwrap(),
        run: ExecutionId::new(),
        status: ReportStatus::Done,
        body: format!("Used {LEAK} to read the board."),
        error: Some(format!("retry with {LEAK}")),
        by: "you".into(),
        started_at: Timestamp(1_000),
        finished_at: Some(Timestamp(2_000)),
    };
    store.add_report(&me, &report).await.unwrap();
    let kept = store.report(&me, report.id).await.unwrap().unwrap();
    assert!(!kept.body.contains(LEAK) && kept.body.contains("to read the board"), "{}", kept.body);
    assert!(!kept.error.unwrap_or_default().contains(LEAK));
}
