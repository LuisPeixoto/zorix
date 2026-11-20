//! Teams: a lead and the colleagues it may ask (ADR-0031). A team is a record; running one runs its lead with the members added to
//! what it may ask.

use zorix_agents::{AgentAsk, AgentCard, ModelResolver, starters};
use zorix_core::{AgentSlug, AgentStore, Avatar, Clock, NewExecution, SystemClock, Team};

use crate::agents::TemplateInstalled;
use crate::{Hub, HubError};

/// A team with the agents it names, as the screens show it.
#[derive(Clone, Debug)]
pub struct TeamCard {
    /// The team itself.
    pub team: Team,
    /// Its lead, when that agent exists.
    pub lead: Option<AgentCard>,
    /// The members that exist.
    pub members: Vec<AgentCard>,
    /// Names the team lists that are not agents (any more).
    pub missing: Vec<String>,
}

/// A starter team, as the gallery offers it.
#[derive(Clone, Debug)]
pub struct TeamTemplate {
    /// What to type after `--template`.
    pub id: String,
    /// Its name.
    pub name: String,
    /// What it is for.
    pub description: String,
    /// Its face.
    pub emoji: String,
    /// Its colour name.
    pub color: String,
    /// The lead's slug.
    pub lead: String,
    /// The members' slugs.
    pub members: Vec<String>,
    /// A team with this id already exists.
    pub installed: bool,
}

/// What installing a starter team did.
#[derive(Clone, Debug)]
pub struct TeamInstalled {
    /// The team.
    pub team: Team,
    /// The agents that were added (the ones that were already here are not touched).
    pub added: Vec<TemplateInstalled>,
}

impl Hub {
    async fn team_card(&self, team: Team) -> Result<TeamCard, HubError> {
        let cards = self.agent_cards(true).await?;
        let find = |slug: &AgentSlug| cards.iter().find(|c| c.slug == slug.as_str()).cloned();
        let lead = find(&team.lead);
        let members: Vec<AgentCard> = team.members.iter().filter_map(find).collect();
        let mut missing: Vec<String> = Vec::new();
        if lead.is_none() {
            missing.push(team.lead.to_string());
        }
        missing.extend(team.members.iter().filter(|m| find(m).is_none()).map(|m| m.to_string()));
        Ok(TeamCard { team, lead, members, missing })
    }

    /// Every team, by slug.
    pub async fn teams(&self) -> Result<Vec<TeamCard>, HubError> {
        let mut out = Vec::new();
        for team in self.agents.teams(&self.tenant).await? {
            out.push(self.team_card(team).await?);
        }
        Ok(out)
    }

    /// One team by name.
    pub async fn team(&self, slug: &str) -> Result<TeamCard, HubError> {
        let slug = AgentSlug::new(slug).map_err(HubError)?;
        let team = self
            .agents
            .team(&self.tenant, &slug)
            .await?
            .ok_or_else(|| HubError(format!("There is no team called {slug}. `zorix team list` shows them.")))?;
        self.team_card(team).await
    }

    /// Make a team, or change the one with this slug. The lead and every member must be an agent that exists.
    pub async fn save_team(&self, mut team: Team) -> Result<TeamCard, HubError> {
        team.tenant = self.tenant.clone();
        team.updated_at = SystemClock.now();
        team.validate()?;
        let cards = self.agent_cards(false).await?;
        for name in std::iter::once(&team.lead).chain(team.members.iter()) {
            if !cards.iter().any(|c| c.slug == name.as_str()) {
                return Err(HubError(format!(
                    "There is no agent called {name} (or it is archived), so it cannot be in a team. `zorix agent list` shows them."
                )));
            }
        }
        let saved = self.agents.put_team(&self.tenant, team).await?;
        self.team_card(saved).await
    }

    /// Delete a team. The agents in it stay.
    pub async fn remove_team(&self, slug: &str) -> Result<(), HubError> {
        let parsed = AgentSlug::new(slug).map_err(HubError)?;
        if self.agents.remove_team(&self.tenant, &parsed).await? {
            Ok(())
        } else {
            Err(HubError(format!("There is no team called {slug}.")))
        }
    }

    /// The execution for a request to a team: its lead runs, and may ask the members.
    pub async fn prepare_team_run(
        &self,
        slug: &str,
        mut ask: AgentAsk,
        resolver: &dyn ModelResolver,
    ) -> Result<NewExecution, HubError> {
        let card = self.team(slug).await?;
        if let Some(gone) = card.missing.first() {
            return Err(HubError(format!(
                "The team {slug} lists {gone}, which is not an agent any more. Change the team (or add that agent back)."
            )));
        }
        ask.team = Some(card.team.slug.to_string());
        ask.extra_access.extend(card.team.access());
        self.prepare_agent_run(card.team.lead.as_str(), None, ask, resolver).await
    }

    /// The ready-made teams, with whether each is already here.
    pub async fn team_templates(&self) -> Result<Vec<TeamTemplate>, HubError> {
        let mut out = Vec::new();
        for t in starters::teams() {
            let slug = AgentSlug::new(t.id).map_err(HubError)?;
            out.push(TeamTemplate {
                id: t.id.into(),
                name: t.name.into(),
                description: t.description.into(),
                emoji: t.emoji.into(),
                color: t.color.into(),
                lead: t.lead.into(),
                members: t.members.iter().map(|m| m.to_string()).collect(),
                installed: self.agents.team(&self.tenant, &slug).await?.is_some(),
            });
        }
        Ok(out)
    }

    /// Install a starter team: the agents it names that are not here yet (with their skills and cases), then the team itself. An
    /// agent that already exists is left as it is, so nothing a person changed is overwritten.
    pub async fn add_team_template(&self, id: &str) -> Result<TeamInstalled, HubError> {
        let template = starters::team(id).ok_or_else(|| {
            let known: Vec<&str> = starters::teams().iter().map(|t| t.id).collect();
            HubError(format!("There is no team template called {id}. The team templates are: {}.", known.join(", ")))
        })?;
        let mut added = Vec::new();
        for agent in std::iter::once(&template.lead).chain(template.members.iter()) {
            let slug = AgentSlug::new(*agent).map_err(HubError)?;
            if self.agents.agent_by_slug(&self.tenant, &slug).await?.is_none() {
                added.push(self.add_template(agent, None).await?);
            }
        }
        let team = Team {
            tenant: self.tenant.clone(),
            slug: AgentSlug::new(template.id).map_err(HubError)?,
            name: template.name.into(),
            description: template.description.into(),
            avatar: Avatar { emoji: template.emoji.into(), color: template.color.into() },
            lead: AgentSlug::new(template.lead).map_err(HubError)?,
            members: template.members.iter().map(|m| AgentSlug::new(*m).map_err(HubError)).collect::<Result<_, _>>()?,
            updated_at: SystemClock.now(),
        };
        let saved = self.save_team(team).await?;
        Ok(TeamInstalled { team: saved.team, added })
    }
}
