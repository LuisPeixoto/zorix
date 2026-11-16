//! `zorix models`, `zorix provider …` and `zorix route …`.

use std::io::{IsTerminal, Read};
use std::path::Path;

use tokio_util::sync::CancellationToken;
use zorix_core::Secret;
use zorix_providers::secrets::{validate_name, validate_value};
use zorix_providers::{KeyStatus, discover_all, provider_statuses};

use crate::app::{CliError, load_config, open_router, secrets};
use crate::cli::{KeyArgs, ModelArgs, ModelsArgs, NameArg, ProviderCommand, RouteCommand};
use crate::follow::{Exit, Printer};

fn key_text(status: &KeyStatus) -> String {
    match status {
        KeyStatus::NotNeeded => "no key needed".into(),
        KeyStatus::Present => "key stored".into(),
        KeyStatus::Missing => "no key (zorix provider set-key)".into(),
        KeyStatus::Unavailable(why) => format!("keychain unavailable: {why}"),
    }
}

/// `zorix models`.
pub async fn models(home: &Path, printer: Printer, args: ModelsArgs) -> Result<Exit, CliError> {
    let router = open_router(home, &ModelArgs::default_demo())?;
    if args.available {
        for found in discover_all(&router, &CancellationToken::new()).await {
            if let Err(why) = &found.models {
                eprintln!("{}: {why}", found.provider);
            }
        }
    }
    let mut list = router.known_models();
    list.sort_by(|a, b| (&a.provider, &a.id).cmp(&(&b.provider, &b.id)));
    if printer.json {
        println!("{}", serde_json::to_string(&list)?);
        return Ok(Exit::Done);
    }
    for m in &list {
        let price = m.price.map_or("price unknown".to_string(), |p| format!("${}/${} per M tokens", p.input, p.output));
        println!(
            "{:<44} {:>9} ctx  {}{}  {}",
            format!("{}/{}", m.provider, m.id),
            m.context_window,
            if m.tools { "tools " } else { "" },
            if m.vision { "vision" } else { "" },
            price
        );
    }
    Ok(Exit::Done)
}

fn read_key(args: &KeyArgs) -> Result<String, CliError> {
    if args.stdin {
        let mut text = String::new();
        std::io::stdin().take(8192).read_to_string(&mut text)?;
        return Ok(text.lines().next().unwrap_or("").trim().to_string());
    }
    if !std::io::stdin().is_terminal() {
        return Err(CliError("There is no terminal to ask on. Pipe the key and add --stdin.".into()));
    }
    rpassword::prompt_password(format!("Key for {} (hidden): ", args.name)).map_err(CliError::from)
}

fn known(home: &Path, name: &str) -> Result<zorix_providers::ProvidersConfig, CliError> {
    validate_name(name).map_err(|e| CliError(e.to_string()))?;
    let config = load_config(home)?;
    if !config.instances().iter().any(|p| p.name == name) {
        let names: Vec<String> = config.instances().into_iter().map(|p| p.name).collect();
        return Err(CliError(format!("There is no provider called “{name}”. Known: {}.", names.join(", "))));
    }
    Ok(config)
}

/// `zorix provider …`.
pub async fn provider(home: &Path, printer: Printer, command: ProviderCommand) -> Result<Exit, CliError> {
    match command {
        ProviderCommand::List => {
            let config = load_config(home)?;
            let statuses = provider_statuses(&config, &secrets(&config));
            if printer.json {
                let rows: Vec<_> = statuses
                    .iter()
                    .map(|s| {
                        serde_json::json!({"name": s.name, "kind": s.kind.as_str(), "url": s.url, "local": s.local,
                        "enabled": s.enabled, "key": key_text(&s.key)})
                    })
                    .collect();
                println!("{}", serde_json::Value::Array(rows));
                return Ok(Exit::Done);
            }
            for s in &statuses {
                println!(
                    "{:<12} {:<12} {:<6} {:<40} {}{}",
                    s.name,
                    s.kind.as_str(),
                    if s.local { "local" } else { "cloud" },
                    s.url,
                    key_text(&s.key),
                    if s.enabled { "" } else { "  (off)" }
                );
            }
            Ok(Exit::Done)
        }
        ProviderCommand::SetKey(args) => {
            let config = known(home, &args.name)?;
            let value = read_key(&args)?;
            validate_value(&value).map_err(|e| CliError(e.to_string()))?;
            let store = secrets(&config);
            store.set(&args.name, &Secret::new(value)).map_err(|e| CliError(e.to_string()))?;
            eprintln!("Stored the key for {} in {}.", args.name, store.describe());
            Ok(Exit::Done)
        }
        ProviderCommand::RemoveKey(NameArg { name }) => {
            let config = known(home, &name)?;
            let removed = secrets(&config).remove(&name).map_err(|e| CliError(e.to_string()))?;
            eprintln!(
                "{}",
                if removed { format!("Removed the key for {name}.") } else { format!("There was no key for {name}.") }
            );
            Ok(Exit::Done)
        }
        ProviderCommand::Test(NameArg { name }) => {
            known(home, &name)?;
            let router = open_router(home, &ModelArgs::default_demo())?;
            let found = discover_all(&router, &CancellationToken::new()).await;
            match found.into_iter().find(|d| d.provider == name) {
                Some(d) => match d.models {
                    Ok(models) => {
                        eprintln!("{name}: reachable, {} model(s) listed.", models.len());
                        Ok(Exit::Done)
                    }
                    Err(why) => Err(CliError(format!("{name}: {why}"))),
                },
                None => Err(CliError(format!("{name} is turned off or cannot list its models."))),
            }
        }
    }
}

/// `zorix route list`.
pub fn route(home: &Path, printer: Printer, command: RouteCommand) -> Result<Exit, CliError> {
    let RouteCommand::List = command;
    let config = load_config(home)?;
    if printer.json {
        println!("{}", serde_json::json!({"routes": config.routes, "alias": config.alias}));
        return Ok(Exit::Done);
    }
    for (alias, target) in &config.alias {
        println!("alias  {alias:<14} → {target}");
    }
    for r in &config.routes {
        println!("route  {:<14} → {}", r.name, r.candidates.join(", "));
    }
    if config.alias.is_empty() && config.routes.is_empty() {
        eprintln!("No routes or aliases. Add them to {}.", zorix_providers::config_path(home).display());
    }
    Ok(Exit::Done)
}
