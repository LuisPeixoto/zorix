// The app's state. Effects follow ids, never the objects, which are replaced on every refresh.
import {
  type AppInfo,
  type ExecEvent,
  type ModelChoices,
  type ProviderRow,
  type Execution,
  type RunDetail,
  type StartRun,
  type ToolSpec,
  appInfo,
  cancelRun,
  decideRun,
  errorText,
  listModels,
  listProviders,
  listRuns,
  listTools,
  pauseRun,
  removeProviderKey,
  resumeRun,
  runDetail,
  setProviderKey,
  startRun,
  stopWatch,
  testProvider,
  watchRun,
} from "../api.ts";
import { isFinished } from "../format.ts";

class RunsStore {
  info = $state<AppInfo | null>(null);
  tools = $state<ToolSpec[]>([]);
  runs = $state<Execution[]>([]);
  selectedId = $state<string | null>(null);
  detail = $state<RunDetail | null>(null);
  events = $state<ExecEvent[]>([]);
  error = $state<string | null>(null);
  busy = $state(false);
  view = $state<"runs" | "providers">("runs");
  providers = $state<ProviderRow[]>([]);
  models = $state<ModelChoices | null>(null);
  providerNote = $state<string | null>(null);
  now = $state(Date.now());

  #refreshTimer: ReturnType<typeof setTimeout> | null = null;
  #started = false;

  /** Load what the screen needs and keep the list fresh. */
  async init(): Promise<void> {
    if (this.#started) return;
    this.#started = true;
    try {
      this.info = await appInfo();
      if (this.info.error) return;
      this.tools = await listTools();
      await this.refreshProviders();
      await this.refreshList();
    } catch (e) {
      this.error = errorText(e);
    }
    setInterval(() => {
      this.now = Date.now();
      if (!this.info?.error) void this.refreshList();
    }, 2000);
  }

  async refreshProviders(): Promise<void> {
    try {
      [this.providers, this.models] = await Promise.all([listProviders(), listModels()]);
    } catch (e) {
      this.error = errorText(e);
    }
  }

  showProviders(): void {
    this.view = "providers";
    this.providerNote = null;
    void this.refreshProviders();
  }

  showRuns(): void {
    this.view = "runs";
  }

  async saveKey(name: string, key: string): Promise<boolean> {
    this.providerNote = null;
    const ok = await this.#act(() => setProviderKey(name, key));
    if (ok) this.providerNote = `Key for ${name} saved in the system keychain.`;
    await this.refreshProviders();
    return ok;
  }

  async deleteKey(name: string): Promise<void> {
    this.providerNote = null;
    const ok = await this.#act(() => removeProviderKey(name));
    if (ok) this.providerNote = `Key for ${name} removed.`;
    await this.refreshProviders();
  }

  async check(name: string): Promise<void> {
    this.providerNote = null;
    let count = 0;
    const ok = await this.#act(async () => {
      count = await testProvider(name);
    });
    if (ok) this.providerNote = `${name} answered: ${count} model(s) available.`;
    await this.refreshProviders();
  }

  async refreshList(): Promise<void> {
    try {
      this.runs = await listRuns();
    } catch (e) {
      this.error = errorText(e);
    }
  }

  async #loadDetail(id: string): Promise<void> {
    try {
      const detail = await runDetail(id);
      if (this.selectedId === id) this.detail = detail;
    } catch (e) {
      this.error = errorText(e);
    }
  }

  /** Show a run and follow it live. */
  async select(id: string): Promise<void> {
    const previous = this.selectedId;
    if (previous && previous !== id) void stopWatch(previous).catch(() => {});
    this.selectedId = id;
    this.detail = null;
    this.events = [];
    this.error = null;
    await this.#loadDetail(id);
    void watchRun(id, 0, (event) => {
      if (this.selectedId !== id) return;
      if (!this.events.some((e) => e.seq === event.seq)) this.events = [...this.events, event];
      this.#scheduleRefresh(id);
    }).catch((e) => {
      if (this.selectedId === id) this.error = errorText(e);
    });
  }

  /** Go back to the "new run" form. */
  showForm(): void {
    if (this.selectedId) void stopWatch(this.selectedId).catch(() => {});
    this.selectedId = null;
    this.detail = null;
    this.events = [];
    this.error = null;
  }

  #scheduleRefresh(id: string): void {
    if (this.#refreshTimer) clearTimeout(this.#refreshTimer);
    this.#refreshTimer = setTimeout(() => {
      void this.#loadDetail(id);
      void this.refreshList();
    }, 80);
  }

  async #act(work: () => Promise<unknown>): Promise<boolean> {
    this.busy = true;
    this.error = null;
    try {
      await work();
      return true;
    } catch (e) {
      this.error = errorText(e);
      return false;
    } finally {
      this.busy = false;
    }
  }

  async start(request: StartRun): Promise<void> {
    let created: Execution | null = null;
    const ok = await this.#act(async () => {
      created = await startRun(request);
    });
    if (ok && created) {
      await this.refreshList();
      await this.select((created as Execution).id);
    }
  }

  async decide(approvalId: string, callHash: string, approve: boolean, note?: string): Promise<void> {
    const id = this.selectedId;
    if (!id) return;
    await this.#act(() => decideRun(id, approvalId, approve, callHash, note));
    this.#scheduleRefresh(id);
  }

  async cancel(): Promise<void> {
    const id = this.selectedId;
    if (!id) return;
    await this.#act(() => cancelRun(id, "Cancelled from the desktop app"));
    this.#scheduleRefresh(id);
  }

  async pause(): Promise<void> {
    const id = this.selectedId;
    if (!id) return;
    await this.#act(() => pauseRun(id));
    this.#scheduleRefresh(id);
  }

  async resume(): Promise<void> {
    const id = this.selectedId;
    if (!id) return;
    await this.#act(() => resumeRun(id));
    this.#scheduleRefresh(id);
  }

  get finished(): boolean {
    return this.detail ? isFinished(this.detail.execution.state) : false;
  }
}

export const store = new RunsStore();
