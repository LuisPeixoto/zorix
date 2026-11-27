//! Helpers for tests, shared with the other crates (enable the `testing` feature).
//!
//! The repository contract tests live here so that every implementation, in memory or SQLite, is held to the same
//! behaviour. Use [`repository_contract_tests!`](crate::repository_contract_tests) with an expression that makes a fresh
//! repository.

use serde_json::json;
use zorix_core::{
    Approval, ApprovalId, ApprovalStatus, Checkpoint, Commit, CreateOutcome, Error, EventKind, Execution, ExecutionId,
    ExecutionRepository, ExecutionState, ListFilter, LoopState, Message, NewEvent, NewExecution, TaskRequest, TenantId,
    Timestamp, ToolCallId, ToolCallRecord, ToolCallStatus, Trigger,
};

/// A valid execution for a tenant, optionally with an idempotency key.
pub fn sample_execution(tenant: &str, key: Option<&str>) -> Execution {
    let new = NewExecution {
        tenant: Some(TenantId::new(tenant).expect("a valid tenant")),
        request: TaskRequest { prompt: "do the thing".into(), model: "sim".into(), ..TaskRequest::default() },
        idempotency_key: key.map(str::to_string),
        ..NewExecution::default()
    };
    Execution::new(ExecutionId::new(), new, Timestamp(1_000))
}

/// An event with an empty payload.
pub fn event(kind: EventKind) -> NewEvent {
    NewEvent::new(kind, Timestamp(1_000), json!({}))
}

async fn stored<R: ExecutionRepository>(repo: &R, tenant: &str, key: Option<&str>) -> Execution {
    match repo.create(sample_execution(tenant, key), vec![event(EventKind::Created)]).await.expect("create") {
        CreateOutcome::Created(done) => done.execution,
        CreateOutcome::Existing(_) => panic!("expected a new execution"),
    }
}

/// Creating stores the execution at version 1 and numbers its events.
pub async fn create_stores_and_numbers_events<R: ExecutionRepository>(repo: &R) {
    let execution = sample_execution("acme", None);
    let id = execution.id;
    let outcome = repo.create(execution, vec![event(EventKind::Created), event(EventKind::Queued)]).await.unwrap();
    let CreateOutcome::Created(done) = outcome else { panic!("expected Created") };
    assert_eq!(done.execution.version, 1);
    assert_eq!(done.events.len(), 2);
    assert!(done.events[0].seq < done.events[1].seq);
    assert_eq!(done.events[0].kind, EventKind::Created);
    let tenant = TenantId::new("acme").unwrap();
    assert_eq!(repo.get(&tenant, id).await.unwrap().unwrap().version, 1);
}

/// The same idempotency key gives back the same execution and stores nothing new.
pub async fn idempotency_key_returns_the_existing_execution<R: ExecutionRepository>(repo: &R) {
    let first = stored(repo, "acme", Some("k1")).await;
    let again = repo.create(sample_execution("acme", Some("k1")), vec![event(EventKind::Created)]).await.unwrap();
    let CreateOutcome::Existing(existing) = again else { panic!("expected Existing") };
    assert_eq!(existing.id, first.id);
    assert_eq!(repo.events(first.id, 0, 100).await.unwrap().len(), 1, "no second event");

    // The key belongs to the tenant: another tenant using it gets its own execution.
    let other = repo.create(sample_execution("globex", Some("k1")), vec![event(EventKind::Created)]).await.unwrap();
    assert!(matches!(other, CreateOutcome::Created(_)));
}

/// A commit moves the version up by one, stores the change and appends events after the earlier ones.
pub async fn commit_applies_changes_in_order<R: ExecutionRepository>(repo: &R) {
    let mut execution = stored(repo, "acme", None).await;
    let before = repo.events(execution.id, 0, 100).await.unwrap();
    execution.apply(Trigger::Enqueue, Timestamp(2_000)).unwrap();
    let mut commit = Commit::new(execution.clone(), 1);
    commit.events = vec![event(EventKind::Queued), event(EventKind::Progress)];
    let done = repo.commit(commit).await.unwrap();
    assert_eq!(done.execution.version, 2);
    assert_eq!(done.execution.state, ExecutionState::Queued);
    let after = repo.events(execution.id, 0, 100).await.unwrap();
    assert_eq!(after.len(), before.len() + 2);
    assert!(after.windows(2).all(|pair| pair[0].seq < pair[1].seq), "sequence numbers strictly increase");
}

/// A commit that read an old version is refused and changes nothing, not even the events.
pub async fn stale_commit_is_a_conflict_and_changes_nothing<R: ExecutionRepository>(repo: &R) {
    let mut execution = stored(repo, "acme", None).await;
    execution.apply(Trigger::Enqueue, Timestamp(2_000)).unwrap();
    let mut commit = Commit::new(execution.clone(), 1);
    commit.events = vec![event(EventKind::Queued)];
    repo.commit(commit.clone()).await.unwrap();

    let error = repo.commit(commit).await.unwrap_err();
    assert_eq!(error, Error::Conflict(execution.id));
    assert_eq!(repo.events(execution.id, 0, 100).await.unwrap().len(), 2, "the rejected commit added no event");
    let tenant = TenantId::new("acme").unwrap();
    assert_eq!(repo.get(&tenant, execution.id).await.unwrap().unwrap().version, 2);
}

/// A commit made for an engine is refused unless that engine holds the lease, and nothing of it is written; a commit from outside a
/// run names no engine and is not affected.
pub async fn a_commit_for_an_engine_that_lost_the_lease_is_refused<R: ExecutionRepository>(repo: &R) {
    let mut execution = stored(repo, "acme", None).await;
    let id = execution.id;
    execution.apply(Trigger::Enqueue, Timestamp(2_000)).unwrap();
    let commit = |version: u64, owner: Option<&str>| {
        let mut commit = Commit::new(execution.clone(), version);
        commit.events = vec![event(EventKind::Queued)];
        commit.lease_owner = owner.map(str::to_string);
        commit
    };
    let now = Timestamp(1_000);
    assert!(repo.acquire_lease(id, "engine-a", now, 60_000).await.unwrap());

    // Another engine took it over (the first one's lease ran out in the meantime).
    assert!(repo.acquire_lease(id, "engine-b", Timestamp(1_000 + 61_000), 60_000).await.unwrap());
    let before = repo.events(id, 0, 100).await.unwrap().len();
    assert_eq!(
        repo.commit(commit(1, Some("engine-a"))).await.unwrap_err(),
        Error::Conflict(id),
        "the old holder is refused"
    );
    assert_eq!(repo.events(id, 0, 100).await.unwrap().len(), before, "and nothing of its commit was written");
    assert_eq!(repo.load(id).await.unwrap().unwrap().version, 1);

    assert_eq!(repo.commit(commit(1, Some("engine-b"))).await.unwrap().execution.version, 2, "the holder is not");
    let mut again = commit(2, None);
    again.events = vec![event(EventKind::Progress)];
    assert!(repo.commit(again).await.is_ok(), "a change from outside a run names no engine");
}

/// Another tenant cannot see an execution, in `get` or in `list`.
pub async fn tenants_are_isolated<R: ExecutionRepository>(repo: &R) {
    let mine = stored(repo, "acme", None).await;
    let theirs = stored(repo, "globex", None).await;
    let acme = TenantId::new("acme").unwrap();
    let globex = TenantId::new("globex").unwrap();
    assert!(repo.get(&globex, mine.id).await.unwrap().is_none());
    assert_eq!(repo.load(mine.id).await.unwrap().unwrap().id, mine.id, "load ignores tenants, for workers");
    assert!(repo.load(ExecutionId::new()).await.unwrap().is_none());
    assert!(repo.get(&acme, theirs.id).await.unwrap().is_none());
    let listed = repo.list(&acme, &ListFilter::default()).await.unwrap();
    assert_eq!(listed.iter().map(|e| e.id).collect::<Vec<_>>(), vec![mine.id]);
}

/// Listing filters by state and returns the newest first.
pub async fn list_filters_and_orders<R: ExecutionRepository>(repo: &R) {
    let first = stored(repo, "acme", None).await;
    let mut later = sample_execution("acme", None);
    later.created_at = Timestamp(5_000);
    repo.create(later.clone(), vec![event(EventKind::Created)]).await.unwrap();

    let tenant = TenantId::new("acme").unwrap();
    let all = repo.list(&tenant, &ListFilter::default()).await.unwrap();
    assert_eq!(all.iter().map(|e| e.id).collect::<Vec<_>>(), vec![later.id, first.id]);

    let queued =
        repo.list(&tenant, &ListFilter { states: vec![ExecutionState::Queued], ..ListFilter::default() }).await;
    assert!(queued.unwrap().is_empty());
    let limited = repo.list(&tenant, &ListFilter { limit: 1, ..ListFilter::default() }).await.unwrap();
    assert_eq!(limited.len(), 1);
}

/// `in_states` looks across tenants.
pub async fn in_states_spans_tenants<R: ExecutionRepository>(repo: &R) {
    stored(repo, "acme", None).await;
    stored(repo, "globex", None).await;
    assert_eq!(repo.in_states(&[ExecutionState::Created]).await.unwrap().len(), 2);
    assert!(repo.in_states(&[ExecutionState::Running]).await.unwrap().is_empty());
}

/// Events can be read after a position and with a limit.
pub async fn events_can_be_paged<R: ExecutionRepository>(repo: &R) {
    let mut execution = stored(repo, "acme", None).await;
    execution.apply(Trigger::Enqueue, Timestamp(2_000)).unwrap();
    let mut commit = Commit::new(execution.clone(), 1);
    commit.events = vec![event(EventKind::Queued), event(EventKind::Started), event(EventKind::Progress)];
    repo.commit(commit).await.unwrap();

    let all = repo.events(execution.id, 0, 100).await.unwrap();
    assert_eq!(all.len(), 4);
    let page = repo.events(execution.id, all[0].seq, 2).await.unwrap();
    assert_eq!(page.iter().map(|e| e.seq).collect::<Vec<_>>(), vec![all[1].seq, all[2].seq]);
    assert!(repo.events(execution.id, all[3].seq, 10).await.unwrap().is_empty());
}

/// The latest checkpoint wins; tool calls and approvals are replaced by id and keep their order.
pub async fn records_are_replaced_by_id<R: ExecutionRepository>(repo: &R) {
    let execution = stored(repo, "acme", None).await;
    let id = execution.id;
    let checkpoint = |step: u64, text: &str| Checkpoint {
        execution_id: id,
        step,
        state: LoopState { transcript: vec![Message::user(text)], pending: vec![] },
        created_at: Timestamp(step as i64),
    };
    let call = |n: &str, status| ToolCallRecord {
        execution_id: id,
        call_id: ToolCallId(n.into()),
        tool: "clock.now".into(),
        arguments: json!({}),
        call_hash: "h".into(),
        status,
        result: None,
        error: None,
        attempts: 1,
        idempotent: true,
        requested_at: Timestamp(1),
        started_at: None,
        finished_at: None,
    };
    let approval_id = ApprovalId::new();
    let approval = |status| Approval {
        id: approval_id,
        execution_id: id,
        call_id: ToolCallId("a".into()),
        tool: "files.write".into(),
        arguments: json!({"path": "x"}),
        call_hash: "h".into(),
        reason: "writes".into(),
        status,
        requested_at: Timestamp(1),
        decided_at: None,
        decided_by: None,
        note: None,
    };

    let mut first = Commit::new(execution.clone(), 1);
    first.checkpoint = Some(checkpoint(1, "one"));
    first.tool_calls = vec![call("a", ToolCallStatus::Requested), call("b", ToolCallStatus::Requested)];
    first.approvals = vec![approval(ApprovalStatus::Pending)];
    repo.commit(first).await.unwrap();

    let mut second = Commit::new(execution.clone(), 2);
    second.checkpoint = Some(checkpoint(2, "two"));
    second.tool_calls = vec![call("a", ToolCallStatus::Succeeded)];
    second.approvals = vec![approval(ApprovalStatus::Approved)];
    repo.commit(second).await.unwrap();

    assert_eq!(repo.checkpoint(id).await.unwrap().unwrap().step, 2);
    let calls = repo.tool_calls(id).await.unwrap();
    assert_eq!(
        calls.iter().map(|c| (c.call_id.0.as_str(), c.status)).collect::<Vec<_>>(),
        vec![("a", ToolCallStatus::Succeeded), ("b", ToolCallStatus::Requested)]
    );
    assert_eq!(repo.approvals(id).await.unwrap().len(), 1);
    assert_eq!(repo.approval(approval_id).await.unwrap().unwrap().status, ApprovalStatus::Approved);
    assert!(repo.approval(ApprovalId::new()).await.unwrap().is_none());
}

/// A lease keeps others out until it expires or is released; its owner can extend it.
pub async fn leases_exclude_other_owners<R: ExecutionRepository>(repo: &R) {
    let execution = stored(repo, "acme", None).await;
    let id = execution.id;
    let t = Timestamp(10_000);
    assert!(repo.acquire_lease(id, "worker-1", t, 1_000).await.unwrap());
    assert!(!repo.acquire_lease(id, "worker-2", t.plus_millis(500), 1_000).await.unwrap());
    assert!(repo.acquire_lease(id, "worker-1", t.plus_millis(500), 1_000).await.unwrap(), "the owner may extend");
    assert!(!repo.acquire_lease(id, "worker-2", t.plus_millis(1_200), 1_000).await.unwrap(), "extended to 1500");
    assert!(repo.acquire_lease(id, "worker-2", t.plus_millis(1_600), 1_000).await.unwrap(), "expired");
    repo.release_lease(id, "worker-1").await.unwrap();
    assert!(
        !repo.acquire_lease(id, "worker-1", t.plus_millis(1_700), 1_000).await.unwrap(),
        "only the holder releases"
    );
    repo.release_lease(id, "worker-2").await.unwrap();
    assert!(repo.acquire_lease(id, "worker-1", t.plus_millis(1_800), 1_000).await.unwrap());
}

/// Dropping every lease lets a new owner in at once.
pub async fn all_leases_can_be_dropped<R: ExecutionRepository>(repo: &R) {
    let first = stored(repo, "acme", None).await;
    let second = stored(repo, "acme", None).await;
    let t = Timestamp(10_000);
    assert!(repo.acquire_lease(first.id, "gone-1", t, 60_000).await.unwrap());
    assert!(repo.acquire_lease(second.id, "gone-2", t, 60_000).await.unwrap());
    assert_eq!(repo.release_all_leases().await.unwrap(), 2);
    assert!(repo.acquire_lease(first.id, "new", t, 1_000).await.unwrap());
    assert!(repo.acquire_lease(second.id, "new", t, 1_000).await.unwrap());
    assert_eq!(repo.release_all_leases().await.unwrap(), 2);
}

/// Children are found by their parent, roots are the executions nobody started, a tree is walked nearest first, and a tenant never
/// sees another's tree.
pub async fn trees_are_queries<R: ExecutionRepository>(repo: &R) {
    let tenant = TenantId::new("acme").unwrap();
    let child_of = |parent: &Execution, root: &Execution, depth: u32, at: i64| {
        let new = NewExecution {
            tenant: Some(tenant.clone()),
            parent: Some(parent.id),
            root: Some(root.id),
            depth,
            request: TaskRequest { prompt: "part of it".into(), model: "sim".into(), ..TaskRequest::default() },
            ..NewExecution::default()
        };
        Execution::new(ExecutionId::new(), new, Timestamp(at))
    };
    let root = stored(repo, "acme", None).await;
    let alone = stored(repo, "acme", None).await;
    let first = child_of(&root, &root, 1, 2_000);
    let second = child_of(&root, &root, 1, 3_000);
    let grandchild = child_of(&first, &root, 2, 4_000);
    for e in [&first, &second, &grandchild] {
        repo.create(e.clone(), vec![event(EventKind::Created)]).await.unwrap();
    }
    // Someone else's execution never shows up, even if it names this parent.
    let mut stranger = child_of(&root, &root, 1, 5_000);
    stranger.tenant = TenantId::new("globex").unwrap();
    repo.create(stranger, vec![event(EventKind::Created)]).await.unwrap();

    let kids = repo.children(&tenant, root.id).await.unwrap();
    assert_eq!(
        kids.iter().map(|e| e.id).collect::<Vec<_>>(),
        vec![first.id, second.id],
        "oldest first, only this tenant's"
    );
    assert!(repo.children(&tenant, alone.id).await.unwrap().is_empty());
    let below = repo.descendants(&tenant, root.id).await.unwrap();
    assert_eq!(below.len(), 3);
    assert_eq!(below.last().unwrap().id, grandchild.id, "nearest first: the grandchild comes last");
    assert_eq!(below.iter().find(|e| e.id == grandchild.id).unwrap().depth, 2);

    let roots = repo.list(&tenant, &ListFilter { roots_only: true, ..ListFilter::default() }).await.unwrap();
    assert_eq!(roots.len(), 2, "only the two that nobody started");
    assert!(roots.iter().all(|e| e.parent.is_none()));
    let filtered = repo.list(&tenant, &ListFilter { parent: Some(first.id), ..ListFilter::default() }).await.unwrap();
    assert_eq!(filtered.iter().map(|e| e.id).collect::<Vec<_>>(), vec![grandchild.id]);
    assert_eq!(repo.get(&tenant, grandchild.id).await.unwrap().unwrap().root, Some(root.id));
}

/// Generates one `#[tokio::test]` per repository contract case. `$make` is an expression that builds a fresh repository.
#[macro_export]
macro_rules! repository_contract_tests {
    ($make:expr) => {
        $crate::repository_contract_tests!(@case $make, create_stores_and_numbers_events);
        $crate::repository_contract_tests!(@case $make, idempotency_key_returns_the_existing_execution);
        $crate::repository_contract_tests!(@case $make, commit_applies_changes_in_order);
        $crate::repository_contract_tests!(@case $make, stale_commit_is_a_conflict_and_changes_nothing);
        $crate::repository_contract_tests!(@case $make, a_commit_for_an_engine_that_lost_the_lease_is_refused);
        $crate::repository_contract_tests!(@case $make, tenants_are_isolated);
        $crate::repository_contract_tests!(@case $make, list_filters_and_orders);
        $crate::repository_contract_tests!(@case $make, in_states_spans_tenants);
        $crate::repository_contract_tests!(@case $make, events_can_be_paged);
        $crate::repository_contract_tests!(@case $make, records_are_replaced_by_id);
        $crate::repository_contract_tests!(@case $make, trees_are_queries);
        $crate::repository_contract_tests!(@case $make, leases_exclude_other_owners);
        $crate::repository_contract_tests!(@case $make, all_leases_can_be_dropped);
    };
    (@case $make:expr, $case:ident) => {
        #[tokio::test]
        async fn $case() {
            let repository = $make;
            $crate::testing::$case(&repository).await;
        }
    };
}

// ---------------------------------------------------------------------------------------------------------------------
// Faults

use std::sync::Arc;

use async_trait::async_trait;
use parking_lot::Mutex;
use zorix_core::{Committed, ExecutionEvent, Result};

/// What a [`FaultyRepository`] is told to break.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    /// `create`.
    Create,
    /// `commit`.
    Commit,
    /// `load`.
    Load,
    /// `events`.
    Events,
    /// `checkpoint`.
    Checkpoint,
    /// `acquire_lease`.
    AcquireLease,
}

/// How a call breaks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Fault {
    /// The call fails and nothing was written.
    Before(Error),
    /// The write happens, and then the call reports a failure: the caller does not know whether it worked (a connection that dropped
    /// after the commit, a process that died right after it). Only `create` and `commit` write; for the rest it is like `Before`.
    AfterWrite(Error),
}

struct Plan {
    method: Method,
    /// The call that breaks, counting from 1 among the calls of that method.
    at: u32,
    fault: Fault,
    seen: u32,
    fired: bool,
}

/// A repository that breaks one call on purpose and is otherwise the one it wraps. Used to show that a failure at any point leaves a
/// state a restart can continue from.
pub struct FaultyRepository {
    inner: Arc<dyn ExecutionRepository>,
    plan: Mutex<Option<Plan>>,
}

impl FaultyRepository {
    /// Wrap a repository. Nothing breaks until [`fail`](Self::fail) is called.
    pub fn new(inner: Arc<dyn ExecutionRepository>) -> Self {
        Self { inner, plan: Mutex::new(None) }
    }

    /// Break the `at`-th call (from 1) of `method`, once.
    pub fn fail(&self, method: Method, at: u32, fault: Fault) {
        *self.plan.lock() = Some(Plan { method, at, fault, seen: 0, fired: false });
    }

    /// Whether the planned fault has happened.
    pub fn fired(&self) -> bool {
        self.plan.lock().as_ref().is_some_and(|p| p.fired)
    }

    fn hit(&self, method: Method) -> Option<Fault> {
        let mut plan = self.plan.lock();
        let plan = plan.as_mut()?;
        if plan.method != method || plan.fired {
            return None;
        }
        plan.seen += 1;
        (plan.seen == plan.at).then(|| {
            plan.fired = true;
            plan.fault.clone()
        })
    }
}

#[async_trait]
impl ExecutionRepository for FaultyRepository {
    async fn create(&self, execution: Execution, events: Vec<NewEvent>) -> Result<CreateOutcome> {
        match self.hit(Method::Create) {
            Some(Fault::Before(error)) => Err(error),
            Some(Fault::AfterWrite(error)) => {
                self.inner.create(execution, events).await?;
                Err(error)
            }
            None => self.inner.create(execution, events).await,
        }
    }

    async fn commit(&self, commit: Commit) -> Result<Committed> {
        match self.hit(Method::Commit) {
            Some(Fault::Before(error)) => Err(error),
            Some(Fault::AfterWrite(error)) => {
                self.inner.commit(commit).await?;
                Err(error)
            }
            None => self.inner.commit(commit).await,
        }
    }

    async fn load(&self, id: ExecutionId) -> Result<Option<Execution>> {
        match self.hit(Method::Load) {
            Some(Fault::Before(error) | Fault::AfterWrite(error)) => Err(error),
            None => self.inner.load(id).await,
        }
    }

    async fn get(&self, tenant: &TenantId, id: ExecutionId) -> Result<Option<Execution>> {
        self.inner.get(tenant, id).await
    }

    async fn list(&self, tenant: &TenantId, filter: &ListFilter) -> Result<Vec<Execution>> {
        self.inner.list(tenant, filter).await
    }

    async fn in_states(&self, states: &[ExecutionState]) -> Result<Vec<Execution>> {
        self.inner.in_states(states).await
    }

    async fn events(&self, id: ExecutionId, after_seq: u64, limit: usize) -> Result<Vec<ExecutionEvent>> {
        match self.hit(Method::Events) {
            Some(Fault::Before(error) | Fault::AfterWrite(error)) => Err(error),
            None => self.inner.events(id, after_seq, limit).await,
        }
    }

    async fn checkpoint(&self, id: ExecutionId) -> Result<Option<Checkpoint>> {
        match self.hit(Method::Checkpoint) {
            Some(Fault::Before(error) | Fault::AfterWrite(error)) => Err(error),
            None => self.inner.checkpoint(id).await,
        }
    }

    async fn tool_calls(&self, id: ExecutionId) -> Result<Vec<ToolCallRecord>> {
        self.inner.tool_calls(id).await
    }

    async fn approvals(&self, id: ExecutionId) -> Result<Vec<Approval>> {
        self.inner.approvals(id).await
    }

    async fn approval(&self, id: ApprovalId) -> Result<Option<Approval>> {
        self.inner.approval(id).await
    }

    async fn acquire_lease(&self, id: ExecutionId, owner: &str, now: Timestamp, ttl_ms: u64) -> Result<bool> {
        match self.hit(Method::AcquireLease) {
            Some(Fault::Before(error) | Fault::AfterWrite(error)) => Err(error),
            None => self.inner.acquire_lease(id, owner, now, ttl_ms).await,
        }
    }

    async fn release_lease(&self, id: ExecutionId, owner: &str) -> Result<()> {
        self.inner.release_lease(id, owner).await
    }

    async fn release_all_leases(&self) -> Result<u64> {
        self.inner.release_all_leases().await
    }
}
