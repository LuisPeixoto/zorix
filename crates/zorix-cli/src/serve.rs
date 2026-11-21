//! `zorix serve`: the workflow host, headless (ADR-0034).
//!
//! It takes the data folder like any other program that runs things, starts the schedules, the event triggers and the runs that
//! were waiting, and stays until Ctrl-C. Webhooks listen only when asked, and on this computer unless an address says otherwise.

use std::path::Path;
use std::sync::Arc;

use zorix_core::{StepKind, TenantId, WorkflowStore};
use zorix_workflows::{DEFAULT_WEBHOOK_ADDRESS, Workflows, parse_address};

use crate::app::{CliError, open_engine, secrets};
use crate::cli::ServeArgs;
use crate::follow::{Exit, Printer};

pub async fn serve(home: &Path, tenant: TenantId, printer: Printer, args: ServeArgs) -> Result<Exit, CliError> {
    let webhook = match (&args.bind, args.webhooks) {
        (Some(address), _) => Some(parse_address(address).map_err(|e| CliError(e.to_string()))?),
        (None, true) => Some(parse_address(DEFAULT_WEBHOOK_ADDRESS).map_err(|e| CliError(e.to_string()))?),
        (None, false) => None,
    };
    let app = open_engine(home, &tenant, &args.model)?;
    let config = crate::app::load_config(home)?;
    let workflows = Workflows::open(home, tenant.clone(), app.hub.clone(), app.engine.clone(), app.router.clone())
        .map_err(|e| CliError(e.to_string()))?;
    let tickers: Vec<Arc<dyn zorix_workflows::host::Ticker>> = vec![zorix_projects::runtime::ticker(&app.projects)];
    let host =
        workflows.start_host_with(secrets(&config), webhook, tickers).await.map_err(|e| CliError(e.to_string()))?;

    let all = workflows.store.workflows(&tenant).await.map_err(|e| CliError(e.to_string()))?;
    let on = all.iter().filter(|w| w.enabled).count();
    let listening = host.webhook_addr();
    if printer.json {
        println!(
            "{}",
            serde_json::json!({"serving": true, "workflows_on": on, "webhooks": listening.map(|a| a.to_string())})
        );
    } else {
        println!(
            "zorix: serving {on} workflow{} (schedules, events and runs that were waiting).",
            if on == 1 { "" } else { "s" }
        );
        match listening {
            Some(address) => {
                let hooks = all
                    .iter()
                    .filter(|w| w.enabled)
                    .any(|w| w.draft.triggers().any(|t| matches!(t.kind, StepKind::Webhook { .. })));
                println!("zorix: webhooks arrive on http://{address}/hooks/<workflow>/<secret>.");
                if hooks {
                    println!("zorix: `zorix workflow webhook <name>` shows a workflow's address.");
                }
            }
            None => println!("zorix: webhooks are off (add --webhooks to receive them)."),
        }
        println!("zorix: press Ctrl-C to stop.");
    }
    let _ = tokio::signal::ctrl_c().await;
    host.shutdown().await;
    app.shutdown().await;
    Ok(Exit::Done)
}
