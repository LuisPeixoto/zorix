//! Permissions and grants.
//!
//! An execution has an explicit set of [`Grants`]. Nothing is allowed unless a grant covers it.

use std::fmt;

use serde::{Deserialize, Serialize};

/// What kind of thing is being done.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    /// Read data.
    Read,
    /// Create or change data.
    Write,
    /// Run something.
    Execute,
    /// Talk to something outside this computer.
    Network,
    /// Use a tool at all. The resource is the tool id.
    UseTool,
}

impl fmt::Display for Action {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Action::Read => "read",
            Action::Write => "write",
            Action::Execute => "execute",
            Action::Network => "network",
            Action::UseTool => "use_tool",
        })
    }
}

/// An action on a resource.
///
/// In a **grant** the resource is a pattern: `*` matches everything, `prefix*` matches anything that starts with
/// `prefix`, anything else must be equal. In a **requirement** (what a tool needs) the resource is concrete.
/// Resources are named with a namespace, for example `workspace:` or `notes:` or a tool id.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct Permission {
    /// The kind of action.
    pub action: Action,
    /// The resource, or the pattern that names several.
    pub resource: String,
}

impl Permission {
    /// A permission for `action` on `resource`.
    pub fn new(action: Action, resource: impl Into<String>) -> Self {
        Self { action, resource: resource.into() }
    }

    /// The permission to use the tool with this id.
    pub fn use_tool(tool_id: &str) -> Self {
        Self::new(Action::UseTool, tool_id)
    }

    /// True when this grant covers `required`.
    pub fn covers(&self, required: &Permission) -> bool {
        self.action == required.action && pattern_matches(&self.resource, &required.resource)
    }
}

impl fmt::Display for Permission {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.action, self.resource)
    }
}

fn pattern_matches(pattern: &str, resource: &str) -> bool {
    match pattern.strip_suffix('*') {
        Some(prefix) => resource.starts_with(prefix),
        None => pattern == resource,
    }
}

/// Check the text of a tool pattern given by a person: a tool id, or a prefix ending in `*`.
pub fn validate_tool_pattern(text: &str) -> Result<(), String> {
    let ok = !text.is_empty()
        && text.len() <= 64
        && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '*'));
    if ok { Ok(()) } else { Err("a tool id has letters, digits, '.', '_', '-' and an optional '*'".into()) }
}

/// Check the text of a workspace path pattern given by a person: relative, with forward slashes, no `..`, such as `*`
/// or `docs/*`.
pub fn validate_workspace_pattern(text: &str) -> Result<(), String> {
    let bad = text.is_empty()
        || text.len() > 512
        || text.starts_with('/')
        || text.contains('\\')
        || text.contains(':')
        || text.split('/').any(|part| part == "..")
        || text.chars().any(char::is_control);
    if bad {
        Err("use a relative pattern with forward slashes, such as `*` or `docs/*`, and no `..`".into())
    } else {
        Ok(())
    }
}

/// The grant that lets an execution use the tools matching `pattern`.
pub fn tool_grant(pattern: &str) -> Result<Permission, String> {
    validate_tool_pattern(pattern)?;
    Ok(Permission::use_tool(pattern))
}

/// The grant that lets file tools do `action` on workspace paths matching `pattern`.
pub fn workspace_grant(action: Action, pattern: &str) -> Result<Permission, String> {
    validate_workspace_pattern(pattern)?;
    Ok(Permission::new(action, format!("workspace:{pattern}")))
}

/// Check a host pattern given by a person for `--allow-http`: a name or address, with `*` only at the end.
pub fn validate_host_pattern(text: &str) -> Result<(), String> {
    let body = text.strip_suffix('*').unwrap_or(text);
    let ok = !text.is_empty()
        && text.len() <= 253
        && !body.contains('*')
        && text.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']' | '_' | '*'));
    if ok {
        Ok(())
    } else {
        Err(format!("“{text}” is not a host name or address (a `*` is only allowed at the end)."))
    }
}

/// A grant to reach a host over HTTP (`network` on `http:<host>`). The host is compared in lowercase.
pub fn network_grant(host_pattern: &str) -> Result<Permission, String> {
    validate_host_pattern(host_pattern)?;
    Ok(Permission::new(Action::Network, format!("http:{}", host_pattern.to_ascii_lowercase())))
}

/// What an execution is allowed to do. The default grants nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Grants(Vec<Permission>);

impl Grants {
    /// No permissions.
    pub fn none() -> Self {
        Self::default()
    }

    /// A set of permissions.
    pub fn new(permissions: Vec<Permission>) -> Self {
        Self(permissions)
    }

    /// Add one permission.
    pub fn with(mut self, permission: Permission) -> Self {
        self.0.push(permission);
        self
    }

    /// The permissions in the set.
    pub fn iter(&self) -> impl Iterator<Item = &Permission> {
        self.0.iter()
    }

    /// True when some grant covers `required`.
    pub fn allows(&self, required: &Permission) -> bool {
        self.0.iter().any(|grant| grant.covers(required))
    }

    /// The members of `required` that no grant covers.
    pub fn missing<'a>(&self, required: &'a [Permission]) -> Vec<&'a Permission> {
        required.iter().filter(|permission| !self.allows(permission)).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_is_allowed_by_default() {
        assert!(!Grants::none().allows(&Permission::use_tool("clock.now")));
    }

    #[test]
    fn an_exact_grant_covers_only_that_resource_and_action() {
        let grants = Grants::none().with(Permission::use_tool("clock.now"));
        assert!(grants.allows(&Permission::use_tool("clock.now")));
        assert!(!grants.allows(&Permission::use_tool("clock.nowhere")));
        assert!(!grants.allows(&Permission::new(Action::Read, "clock.now")));
    }

    #[test]
    fn a_prefix_pattern_covers_everything_below_it() {
        let grants = Grants::none().with(Permission::new(Action::Read, "workspace:*"));
        assert!(grants.allows(&Permission::new(Action::Read, "workspace:notes/a.txt")));
        assert!(!grants.allows(&Permission::new(Action::Write, "workspace:notes/a.txt")));
        assert!(!grants.allows(&Permission::new(Action::Read, "notes:a")));
    }

    #[test]
    fn a_star_covers_every_resource_of_that_action_only() {
        let grants = Grants::none().with(Permission::new(Action::UseTool, "*"));
        assert!(grants.allows(&Permission::use_tool("anything")));
        assert!(!grants.allows(&Permission::new(Action::Network, "example.com")));
    }

    #[test]
    fn missing_lists_what_is_not_covered() {
        let grants = Grants::none().with(Permission::new(Action::Read, "workspace:*"));
        let required = [Permission::new(Action::Read, "workspace:a"), Permission::new(Action::Write, "workspace:a")];
        assert_eq!(grants.missing(&required), vec![&required[1]]);
    }

    #[test]
    fn patterns_given_by_people_are_checked() {
        for good in ["math.evaluate", "files.*", "*", "a_b-c"] {
            assert!(tool_grant(good).is_ok(), "{good}");
        }
        for bad in ["", "bad tool", "a/b", &"x".repeat(65)] {
            assert!(tool_grant(bad).is_err(), "{bad:?}");
        }
        assert_eq!(workspace_grant(Action::Read, "docs/*").unwrap(), Permission::new(Action::Read, "workspace:docs/*"));
        for bad in ["", "/etc/*", "../x", "a/../b", "a\\b", "C:/x", "tab\there"] {
            assert!(workspace_grant(Action::Write, bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn network_grants_name_a_host_and_only_exact_ones_name_a_private_machine() {
        let grants =
            Grants::none().with(network_grant("API.Example.com").unwrap()).with(network_grant("docs.*").unwrap());
        assert!(grants.allows(&Permission::new(Action::Network, "http:api.example.com")));
        assert!(grants.allows(&Permission::new(Action::Network, "http:docs.rs")));
        assert!(!grants.allows(&Permission::new(Action::Network, "http:evil.example.com")));
        assert!(!grants.allows(&Permission::new(Action::Read, "http:api.example.com")), "network is its own action");
        for bad in ["", "a b", "a*b", "*.example.com", "http://x", "x/y", "x?"] {
            assert!(validate_host_pattern(bad).is_err(), "{bad}");
        }
        assert!(
            validate_host_pattern("*").is_ok()
                && validate_host_pattern("127.0.0.1").is_ok()
                && validate_host_pattern("[::1]").is_ok()
        );
    }

    #[test]
    fn grants_serialize_as_a_plain_list() {
        let grants = Grants::none().with(Permission::use_tool("x"));
        assert_eq!(serde_json::to_string(&grants).unwrap(), r#"[{"action":"use_tool","resource":"x"}]"#);
    }
}
