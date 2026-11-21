//! Reading what a link on a task points at. Only pull requests are read, and only through a connection that already exists.

use std::sync::Arc;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;
use zorix_core::{Action, ExecutionId, Grants, Permission, TenantId, ToolCallId, ToolContext, ToolRegistry};
use zorix_hub::Hub;

use crate::rules::LinkKind;

/// What is known about a pull request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PullState {
    /// Its number.
    pub number: u64,
    /// Its title.
    pub title: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    /// A draft that is not ready for review.
    pub draft: bool,
}

/// Something that can read a pull request.
#[async_trait]
pub trait PullReader: Send + Sync {
    /// The state of the pull request a link points at, or none when it cannot be read (no connection, no access, offline).
    async fn read(&self, kind: &LinkKind) -> Option<PullState>;
}

/// Reads pull requests with the native `github.get_pull` and `gitlab.get_pull` tools, granted exactly the repository of the link.
pub struct HubPullReader {
    hub: Arc<Hub>,
    tenant: TenantId,
}

impl HubPullReader {
    /// A reader for one tenant.
    pub fn new(hub: Arc<Hub>, tenant: TenantId) -> Self {
        Self { hub, tenant }
    }
}

#[async_trait]
impl PullReader for HubPullReader {
    async fn read(&self, kind: &LinkKind) -> Option<PullState> {
        let (forge, repo, number) = match kind {
            LinkKind::GithubPull { repo, number } => ("github", repo, *number),
            LinkKind::GitlabPull { repo, number } => ("gitlab", repo, *number),
            LinkKind::Other => return None,
        };
        let id = format!("{forge}.get_pull");
        let tool = self.hub.registry.get(&id)?;
        let ctx = ToolContext {
            execution_id: ExecutionId::new(),
            tenant: self.tenant.clone(),
            call_id: ToolCallId("task-link".into()),
            grants: Grants::new(vec![
                Permission::use_tool(&id),
                Permission::new(Action::Read, format!("{forge}:{repo}")),
            ]),
            cancel: CancellationToken::new(),
            workspace_root: None,
            conversation: None,
            agent: None,
        };
        let args = json!({"repo": repo, "number": number});
        let value: Value = zorix_runtime::ToolExecutor::run(&*tool, &ctx, args).await.ok()?;
        let merged = value["merged"].as_bool().unwrap_or(false);
        let state = if merged { "merged".to_string() } else { value["state"].as_str().unwrap_or("open").to_string() };
        Some(PullState {
            number,
            title: value["title"].as_str().unwrap_or_default().to_string(),
            state,
            draft: value["draft"].as_bool().unwrap_or(false),
        })
    }
}
