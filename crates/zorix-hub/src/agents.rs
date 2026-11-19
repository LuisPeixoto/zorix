//! What a person does with agents: find one, import and export its Markdown file, compare versions, and build the execution for
//! a request. The rules live in `zorix-agents` and the store; this module only puts them together.

use std::collections::HashMap;

use std::path::Path;
use zorix_agents::diff::{DiffLine, diff};
use zorix_agents::{AgentAsk, ModelResolver, format, prepare, starters};

use zorix_core::{
    Agent, AgentSlug, AgentSpec, AgentStore, AgentVersion, Clock, EvalCase, NewExecution, SkillRecord, SystemClock,
};

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

/// A starter agent that ships with ZORIX, as the lists show it.
#[derive(Clone, Debug)]
pub struct TemplateInfo {
    /// What to type after `--template`.
    pub id: String,
    /// The agent's name.
    pub name: String,
    /// What it does.
    pub description: String,
    /// Its avatar's emoji.
    pub emoji: String,
    /// Its avatar's colour name.
    pub color: String,
    /// The skills it comes with.
    pub skills: Vec<String>,
    /// How many evaluation cases it comes with.
    pub cases: usize,
    /// An agent with this id already exists here.
    pub installed: bool,
}

/// What installing a starter did.
#[derive(Clone, Debug)]
pub struct TemplateInstalled {
    /// The agent that was created (or got a new version).
    pub imported: Imported,
    /// The skills that were put in the library.
    pub skills: Vec<SkillRecord>,
    /// The evaluation cases that were added or refreshed.
    pub cases: usize,
}

impl Hub {
    /// The starter agents, with whether each is already here.
    pub async fn templates(&self) -> Result<Vec<TemplateInfo>, HubError> {
        let mut out = Vec::new();
        for starter in starters::all() {
            let parsed = format::parse(starter.agent).map_err(HubError)?;
            let slug = parsed.spec.slug.clone();
            let installed = self.agents.agent_by_slug(&self.tenant, &slug).await?.is_some();
            out.push(TemplateInfo {
                id: starter.id.to_string(),
                name: parsed.spec.name.clone(),
                description: parsed.spec.description.clone(),
                emoji: parsed.spec.avatar.emoji.clone(),
                color: parsed.spec.avatar.color.clone(),
                skills: starter.skills.iter().map(|s| s.name.to_string()).collect(),
                cases: starter.cases.len(),
                installed,
            });
        }
        Ok(out)
    }

    /// Install a starter: its skills go in the library, the agent is created (or gets a new version when one with its slug
    /// exists and differs), and its evaluation cases are added. `rename` installs it under another slug.
    pub async fn add_template(&self, id: &str, rename: Option<&str>) -> Result<TemplateInstalled, HubError> {
        let starter = starters::get(id).ok_or_else(|| {
            let known: Vec<&str> = starters::all().iter().map(|s| s.id).collect();
            HubError(format!("There is no template called {id}. The templates are: {}.", known.join(", ")))
        })?;
        let now = SystemClock.now();
        let mut skills = Vec::new();
        for skill in starter.skills {
            let library = self.skills.clone();
            let (name, files) = (skill.name, skill.owned_files());
            let record = tokio::task::spawn_blocking(move || {
                library.import_files(files, Some(name), Some("built-in template".to_string()), now)
            })
            .await
            .map_err(|e| HubError(e.to_string()))?
            .map_err(|e| HubError(e.to_string()))?;
            self.agents.record_skill(&self.tenant, record.clone()).await?;
            skills.push(record);
        }
        let imported = self.import_agent(starter.agent, rename, Some(format!("From the {id} template."))).await?;
        let mut cases = 0;
        for text in starter.cases {
            let case: EvalCase = serde_json::from_str(text).map_err(|e| HubError(e.to_string()))?;
            case.validate().map_err(|e| HubError(e.to_string()))?;
            self.agents.put_eval_case(&self.tenant, imported.agent.id, case, now).await?;
            cases += 1;
        }
        Ok(TemplateInstalled { imported, skills, cases })
    }

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

    // -----------------------------------------------------------------------------------------------------------------
    // Skills

    /// Import a skill from a folder or a zip file into the library and remember it as the newest of its name.
    pub async fn import_skill(&self, path: &Path) -> Result<SkillRecord, HubError> {
        let now = SystemClock.now();
        let library = self.skills.clone();
        let owned = path.to_path_buf();
        let is_zip = owned.is_file();
        let record = tokio::task::spawn_blocking(move || {
            if is_zip { library.import_zip(&owned, None, now) } else { library.import_folder(&owned, None, now) }
        })
        .await
        .map_err(|e| HubError(e.to_string()))?
        .map_err(|e| HubError(e.to_string()))?;
        self.agents.record_skill(&self.tenant, record.clone()).await?;
        Ok(record)
    }

    /// Every skill, by name.
    pub async fn skills_list(&self) -> Result<Vec<SkillRecord>, HubError> {
        Ok(self.agents.skills(&self.tenant).await?)
    }

    /// One skill by name.
    pub async fn skill(&self, name: &str) -> Result<SkillRecord, HubError> {
        self.agents
            .skill(&self.tenant, name)
            .await?
            .ok_or_else(|| HubError(format!("There is no skill called {name}. `zorix skill list` shows them.")))
    }

    /// Forget a skill. Agents that pinned it keep working until its files are cleaned up.
    pub async fn remove_skill(&self, name: &str) -> Result<(), HubError> {
        if self.agents.remove_skill(&self.tenant, name).await? {
            Ok(())
        } else {
            Err(HubError(format!("There is no skill called {name}.")))
        }
    }

    /// Delete the files of skills that nothing refers to (no agent version names them, none is current).
    pub async fn clean_skills(&self) -> Result<zorix_agents::skills::GcReport, HubError> {
        let keep: std::collections::HashSet<String> =
            self.agents.referenced_skill_hashes(&self.tenant).await?.into_iter().collect();
        let library = self.skills.clone();
        tokio::task::spawn_blocking(move || library.gc(&keep))
            .await
            .map_err(|e| HubError(e.to_string()))?
            .map_err(|e| HubError(e.to_string()))
    }
}
