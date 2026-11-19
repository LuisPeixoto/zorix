//! What a person does with agents: find one, import and export its Markdown file, compare versions, and build the execution for
//! a request. The rules live in `zorix-agents` and the store; this module only puts them together.

use std::collections::HashMap;

use zorix_agents::diff::{DiffLine, diff};
use zorix_agents::{AgentAsk, ModelResolver, format, prepare};
use zorix_core::{Agent, AgentSlug, AgentSpec, AgentStore, AgentVersion, Clock, NewExecution, SystemClock};

use crate::{Hub, HubError};

/// What importing a file did.
#[derive(Clone, Debug)]
pub struct Imported {
    /// The agent.
    pub agent: Agent,
    /// Its newest version after the import.
    pub version: AgentVersion,
    /// The agent did not exist before.
    pub created: bool,
    /// A new version was written (false when the file matched the newest version exactly).
    pub changed: bool,
}

impl Hub {
    /// One agent by name, with its newest version.
    pub async fn agent(&self, slug: &str) -> Result<(Agent, AgentVersion), HubError> {
        let slug = AgentSlug::new(slug).map_err(HubError)?;
        let agent = self
            .agents
            .agent_by_slug(&self.tenant, &slug)
            .await?
            .ok_or_else(|| HubError(format!("There is no agent called {slug}. `zorix agent list` shows them.")))?;
        let version = self
            .agents
            .version(&self.tenant, agent.id, agent.current_version)
            .await?
            .ok_or_else(|| HubError(format!("The newest version of {slug} is missing.")))?;
        Ok((agent, version))
    }

    /// One version of an agent (`None` means the newest).
    pub async fn agent_version(&self, slug: &str, version: Option<u32>) -> Result<(Agent, AgentVersion), HubError> {
        let (agent, newest) = self.agent(slug).await?;
        match version {
            None => Ok((agent, newest)),
            Some(n) => {
                let found = self
                    .agents
                    .version(&self.tenant, agent.id, n)
                    .await?
                    .ok_or_else(|| HubError(format!("{} has no version {n}.", agent.slug)))?;
                Ok((agent, found))
            }
        }
    }

    /// Read an agent file. Skills it names without a hash take the library's current version. A new slug makes a new agent;
    /// an existing one gets a new version (or nothing, when the file matches its newest version). `rename` imports it under
    /// another slug instead.
    pub async fn import_agent(
        &self,
        file: &str,
        rename: Option<&str>,
        note: Option<String>,
    ) -> Result<Imported, HubError> {
        let parsed = format::parse(file).map_err(HubError)?;
        let mut current: HashMap<String, String> = HashMap::new();
        for skill in parsed.skills.iter().filter(|s| s.hash.is_none()) {
            if let Some(record) = self.agents.skill(&self.tenant, &skill.name).await? {
                current.insert(skill.name.clone(), record.hash);
            }
        }
        let mut spec = parsed.into_spec(|name| current.get(name).cloned()).map_err(HubError)?;
        if let Some(rename) = rename {
            spec.slug = AgentSlug::new(rename).map_err(HubError)?;
        }
        self.save_spec(spec, note).await
    }

    /// Create the agent of a spec, or add a version to the one with its slug.
    pub async fn save_spec(&self, spec: AgentSpec, note: Option<String>) -> Result<Imported, HubError> {
        let now = SystemClock.now();
        match self.agents.agent_by_slug(&self.tenant, &spec.slug).await? {
            None => {
                let (agent, version) = self.agents.create_agent(&self.tenant, spec, note, now).await?;
                Ok(Imported { agent, version, created: true, changed: true })
            }
            Some(agent) => {
                let (version, changed) = self.agents.save_version(&self.tenant, agent.id, spec, note, now).await?;
                let agent = self.agents.agent(&self.tenant, agent.id).await?.unwrap_or(agent);
                Ok(Imported { agent, version, created: false, changed })
            }
        }
    }

    /// An agent version as a Markdown file.
    pub async fn export_agent(&self, slug: &str, version: Option<u32>) -> Result<String, HubError> {
        let (_, found) = self.agent_version(slug, version).await?;
        Ok(format::render(&found.spec, Some(found.version)))
    }

    /// What changed between two versions of an agent.
    pub async fn diff_agent(&self, slug: &str, from: u32, to: u32) -> Result<Vec<DiffLine>, HubError> {
        let (_, a) = self.agent_version(slug, Some(from)).await?;
        let (_, b) = self.agent_version(slug, Some(to)).await?;
        Ok(diff(&a.spec, &b.spec))
    }

    /// Write a new version that copies an older one.
    pub async fn rollback_agent(&self, slug: &str, to: u32) -> Result<AgentVersion, HubError> {
        let (agent, _) = self.agent(slug).await?;
        Ok(self.agents.rollback(&self.tenant, agent.id, to, SystemClock.now()).await?)
    }

    /// The execution for a request to an agent (`version` `None` means the newest).
    pub async fn prepare_agent_run(
        &self,
        slug: &str,
        version: Option<u32>,
        ask: AgentAsk,
        resolver: &dyn ModelResolver,
    ) -> Result<NewExecution, HubError> {
        let (agent, found) = self.agent_version(slug, version).await?;
        prepare(&agent, &found, ask, &*self.agents, resolver).await.map_err(|e| HubError(e.to_string()))
    }
}
