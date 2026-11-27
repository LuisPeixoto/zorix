//! The settings store: defaults, persistence across opens, refusal of values that make no sense, and tenants kept apart.

use zorix_core::{
    AppSettings, KnowledgeSettings, RetentionMode, RetentionSettings, SettingsStore, TelemetrySettings, TenantId,
};
use zorix_store::SqliteSettingsStore;

fn tenant(name: &str) -> TenantId {
    TenantId::new(name).unwrap()
}

#[tokio::test]
async fn settings_start_as_the_defaults_and_are_kept_for_the_next_program() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("zorix.sqlite");
    let me = tenant("local");
    {
        let store = SqliteSettingsStore::open(&path).unwrap();
        assert_eq!(store.settings(&me).await.unwrap(), AppSettings::default());
        let changed = AppSettings {
            retention: RetentionSettings { days: Some(90), mode: RetentionMode::Delete },
            telemetry: TelemetrySettings { endpoint: Some("https://otel.example.com:4318".into()) },
            knowledge: KnowledgeSettings { ocr: true },
            ..AppSettings::default()
        };
        store.put_settings(&me, &changed).await.unwrap();
        assert_eq!(store.settings(&me).await.unwrap(), changed);
    }
    // Another program opens the same file later and finds them.
    let again = SqliteSettingsStore::open(&path).unwrap();
    let found = again.settings(&me).await.unwrap();
    assert_eq!(found.retention.days, Some(90));
    assert_eq!(found.retention.mode, RetentionMode::Delete);
    assert!(found.knowledge.ocr);
}

#[tokio::test]
async fn a_value_that_makes_no_sense_is_refused_and_changes_nothing() {
    let store = SqliteSettingsStore::open_in_memory().unwrap();
    let me = tenant("local");
    let good =
        AppSettings { retention: RetentionSettings { days: Some(30), ..Default::default() }, ..Default::default() };
    store.put_settings(&me, &good).await.unwrap();
    let bad =
        AppSettings { retention: RetentionSettings { days: Some(0), ..Default::default() }, ..Default::default() };
    assert!(store.put_settings(&me, &bad).await.is_err());
    let bad_address =
        AppSettings { telemetry: TelemetrySettings { endpoint: Some("not an address".into()) }, ..Default::default() };
    assert!(store.put_settings(&me, &bad_address).await.is_err());
    assert_eq!(store.settings(&me).await.unwrap(), good, "the refused ones left the saved settings as they were");
}

#[tokio::test]
async fn tenants_have_their_own_settings() {
    let store = SqliteSettingsStore::open_in_memory().unwrap();
    let on = AppSettings { knowledge: KnowledgeSettings { ocr: true }, ..Default::default() };
    store.put_settings(&tenant("a"), &on).await.unwrap();
    assert!(store.settings(&tenant("a")).await.unwrap().knowledge.ocr);
    assert!(!store.settings(&tenant("b")).await.unwrap().knowledge.ocr);
}
