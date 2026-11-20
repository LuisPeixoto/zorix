# Multi-agent orchestration: what exists and what ZORIX takes

Research for Phase 6. Checked on 2026-11-20. Sources were read through web search results that day; the vendor pages are the primary
sources, and the blog summaries are marked as secondary. Numbers quoted from secondary summaries are not used as design inputs.

## What people build

| Pattern | Who decides the next step | Shape |
|---|---|---|
| **Orchestrator and workers** (Anthropic's Research system) | A lead agent, after each round of results | One lead plans, starts subagents in parallel, reads what they return, and decides whether to go on |
| **Agents as tools** (OpenAI Agents SDK) | The calling agent, which keeps the conversation | A specialist handles a bounded subtask and returns a result; its state is not inherited unless shared on purpose |
| **Handoff** (OpenAI Agents SDK) | The specialist takes over the conversation | Control moves; by default the whole history goes with it |
| **Supervisor** (LangGraph supervisor, CrewAI hierarchical process) | One central agent delegates to specialists | The common production default in the sources read |
| **Swarm** (LangGraph swarm) | Agents hand work to each other | Flexible when the path is unknown; sources warn that handoffs can loop and recommend hop counters |
| **Hierarchical** | Supervisors manage supervisors | Nested subgraphs in LangGraph, nested crews in CrewAI |

## Lessons worth taking

From Anthropic's engineering write-up of its multi-agent Research system (primary source:
anthropic.com/engineering/multi-agent-research-system; the pages were summarised by search, not read in full):

- **Delegate with a contract.** Short, vague instructions made subagents misread the task or repeat each other's searches. Each
  subagent needs an objective, an output format, guidance on tools and sources, and clear boundaries. ZORIX turns this into a
  typed task: `agent`, `task`, optional `input`, `expected_output`, `context` and `files`.
- **Match effort to the question.** Simple lookups want one agent; comparisons a few; broad research many. ZORIX bounds the fan-out
  (8 tasks per call, 32 descendants per run, depth 3) and says so in the instructions, so the lead scales effort itself.
- **Breadth is where it pays.** Gains were largest on queries with several independent directions. Parallel children are a first
  class case, not an afterthought.
- **Cost multiplies.** Secondary summaries report many times the tokens of a plain chat. ZORIX therefore counts every descendant
  against the root's budget.
- **A failed agent cannot just restart from zero.** Runs here are checkpointed, and a failed child is *reported* to its parent
  rather than raised, so the lead can retry, reroute or say what is missing.

From OpenAI's orchestration guide (primary source: developers.openai.com/api/docs/guides/agents/orchestration):

- **Agent as tool when the caller keeps the conversation; handoff when the specialist should own the answer.** ZORIX takes the first
  as its delegation model: the lead stays in charge, children return results. A handoff changes who talks to the person, which
  conflicts with "approvals and answers stay where the person is looking", so it is left out.
- **Do not inherit state by accident.** A child starts with the handoff the lead wrote, not the lead's conversation. This also keeps
  a child's context small and its memory narrowed.

From the supervisor and swarm literature (secondary sources only):

- **Loops need a counter.** ZORIX refuses delegation to an agent already in the chain, and caps depth.

## What no framework read here does, and ZORIX must

These frameworks run in one process and wait inside a call. ZORIX's runtime has a few workers and checkpoints every step, so:

1. **A parent must not hold a worker while it waits.** Four parents each waiting for a child would starve a pool of four. The parent
   is suspended in a new state and woken when its children finish, the way an approval already suspends a run.
2. **The wait must survive a crash.** The children are created with idempotency keys derived from the parent's call, and recovery
   reconciles a parent whose children all finished while it was down.
3. **Permissions and budgets only shrink down the tree.** A child gets the intersection of the parent's grants and the child agent's
   own; its cost counts against the parent; cancelling the parent cancels the tree.
4. **A person sees one place to decide.** An approval asked by a grandchild must be answerable from the root, not by opening every
   run.

## What ZORIX takes, in one line each

- Delegation is a **tool the model calls** (`agents.delegate`), restricted by the agent's access or by a **team**.
- Children run **in parallel**, the parent suspends, results come back **structured**.
- Nesting is bounded; cycles are refused.
- Teams are plain records (a lead and members), not a workflow language: scripted orchestration is Phase 7.
