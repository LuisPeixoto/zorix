//! An in-memory [`ExecutionRepository`]: for tests, for demos, and as the reference the SQLite store must match.

use std::collections::HashMap;

use async_trait::async_trait;
use parking_lot::Mutex;
use zorix_core::{
    Approval, ApprovalId, Checkpoint, Commit, Committed, CreateOutcome, EVENT_SCHEMA_VERSION, Error, Execution,
    ExecutionEvent, ExecutionId, ExecutionRepository, ExecutionState, ListFilter, NewEvent, Result, TenantId,
    Timestamp, ToolCallRecord,
};

#[derive(Default)]
struct Inner {
    next_seq: u64,
    executions: HashMap<ExecutionId, Execution>,
    keys: HashMap<(TenantId, String), ExecutionId>,
    events: Vec<ExecutionEvent>,
    checkpoints: HashMap<ExecutionId, Checkpoint>,
    tool_calls: HashMap<ExecutionId, Vec<ToolCallRecord>>,
    approvals: HashMap<ExecutionId, Vec<Approval>>,
    leases: HashMap<ExecutionId, (String, Timestamp)>,
}

impl Inner {
    fn append(&mut self, id: ExecutionId, events: Vec<NewEvent>) -> Vec<ExecutionEvent> {
        events
            .into_iter()
            .map(|event| {
                self.next_seq += 1;
                let stored = ExecutionEvent {
                    seq: self.next_seq,
                    execution_id: id,
                    at: event.at,
                    schema_version: EVENT_SCHEMA_VERSION,
                    kind: event.kind,
                    payload: event.payload,
                };
                self.events.push(stored.clone());
                stored
            })
            .collect()
    }
}

/// Keeps everything in memory. Cheap to create; forgets everything when dropped.
#[derive(Default)]
pub struct MemoryRepository {
    inner: Mutex<Inner>,
}

impl MemoryRepository {
    /// An empty repository.
    pub fn new() -> Self {
        Self::default()
    }
}

fn upsert<T>(items: &mut Vec<T>, item: T, same: impl Fn(&T) -> bool) {
    match items.iter_mut().find(|existing| same(existing)) {
        Some(existing) => *existing = item,
        None => items.push(item),
    }
}

#[async_trait]
impl ExecutionRepository for MemoryRepository {
    async fn create(&self, execution: Execution, events: Vec<NewEvent>) -> Result<CreateOutcome> {
        let mut inner = self.inner.lock();
        if let Some(key) = &execution.idempotency_key
            && let Some(existing) = inner.keys.get(&(execution.tenant.clone(), key.clone()))
            && let Some(found) = inner.executions.get(existing)
        {
            return Ok(CreateOutcome::Existing(found.clone()));
        }
        let mut stored = execution;
        stored.version = 1;
        if let Some(key) = &stored.idempotency_key {
            inner.keys.insert((stored.tenant.clone(), key.clone()), stored.id);
        }
        inner.executions.insert(stored.id, stored.clone());
        let events = inner.append(stored.id, events);
        Ok(CreateOutcome::Created(Committed { execution: stored, events }))
    }

    async fn commit(&self, commit: Commit) -> Result<Committed> {
        let mut inner = self.inner.lock();
        let id = commit.execution.id;
        let current = inner.executions.get(&id).ok_or(Error::NotFound(id))?;
        if current.version != commit.expected_version {
            return Err(Error::Conflict(id));
        }
        if let Some(owner) = &commit.lease_owner
            && inner.leases.get(&id).map(|(holder, _)| holder.as_str()) != Some(owner.as_str())
        {
            return Err(Error::Conflict(id));
        }
        let mut stored = commit.execution;
        stored.version = commit.expected_version + 1;
        inner.executions.insert(id, stored.clone());
        let events = inner.append(id, commit.events);
        if let Some(checkpoint) = commit.checkpoint {
            inner.checkpoints.insert(id, checkpoint);
        }
        for record in commit.tool_calls {
            let call_id = record.call_id.clone();
            upsert(inner.tool_calls.entry(id).or_default(), record, |r| r.call_id == call_id);
        }
        for approval in commit.approvals {
            let approval_id = approval.id;
            upsert(inner.approvals.entry(id).or_default(), approval, |a| a.id == approval_id);
        }
        Ok(Committed { execution: stored, events })
    }

    async fn load(&self, id: ExecutionId) -> Result<Option<Execution>> {
        Ok(self.inner.lock().executions.get(&id).cloned())
    }

    async fn get(&self, tenant: &TenantId, id: ExecutionId) -> Result<Option<Execution>> {
        Ok(self.inner.lock().executions.get(&id).filter(|e| &e.tenant == tenant).cloned())
    }

    async fn list(&self, tenant: &TenantId, filter: &ListFilter) -> Result<Vec<Execution>> {
        let inner = self.inner.lock();
        let mut found: Vec<Execution> = inner
            .executions
            .values()
            .filter(|e| &e.tenant == tenant)
            .filter(|e| filter.states.is_empty() || filter.states.contains(&e.state))
            .filter(|e| filter.conversation.is_none() || e.conversation == filter.conversation)
            .filter(|e| filter.agent.is_none() || e.agent.as_ref().map(|a| a.agent) == filter.agent)
            .filter(|e| filter.parent.is_none() || e.parent == filter.parent)
            .filter(|e| !filter.roots_only || e.parent.is_none())
            .filter(|e| filter.since.is_none_or(|since| e.created_at >= since))
            .cloned()
            .collect();
        found.sort_by_key(|e| std::cmp::Reverse((e.created_at, e.id)));
        found.truncate(if filter.limit == 0 { 100 } else { filter.limit });
        Ok(found)
    }

    async fn in_states(&self, states: &[ExecutionState]) -> Result<Vec<Execution>> {
        let inner = self.inner.lock();
        let mut found: Vec<Execution> =
            inner.executions.values().filter(|e| states.contains(&e.state)).cloned().collect();
        found.sort_by_key(|e| (e.created_at, e.id));
        Ok(found)
    }

    async fn events(&self, id: ExecutionId, after_seq: u64, limit: usize) -> Result<Vec<ExecutionEvent>> {
        let inner = self.inner.lock();
        Ok(inner.events.iter().filter(|e| e.execution_id == id && e.seq > after_seq).take(limit).cloned().collect())
    }

    async fn checkpoint(&self, id: ExecutionId) -> Result<Option<Checkpoint>> {
        Ok(self.inner.lock().checkpoints.get(&id).cloned())
    }

    async fn tool_calls(&self, id: ExecutionId) -> Result<Vec<ToolCallRecord>> {
        Ok(self.inner.lock().tool_calls.get(&id).cloned().unwrap_or_default())
    }

    async fn approvals(&self, id: ExecutionId) -> Result<Vec<Approval>> {
        Ok(self.inner.lock().approvals.get(&id).cloned().unwrap_or_default())
    }

    async fn approval(&self, id: ApprovalId) -> Result<Option<Approval>> {
        Ok(self.inner.lock().approvals.values().flatten().find(|a| a.id == id).cloned())
    }

    async fn acquire_lease(&self, id: ExecutionId, owner: &str, now: Timestamp, ttl_ms: u64) -> Result<bool> {
        let mut inner = self.inner.lock();
        if let Some((holder, expires)) = inner.leases.get(&id)
            && holder != owner
            && *expires > now
        {
            return Ok(false);
        }
        inner.leases.insert(id, (owner.to_string(), now.plus_millis(ttl_ms)));
        Ok(true)
    }

    async fn release_all_leases(&self) -> Result<u64> {
        let mut inner = self.inner.lock();
        let count = inner.leases.len() as u64;
        inner.leases.clear();
        Ok(count)
    }

    async fn release_lease(&self, id: ExecutionId, owner: &str) -> Result<()> {
        let mut inner = self.inner.lock();
        if inner.leases.get(&id).is_some_and(|(holder, _)| holder == owner) {
            inner.leases.remove(&id);
        }
        Ok(())
    }
}
