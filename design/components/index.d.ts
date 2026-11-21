// ZORIX components. The shipped components are Svelte 5 (apps/desktop/src/lib/ui); these declarations document their props.
// Components marked v3 belong to workflows, projects, the workforce, the Command Center and the Marketplace.
type Tone = 'ok' | 'warn' | 'err' | 'info' | 'muted' | 'accent';
type Snippet = unknown;

export interface ButtonProps { variant?: 'primary' | 'secondary' | 'ghost' | 'danger' | 'subtle'; size?: 'sm' | 'md' | 'lg'; icon?: string; iconRight?: string; loading?: boolean; disabled?: boolean; }
export interface IconButtonProps { icon: string; label: string; variant?: 'ghost' | 'secondary' | 'danger'; size?: 'sm' | 'md'; }
export interface TabsProps { tabs: { value: string; label: string; count?: number; icon?: string }[]; value: string; }
export interface SegmentedProps { options: { value: string; label: string; icon?: string }[]; value: string; }
export interface FieldProps { label: string; hint?: string; error?: string; required?: boolean; }
export interface InputProps { value: string; invalid?: boolean; mono?: boolean; icon?: string; trailing?: Snippet; placeholder?: string; }
export interface SwitchProps { checked: boolean; description?: string; disabled?: boolean; }
export interface CheckboxProps { checked: boolean; description?: string; disabled?: boolean; }
export interface ToggleRowProps { title: string; description?: string; checked: boolean; disabled?: boolean; }
export interface BadgeProps { tone?: Tone; variant?: 'soft' | 'outline' | 'solid'; dot?: boolean; live?: boolean; icon?: string; }
export interface ChipProps { selected?: boolean; onclick?: () => void; onremove?: () => void; }
export interface StatusPillProps { status: 'ready' | 'starting' | 'restarting' | 'needs_login' | 'failed' | 'stopped' | 'off' | 'review'; label?: string; }
export interface CalloutProps { tone?: Tone; title?: string; icon?: string; actions?: Snippet; }
export interface ToastApi { ok(text: string): void; err(text: string): void; info(text: string): void; }
export interface StepperProps { steps: string[]; current: number; }
export interface SparkbarProps { value: number; max: number; label: string; }
export interface StatProps { label: string; value: string | number; detail?: string; icon?: string; }
export interface KeyValueProps { items: { label: string; value: string; mono?: boolean }[]; }
export interface ListRowProps { active?: boolean; onclick?: () => void; }
export interface CodeBlockProps { code: string; lang?: 'sql' | 'json' | 'javascript' | 'python' | 'text'; copy?: boolean; }
export interface DiffViewProps { before: string; after: string; mode?: 'words' | 'lines'; }
export interface MarkdownProps { source: string; citations?: { n: number; title: string; path: string }[]; }
export interface ScoreMatrixProps { rows: string[]; columns: string[]; scores: (number | null)[][]; }
export interface CardProps { title?: string; icon?: string; padding?: boolean; interactive?: boolean; actions?: Snippet; footer?: Snippet; }
export interface PageHeaderProps { title: string; subtitle?: string; icon?: string; actions?: Snippet; }
export interface EmptyStateProps { icon: string; title: string; text: string; actions?: Snippet; }
export interface ModalProps { open: boolean; title: string; icon?: string; size?: 'sm' | 'md' | 'lg'; footer?: Snippet; }
export interface DrawerProps { open: boolean; title: string; subtitle?: string; width?: number; leading?: Snippet; actions?: Snippet; footer?: Snippet; }
export interface CommandPaletteProps { open: boolean; commands: { id: string; label: string; group: string; icon?: string; hint?: string; keywords?: string[]; run: () => void }[]; }
export interface AvatarProps { emoji?: string; icon?: string; initials?: string; color?: Tone | 'cyan'; size?: 'sm' | 'md' | 'lg'; human?: boolean; status?: Tone; }
export interface BrandLogoProps { brand: string; size?: number; }
export interface ChatBubbleProps { role: 'user' | 'agent'; agent?: string; version?: number; streaming?: boolean; }
export interface ComposerProps { value: string; placeholder?: string; model?: string; onsend: (text: string) => void; }
export interface ToolCallCardProps { tool: string; state: 'running' | 'done' | 'failed' | 'refused' | 'waiting'; args?: unknown; result?: string; ms?: number; }
export interface ApprovalCardProps { tool: string; args: unknown; reason: string; fingerprint: string; origin?: string; onapprove: () => void; onrefuse: () => void; }
export interface DelegationCardProps { children: { agent: string; task: string; state: string; steps: number; cost: number; share: number }[]; onopen?: (id: string) => void; row?: Snippet; }
export interface TreeViewProps { root: unknown; selected?: string; now?: number; onselect?: (id: string) => void; }
export interface VersionTimelineProps { versions: { n: number; note: string; by: string; at: string; score?: number }[]; current: number; }
/** v3 */ export interface WorkflowCanvasProps { nodes: WorkflowNodeProps[]; edges: { from: string; to: string; label?: string; state?: 'idle' | 'live' | 'done' }[]; selected?: string; readonly?: boolean; }
/** v3 */ export interface WorkflowNodeProps { id: string; kind: 'trigger' | 'agent' | 'logic' | 'approval' | 'action'; title: string; detail?: string; state?: 'idle' | 'running' | 'waiting' | 'done' | 'failed'; x: number; y: number; }
/** v3 */ export interface TriggerChipProps { kind: 'schedule' | 'webhook' | 'event' | 'manual' | 'workflow'; label: string; }
/** v3 */ export interface TaskBoardProps { lanes: { id: string; name: string; tone: Tone }[]; tasks: TaskCardProps[]; onmove?: (task: string, lane: string) => void; }
/** v3 */ export interface TaskCardProps { key: string; title: string; priority: 'urgent' | 'high' | 'normal' | 'low'; assignees: AvatarProps[]; due?: string; late?: boolean; progress?: number; deps?: number; deliverables?: number; }
/** v3 */ export interface OrgChartProps { root: { worker: WorkerCardProps; reports?: unknown[] }; }
/** v3 */ export interface WorkerCardProps { name: string; role: string; avatar: AvatarProps; human?: boolean; figures: [string, string][]; }
/** v3 */ export interface AutonomyDialProps { level: 'suggest' | 'ask' | 'notify' | 'budgeted' | 'full'; limits?: string; }
/** v3 */ export interface MetricTileProps { label: string; icon: string; value: string; unit?: string; delta?: string; deltaTone?: Tone; series: number[]; color?: 1 | 2 | 3 | 4 | 5; detail?: string; }
/** v3 */ export interface ChartPanelProps { title: string; headline: string; series: { name: string; values: number[]; dashed?: boolean }[]; range: '7d' | '14d' | '30d'; breakdown?: { name: string; share: number }[]; }
/** v3 */ export interface LogStreamProps { lines: { at: string; level: 'OK' | 'INFO' | 'WAIT' | 'WARN' | 'ERR'; source: string; message: string }[]; live?: boolean; filter?: string; }
/** v3 */ export interface HealthGridProps { items: { name: string; brand?: string; status: StatusPillProps['status']; hours: ('ok' | 'warn' | 'err')[]; detail: string }[]; }
/** v3 */ export interface ListingCardProps { kind: 'Agent' | 'Team' | 'Skill' | 'Integration' | 'Workflow'; title: string; publisher: string; verified?: boolean; description: string; rating: number; installs: string; installed?: boolean; }
