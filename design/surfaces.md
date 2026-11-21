# Product surfaces

Every screen of ZORIX at version 1.0, the components it is built from, and the one primary action it offers. Phases 1–6 are shipped today; phases 7–11 are specified here in the same system.

| Phase | Screen | Built from | Primary action |
|---|---|---|---|
| 1 · Runtime | Runs | PageHeader, List, Badge, TreeView, Stat, ToolCallCard, ApprovalCard | Start run |
| 1 · Runtime | New run | Field, Switch, Chip, Segmented, Callout | Start run |
| 2 · Providers | Providers and keys | List, BrandLogo, StatusPill, Field, Modal | Save key |
| 3 · Memory | Memory, Documents | List, Markdown, Sparkbar, Callout, DiffView | Approve memory |
| 4 · Tools | Integrations, MCP servers | HealthGrid, BrandLogo, StatusPill, Stepper, KeyValue, Drawer | Connect |
| 5 · Agents | Agents: gallery, chat, definition, versions, evals | Card, Avatar, ChatBubble, Composer, VersionTimeline, DiffView, ScoreMatrix | Open chat |
| 6 · Orchestration | Teams, work tree | DelegationCard, TreeView, Avatar, Sparkbar | Ask the team |
| 7 · Workflows | Workflow list, editor, run view | WorkflowCanvas, WorkflowNode, TriggerChip, Drawer, VersionTimeline, ApprovalCard | Publish workflow |
| 8 · Projects | Projects, board, list, timeline, task | TaskBoard, TaskCard, Avatar, Drawer, Stepper | New task |
| 9 · Workforce | Organisation, worker profile, autonomy | OrgChart, WorkerCard, AutonomyDial, MetricTile, LogStream | Hire a worker |
| 10 · Command Center | Overview, costs, logs, health, reports, settings and permissions | MetricTile, ChartPanel, LogStream, HealthGrid, ToggleRow, KeyValue | Export report |
| 11 · Marketplace | Browse, listing, publish | ListingCard, Tabs, Stepper, Callout, Modal | Install |

## Rules that hold across surfaces

- **Approvals look the same everywhere.** A tool call in a run, a step in a workflow, a task moving to Done under a manager, or an install that asks for new grants: each uses ApprovalCard with the exact arguments and a fingerprint.
- **Versions look the same everywhere.** Agents, teams and workflows save versions shown by VersionTimeline and compared with DiffView.
- **Trees shrink.** A run tree, a workflow and an org chart all show that budgets and permissions only narrow downwards; show the remaining budget beside each child.
- **People are round, agents are square.** Avatars tell them apart on boards, org charts and chats.
- **Live means moving.** Only things happening now animate: a live dot, a running edge, a streaming caret, a live log.

## Assembled screens

The **Screens** group shows 18 of these surfaces fully assembled at 1440 px, inside the app shell. The shell has a glass top bar with ⌘K search, the approvals bell and theme. The sidebar holds the Work, Organisation, Connect and Operate groups, with today's spend at its foot. The screens are: Command Center, Run, New run, Providers, Memory, Integrations, Agents, Agent chat, Agent versions, Workflow editor, Workflow run, Workflows, Project board, Project timeline, Workforce, Logs and traces, Marketplace, and Settings › Permissions. Each one uses only the components and tokens of this system. The shell classes (`zx-app`, `zx-top`, `zx-side`, `zx-nav`, `zx-main`, `zx-panel`, `zx-table`, `zx-gantt`, `zx-wf`) live in `bundle.css`.
