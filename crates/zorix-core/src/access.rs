//! What a person means by `--allow-http`, `--allow-repo` and the like, as grants (ADR-0021). Each flag names a resource; the
//! hub adds the permission on the resource and the right to use the tools that reach it, so one flag is enough.

use crate::permission::{
    Action, Grants, Permission, network_grant, tool_grant, validate_workspace_pattern, workspace_grant,
};

fn tools(patterns: &[&str]) -> Vec<Permission> {
    patterns.iter().map(|p| tool_grant(p).expect("a fixed tool pattern is valid")).collect()
}

/// `name` or `name:write`: the resource and whether writing is meant too.
fn split_write(text: &str) -> (&str, bool) {
    match text.strip_suffix(":write") {
        Some(rest) => (rest, true),
        None => (text, false),
    }
}

fn with_actions(prefix: &str, resource: &str, write: bool) -> Vec<Permission> {
    let name = format!("{prefix}:{resource}");
    let mut grants = vec![Permission::new(Action::Read, name.clone())];
    if write {
        grants.push(Permission::new(Action::Write, name));
    }
    grants
}

fn plain(text: &str, extra: &str) -> bool {
    !text.is_empty()
        && text.len() <= 100
        && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') || extra.contains(c))
}

/// `--allow-http HOST`: reach this host over HTTPS with `http.get`, `http.request` and `webhook.send`.
pub fn http(host: &str) -> Result<Vec<Permission>, String> {
    let mut grants = vec![network_grant(host)?];
    grants.extend(tools(&["http.get", "http.request", "webhook.send"]));
    Ok(grants)
}

/// `--allow-repo [github:|gitlab:]OWNER/REPO[:write]`. Without a forge, both are meant (the connection decides which host
/// answers). `OWNER/*` covers every repository of an owner.
pub fn repo(text: &str) -> Result<Vec<Permission>, String> {
    let (text, write) = split_write(text);
    let (forges, path): (&[&str], &str) = if let Some(rest) = text.strip_prefix("github:") {
        (&["github"], rest)
    } else if let Some(rest) = text.strip_prefix("gitlab:") {
        (&["gitlab"], rest)
    } else {
        (&["github", "gitlab"], text)
    };
    let slashes = path.matches('/').count();
    let ok = plain(path, "/*")
        && (slashes == 1 || (slashes >= 1 && forges == ["gitlab"]))
        && !path.starts_with('/')
        && path.split('/').all(|part| !part.is_empty() && part != "." && part != "..");
    if !ok {
        return Err("use OWNER/REPO (or OWNER/*), for example `acme/api` or `gitlab:group/project:write`".into());
    }
    let mut grants = Vec::new();
    for forge in forges {
        grants.extend(with_actions(forge, path, write));
        grants.extend(tools(&[&format!("{forge}.*")]));
    }
    Ok(grants)
}

/// `--allow-jira KEY[:write]`: a Jira project, or `*`.
pub fn jira(text: &str) -> Result<Vec<Permission>, String> {
    let (key, write) = split_write(text);
    let key = key.to_ascii_uppercase();
    if key != "*" && !(plain(&key, "") && key.chars().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit() || c == '_'))
    {
        return Err("use a Jira project key such as `ENG` or `ENG:write`".into());
    }
    let mut grants = with_actions("jira", &key, write);
    grants.extend(tools(&["jira.*"]));
    Ok(grants)
}

/// `--allow-chat slack:CHANNEL[:write]` or `discord:CHANNEL_ID[:write]`.
pub fn chat(text: &str) -> Result<Vec<Permission>, String> {
    let (text, write) = split_write(text);
    let Some((service, channel)) = text.split_once(':') else {
        return Err("use slack:#channel or discord:<channel id>, for example `slack:#alerts:write`".into());
    };
    if !matches!(service, "slack" | "discord") || !plain(channel, "#*") {
        return Err("use slack:#channel or discord:<channel id>, for example `slack:#alerts:write`".into());
    }
    let mut grants = with_actions(service, channel, write);
    grants.extend(tools(&[&format!("{service}.*")]));
    Ok(grants)
}

/// `--allow-db NAME[:write]`: a database connection.
pub fn db(text: &str) -> Result<Vec<Permission>, String> {
    let (name, write) = split_write(text);
    if name != "*" && !crate::integrations::valid_name(name) {
        return Err("use the name of a database connection, for example `prod` or `prod:write`".into());
    }
    let mut grants = with_actions("db", name, write);
    grants.extend(tools(&["db.*"]));
    Ok(grants)
}

/// `--allow-code LANGUAGE`: run code in the sandbox.
pub fn code(language: &str) -> Result<Vec<Permission>, String> {
    if !matches!(language, "javascript" | "python") {
        return Err("use javascript or python".into());
    }
    let mut grants = vec![Permission::new(Action::Execute, format!("code:{language}"))];
    grants.extend(tools(&["code.*"]));
    Ok(grants)
}

/// One access entry of an agent, in the syntax of the `--allow-…` flags with a prefix naming the family:
/// `http:HOST`, `repo:[github:|gitlab:]OWNER/REPO[:write]`, `jira:KEY[:write]`, `chat:slack:#channel[:write]`,
/// `chat:discord:ID[:write]`, `db:NAME[:write]`, `code:LANGUAGE`, `mcp:SERVER` (every tool of an MCP server), `read:PATTERN`
/// and `write:PATTERN` (workspace paths, relative).
pub fn compile_entry(entry: &str) -> Result<Vec<Permission>, String> {
    let (family, rest) = entry.split_once(':').ok_or_else(|| {
        format!("“{entry}” needs a family before the colon: http, repo, jira, chat, db, code, mcp, read or write.")
    })?;
    match family {
        "http" => http(rest),
        "repo" => repo(rest),
        "jira" => jira(rest),
        "chat" => chat(rest),
        "db" => db(rest),
        "code" => code(rest),
        "mcp" => {
            if !crate::integrations::valid_name(rest) {
                return Err("mcp: takes the name of a server, for example `mcp:github`".into());
            }
            Ok(vec![tool_grant(&format!("mcp.{rest}.*"))?])
        }
        "read" | "write" => {
            validate_workspace_pattern(rest)?;
            let action = if family == "read" { Action::Read } else { Action::Write };
            Ok(vec![workspace_grant(action, rest)?])
        }
        other => {
            Err(format!("“{other}” is not a family of access (http, repo, jira, chat, db, code, mcp, read, write)."))
        }
    }
}

/// Every access entry of an agent as grants. Fails on the first entry that is not valid.
pub fn compile_access(entries: &[String]) -> Result<Grants, String> {
    let mut grants = Grants::none();
    for entry in entries {
        for permission in compile_entry(entry)? {
            grants = grants.with(permission);
        }
    }
    Ok(grants)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn has(grants: &[Permission], action: Action, resource: &str) -> bool {
        grants.iter().any(|g| g.action == action && g.resource == resource)
    }

    #[test]
    fn one_flag_grants_the_resource_and_the_tools_that_reach_it() {
        let grants = http("api.example.com").unwrap();
        assert!(has(&grants, Action::Network, "http:api.example.com"));
        assert!(has(&grants, Action::UseTool, "http.get") && has(&grants, Action::UseTool, "webhook.send"));

        let repo_grants = repo("acme/api").unwrap();
        assert!(
            has(&repo_grants, Action::Read, "github:acme/api") && has(&repo_grants, Action::Read, "gitlab:acme/api")
        );
        assert!(!repo_grants.iter().any(|g| g.action == Action::Write), "read only unless :write");
        let writing = repo("github:acme/*:write").unwrap();
        assert!(has(&writing, Action::Write, "github:acme/*") && !has(&writing, Action::Read, "gitlab:acme/*"));
        assert!(has(&writing, Action::UseTool, "github.*"));

        assert!(has(&jira("eng:write").unwrap(), Action::Write, "jira:ENG"));
        assert!(has(&chat("slack:#alerts:write").unwrap(), Action::Write, "slack:#alerts"));
        assert!(has(&chat("discord:123456789012345678").unwrap(), Action::Read, "discord:123456789012345678"));
        assert!(
            has(&db("prod").unwrap(), Action::Read, "db:prod") && !has(&db("prod").unwrap(), Action::Write, "db:prod")
        );
        assert!(has(&code("python").unwrap(), Action::Execute, "code:python"));
    }

    #[test]
    fn an_agent_access_list_compiles_to_grants_and_a_bad_entry_names_itself() {
        let grants = compile_access(&[
            "http:api.example.com".into(),
            "repo:acme/api:write".into(),
            "db:prod".into(),
            "mcp:github".into(),
            "read:docs/*".into(),
            "code:python".into(),
        ])
        .unwrap();
        assert!(grants.allows(&Permission::new(Action::Network, "http:api.example.com")));
        assert!(grants.allows(&Permission::new(Action::Write, "github:acme/api")));
        assert!(grants.allows(&Permission::use_tool("mcp.github.create_issue")));
        assert!(grants.allows(&Permission::new(Action::Read, "workspace:docs/a.md")));
        assert!(!grants.allows(&Permission::new(Action::Write, "workspace:docs/a.md")));
        for bad in ["nocolon", "ftp:host", "repo:nonsense", "mcp:Bad Name", "read:../etc", "chat:teams:x"] {
            let error = compile_access(&[bad.to_string()]).unwrap_err();
            assert!(!error.is_empty(), "{bad}");
        }
    }

    #[test]
    fn nonsense_is_refused_with_an_example() {
        for bad in ["", "no-slash", "a/b/c", "../x", "a b/c"] {
            assert!(repo(bad).is_err(), "{bad}");
        }
        assert!(repo("group/sub/project").is_err(), "a bare path with two slashes is not an owner/repo");
        assert!(repo("gitlab:group/sub/project").is_ok(), "GitLab has nested groups");
        for bad in ["", "e n g", "ENG-1"] {
            assert!(jira(bad).is_err(), "{bad}");
        }
        assert!(chat("teams:x").is_err() && chat("slack").is_err());
        assert!(db("Prod Server").is_err());
        assert!(code("ruby").is_err());
        assert!(http("bad host").is_err());
        assert!(http("*.example.com").is_err(), "a * is only allowed at the end");
        assert!(http("api.example.*").is_ok());
    }
}
