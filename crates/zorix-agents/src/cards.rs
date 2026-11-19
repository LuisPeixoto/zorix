//! A short description of each agent, for lists, pickers and MCP clients.

use serde::Serialize;
use zorix_core::{AgentStore, Result, TenantId};

/// What a list shows about an agent.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct AgentCard {
    /// Its id, which names its memory scope.
    pub id: String,
    /// Its short name.
    pub slug: String,
    /// Its display name.
    pub name: String,
    /// What it does.
    pub description: String,
    /// Its avatar's emoji.
    pub emoji: String,
    /// Its avatar's colour name.
    pub color: String,
    /// The model it uses when none is asked for.
    pub default_model: String,
    /// Its newest version.
    pub version: u32,
    /// The names of its skills.
    pub skills: Vec<String>,
    /// Hidden from pickers.
    pub archived: bool,
}

/// The agents of a tenant with their newest versions, by slug.
pub async fn cards(store: &dyn AgentStore, tenant: &TenantId, include_archived: bool) -> Result<Vec<AgentCard>> {
    let mut out = Vec::new();
    for agent in store.agents(tenant, include_archived).await? {
        let Some(version) = store.version(tenant, agent.id, agent.current_version).await? else { continue };
        let spec = version.spec;
        out.push(AgentCard {
            id: agent.id.to_string(),
            slug: agent.slug.to_string(),
            name: spec.name,
            description: spec.description,
            emoji: spec.avatar.emoji,
            color: spec.avatar.color,
            default_model: spec.models.default,
            version: agent.current_version,
            skills: spec.skills.into_iter().map(|s| s.name).collect(),
            archived: agent.archived,
        });
    }
    Ok(out)
}
