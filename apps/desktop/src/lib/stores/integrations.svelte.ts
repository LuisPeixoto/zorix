// What the Integrations screens show: MCP servers, the catalogue, connections and the sandbox. The Rust side decides
// everything; this keeps the latest answers and runs the actions, with a toast for the outcome.
import {
  type ServerSandbox,
  type AddServer,
  type CatalogEntry,
  type ConnectionKind,
  type ConnectionRow,
  type ServerDetail,
  type ServerRow,
  connectionAdd,
  connectionRemove,
  connectionTest,
  connectionsList,
  errorText,
  mcpAdd,
  mcpApproveAll,
  mcpSetSandbox,
  mcpCatalog,
  mcpList,
  mcpLogin,
  mcpLoginCancel,
  mcpLogout,
  mcpRefresh,
  mcpRemove,
  mcpRestart,
  mcpReview,
  mcpServer,
  mcpSetEnabled,
  mcpTrust,
  onMcpStatus,
  sandboxInstallPython,
  sandboxStatus,
} from "../api.ts";
import { toast } from "./toast.svelte.ts";
import { t } from "../i18n/index.ts";

class IntegrationsStore {
  servers = $state<ServerRow[]>([]);
  catalog = $state<CatalogEntry[]>([]);
  connections = $state<ConnectionRow[]>([]);
  sandbox = $state<{ javascript: boolean; python: boolean; python_megabytes: number } | null>(null);
  loaded = $state(false);
  error = $state<string | null>(null);
  /** The server whose drawer is open. */
  openName = $state<string | null>(null);
  detail = $state<ServerDetail | null>(null);
  /** Names of servers with a browser sign-in in progress. */
  signingIn = $state<string[]>([]);
  installing = $state<{ done: number; total: number } | null>(null);

  #started = false;
  #timer: ReturnType<typeof setTimeout> | null = null;

  /** How many servers want a person: tools to review, or a sign-in. */
  get attention(): number {
    return this.servers.filter((s) => s.enabled && (s.tools_to_review > 0 || s.state === "needs_login")).length;
  }

  async init(): Promise<void> {
    if (this.#started) return;
    this.#started = true;
    try {
      const [catalog] = await Promise.all([mcpCatalog(), this.refresh()]);
      this.catalog = catalog.entries;
      // Cards update as servers start, stop and fail, without polling.
      await onMcpStatus(() => this.#soon());
    } catch (e) {
      this.error = errorText(e);
    }
  }

  #soon(): void {
    if (this.#timer) return;
    this.#timer = setTimeout(() => {
      this.#timer = null;
      void this.refresh();
    }, 150);
  }

  async refresh(): Promise<void> {
    try {
      const [servers, connections, sandbox] = await Promise.all([mcpList(), connectionsList(), sandboxStatus()]);
      this.servers = servers;
      this.connections = connections;
      this.sandbox = sandbox;
      if (this.openName) await this.#loadDetail(this.openName);
      this.error = null;
    } catch (e) {
      this.error = errorText(e);
    }
    this.loaded = true;
  }

  async #loadDetail(name: string): Promise<void> {
    try {
      const detail = await mcpServer(name);
      if (this.openName === name) this.detail = detail;
    } catch {
      if (this.openName === name) {
        this.openName = null;
        this.detail = null;
      }
    }
  }

  async open(name: string): Promise<void> {
    this.openName = name;
    this.detail = null;
    await this.#loadDetail(name);
  }

  close(): void {
    this.openName = null;
    this.detail = null;
  }

  async #act<T>(work: () => Promise<T>): Promise<T | undefined> {
    try {
      const result = await work();
      await this.refresh();
      return result;
    } catch (e) {
      toast.err(errorText(e));
      await this.refresh();
      return undefined;
    }
  }

  async add(request: AddServer): Promise<ServerRow | undefined> {
    const row = await this.#act(() => mcpAdd(request));
    if (row) toast.ok(t("stores.integrations.wasAddedItIs", { name: row.name }));
    return row;
  }

  async setEnabled(name: string, enabled: boolean): Promise<void> {
    const row = await this.#act(() => mcpSetEnabled(name, enabled));
    if (row) toast.ok(enabled ? t("stores.integrations.isStarting", { name }) : t("stores.integrations.isOffItsTools", { name }));
  }

  async remove(name: string): Promise<void> {
    const ok = await this.#act(() => mcpRemove(name).then(() => true));
    if (ok) {
      this.close();
      toast.ok(t("stores.integrations.wasRemovedWithThe", { name }));
    }
  }

  async restart(name: string): Promise<void> {
    await this.#act(() => mcpRestart(name));
  }

  async refreshTools(name: string): Promise<void> {
    const ok = await this.#act(() => mcpRefresh(name).then(() => true));
    if (ok) toast.info(t("stores.integrations.askedForItsTools", { name }));
  }

  /** Approving sends the hash of the definition that was on screen, so the server cannot change the tool between the look and the click. */
  async review(name: string, tool: string, action: "approve" | "disable", hash: string | null = null): Promise<void> {
    await this.#act(() => mcpReview(name, tool, action, hash));
  }

  async approveAll(name: string, shown: [string, string][]): Promise<void> {
    const n = await this.#act(() => mcpApproveAll(name, shown));
    if (n !== undefined) toast.ok(t("stores.integrations.approvedToolS", { n }));
  }

  /** Choose how a local server is confined. A running server starts again with the choice. */
  async setSandbox(name: string, sandbox: ServerSandbox): Promise<boolean> {
    const done = await this.#act(() => mcpSetSandbox(name, sandbox).then(() => true));
    return done === true;
  }

  async trust(name: string, tool: string, skipAsking: boolean): Promise<void> {
    await this.#act(() => mcpTrust(name, tool, skipAsking));
  }

  async login(name: string): Promise<void> {
    this.signingIn = [...this.signingIn, name];
    toast.info(t("stores.integrations.yourBrowserOpenedTo"));
    try {
      const issuer = await mcpLogin(name);
      toast.ok(t("stores.integrations.signedInToThrough", { name, issuer }));
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      this.signingIn = this.signingIn.filter((n) => n !== name);
      await this.refresh();
    }
  }

  async cancelLogin(name: string): Promise<void> {
    await mcpLoginCancel(name).catch(() => {});
  }

  async logout(name: string): Promise<void> {
    const ok = await this.#act(() => mcpLogout(name).then(() => true));
    if (ok) toast.ok(t("stores.integrations.forgotTheSignIn", { name }));
  }

  // Connections

  async addConnection(kind: ConnectionKind, name: string, settings: Record<string, unknown>, secret?: string): Promise<ConnectionRow | undefined> {
    try {
      const row = await connectionAdd({ kind, name, settings, secret });
      await this.refresh();
      return row;
    } catch (e) {
      toast.err(errorText(e));
      return undefined;
    }
  }

  async testConnection(name: string): Promise<{ ok: boolean; note: string }> {
    try {
      const result = await connectionTest(name);
      await this.refresh();
      return result;
    } catch (e) {
      return { ok: false, note: errorText(e) };
    }
  }

  async removeConnection(name: string): Promise<void> {
    const ok = await this.#act(() => connectionRemove(name).then(() => true));
    if (ok) toast.ok(t("stores.integrations.wasRemovedWithIts", { name }));
  }

  async installPython(): Promise<void> {
    this.installing = { done: 0, total: 1 };
    try {
      await sandboxInstallPython((done, total) => (this.installing = { done, total }));
      toast.ok(t("stores.integrations.pythonIsInstalledFor"));
    } catch (e) {
      toast.err(errorText(e));
    } finally {
      this.installing = null;
      await this.refresh();
    }
  }
}

export const integrations = new IntegrationsStore();
