<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { onDestroy, onMount } from "svelte";

  import { type FileRow, type Found, type RootRow, type SyncProgress, addRoot, cancelSync, errorText, listRoots, onKnowledge, removeRoot, rootFiles, syncRoot, tryQuery } from "../api.ts";
  import { ago } from "../format.ts";
  import { toast } from "../stores/toast.svelte.ts";
  import { Badge, Button, Callout, Card, EmptyState, Field, Icon, Input, Modal, Switch } from "../ui/index.ts";

  let roots = $state<RootRow[]>([]);
  let progress = $state<Record<string, SyncProgress["progress"]>>({});
  let error = $state<string | null>(null);
  let busy = $state(false);
  let stop: (() => void) | null = null;
  let now = $state(Date.now());
  let tick: ReturnType<typeof setInterval> | null = null;

  async function load() {
    try {
      roots = await listRoots();
    } catch (e) {
      error = errorText(e);
    }
  }

  onMount(async () => {
    await load();
    stop = await onKnowledge({
      progress: (p) => {
        progress[p.root] = p.progress;
        if (!roots.some((r) => r.id === p.root)) void load();
      },
      done: (d) => {
        delete progress[d.root];
        if (d.error) toast.err(d.error);
        else if (d.report?.cancelled) toast.ok("Stopped. What was done stays indexed.");
        else toast.ok(`Indexed: ${d.report?.indexed ?? 0} changed, ${d.report?.unchanged ?? 0} unchanged.`);
        void load();
      },
    });
    tick = setInterval(() => (now = Date.now()), 5000);
  });
  onDestroy(() => {
    stop?.();
    if (tick) clearInterval(tick);
  });

  // ---- add a folder ----
  let adding = $state(false);
  let path = $state("");
  let project = $state("");
  let include = $state("");
  let exclude = $state("");
  let auto = $state(false);
  let formError = $state<string | null>(null);

  const lines = (text: string) => text.split(/[\n,]/).map((s) => s.trim()).filter(Boolean);

  async function pick() {
    try {
      const folder = await open({ directory: true, multiple: false, title: "Choose a folder of documents" });
      if (typeof folder === "string") path = folder;
    } catch (e) {
      formError = errorText(e);
    }
  }

  async function add() {
    formError = null;
    busy = true;
    try {
      await addRoot({ path, project: project.trim() || undefined, include: lines(include), exclude: lines(exclude), auto_sync: auto });
      adding = false;
      path = project = include = exclude = "";
      auto = false;
      await load();
    } catch (e) {
      formError = errorText(e);
    } finally {
      busy = false;
    }
  }

  async function sync(id: string) {
    try {
      await syncRoot(id);
      await load();
    } catch (e) {
      error = errorText(e);
    }
  }

  // ---- remove ----
  let doomed = $state<RootRow | null>(null);
  let confirm = $state(false);
  async function remove() {
    if (doomed) {
      try {
        await removeRoot(doomed.id);
        toast.ok("Removed from the index. The files were not touched.");
      } catch (e) {
        error = errorText(e);
      }
    }
    confirm = false;
    doomed = null;
    await load();
  }

  // ---- files ----
  let looking = $state<RootRow | null>(null);
  let files = $state<FileRow[]>([]);
  let problemsOnly = $state(true);
  let filesOpen = $state(false);

  async function showFiles(root: RootRow, only: boolean) {
    looking = root;
    problemsOnly = only;
    try {
      files = await rootFiles(root.id, only);
    } catch (e) {
      files = [];
      error = errorText(e);
    }
    filesOpen = true;
  }

  // ---- try a query ----
  let query = $state("");
  let found = $state<Found | null>(null);
  let searching = $state(false);
  async function search() {
    if (!query.trim()) return;
    searching = true;
    try {
      found = await tryQuery(query, "knowledge");
    } catch (e) {
      error = errorText(e);
    } finally {
      searching = false;
    }
  }

  const phaseText = (p: SyncProgress["progress"]) =>
    p.phase === "scanning" ? "Looking at the folder…" : p.phase === "indexing" ? `Reading files… ${p.done} of ${p.total}` : p.phase === "embedding" ? `Making vectors… ${p.done} of ${p.total}` : "Finishing…";
  const percent = (p: SyncProgress["progress"]) => (p.total ? Math.min(100, Math.round((p.done / p.total) * 100)) : 4);
  const rank = (h: { keyword_rank: number | null; vector_rank: number | null; cosine: number | null }) =>
    [h.keyword_rank ? `keyword #${h.keyword_rank}` : "", h.vector_rank ? `vector #${h.vector_rank}${h.cosine ? ` (${h.cosine.toFixed(2)})` : ""}` : ""].filter(Boolean).join(" · ");
</script>

<div class="space-y-5">
  <Callout tone="info" icon="shield">
    Folders are read, never changed. Files ignored by .gitignore, hidden files, keys and .env files are skipped. A run can read the matching
    pieces and cites the file it used.
  </Callout>

  {#if error}<Callout tone="err">{error}</Callout>{/if}

  <div class="flex justify-end">
    <Button variant="primary" icon="plus" onclick={() => (adding = true)}>Add a folder</Button>
  </div>

  {#if roots.length === 0}
    <Card>
      <EmptyState icon="folder" title="No documents yet" text="Add a folder of notes, docs or code and runs can find and cite what is in it." />
    </Card>
  {/if}

  {#each roots as root (root.id)}
    {@const p = progress[root.id]}
    <Card>
      <div class="flex items-start gap-3">
        <span class="grid size-9 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent"><Icon name="folder" size={17} /></span>
        <div class="min-w-0 flex-1">
          <p class="selectable mono truncate text-md font-medium" title={root.path}>{root.path}</p>
          <div class="mt-1.5 flex flex-wrap items-center gap-1.5">
            <Badge tone="accent">{root.scope}</Badge>
            <Badge tone="ok" dot>{root.indexed} indexed</Badge>
            {#if root.skipped}<Badge tone="muted">{root.skipped} left out</Badge>{/if}
            {#if root.failed}<Badge tone="err" dot>{root.failed} failed</Badge>{/if}
            {#if root.auto_sync}<Badge tone="muted">checked at start</Badge>{/if}
          </div>
          <p class="mt-1.5 text-xs text-faint">
            {root.last_sync_at ? `Last checked ${ago(root.last_sync_at, now)}` : "Not indexed yet"}{root.last_sync ? ` · last pass: ${root.last_sync.indexed} read, ${root.last_sync.embedded} vectors made` : ""}
          </p>
          {#if root.last_sync?.degraded}<p class="mt-1 text-xs text-warn">{root.last_sync.degraded}</p>{/if}
          {#if root.last_sync?.too_many_files}<p class="mt-1 text-xs text-warn">The folder has more files than are looked at.</p>{/if}
        </div>
        <div class="flex shrink-0 flex-wrap justify-end gap-1.5">
          {#if p || root.syncing}
            <Button size="sm" icon="stop" onclick={() => cancelSync(root.id)}>Stop</Button>
          {:else}
            <Button size="sm" icon="refresh" onclick={() => sync(root.id)}>Sync</Button>
          {/if}
          <Button size="sm" icon="list" onclick={() => showFiles(root, root.failed + root.skipped > 0)}>Files</Button>
          <Button size="sm" variant="danger" icon="trash" onclick={() => { doomed = root; confirm = true; }}>Remove</Button>
        </div>
      </div>
      {#if p}
        <div class="mt-4" role="progressbar" aria-valuemin="0" aria-valuemax="100" aria-valuenow={percent(p)} aria-label="Indexing progress">
          <div class="h-1.5 overflow-hidden rounded-full bg-hover"><div class="brand-gradient h-full rounded-full transition-all duration-200" style:width="{percent(p)}%"></div></div>
          <p class="mt-1.5 truncate text-xs text-muted">{phaseText(p)}{p.file ? ` · ${p.file}` : ""}{p.errors ? ` · ${p.errors} failed` : ""}</p>
        </div>
      {/if}
    </Card>
  {/each}

  <Card title="Try a question" icon="search">
    <p class="mb-3 text-sm text-muted">See what a run would find for a question, and which kind of search found it.</p>
    <form class="flex gap-2" onsubmit={(e) => { e.preventDefault(); void search(); }}>
      <Input class="min-w-0 flex-1" bind:value={query} placeholder="For example: how do we deploy?" aria-label="A question to try" />
      <Button type="submit" icon="search" loading={searching} disabled={!query.trim()}>Search</Button>
    </form>
    {#if found}
      {#if found.degraded}<p class="mt-3 text-xs text-warn">{found.degraded}; showing keyword results.</p>{/if}
      <ul class="mt-3 space-y-2">
        {#each found.hits as hit}
          <li class="rounded-lg border border-line bg-panel p-3">
            <div class="flex flex-wrap items-center gap-2">
              <span class="selectable mono text-xs font-medium text-fg">{hit.title}</span>
              <span class="text-xs text-faint">{rank(hit)}</span>
            </div>
            <p class="selectable mt-1 text-sm text-muted">{hit.text}</p>
          </li>
        {:else}
          <li class="text-sm text-muted">Nothing relevant. A run would get no documents for this question.</li>
        {/each}
      </ul>
    {/if}
  </Card>
</div>

<Modal bind:open={adding} title="Add a folder" icon="folder" size="lg">
  <div class="space-y-4">
    <div class="flex gap-2">
      <Input class="min-w-0 flex-1" mono bind:value={path} placeholder="No folder chosen" aria-label="Folder" />
      <Button icon="folder" onclick={pick}>Choose…</Button>
    </div>
    <Field label="Only for runs in this project folder (optional; empty means every run)">
      {#snippet children(f)}<Input id={f.id} mono bind:value={project} placeholder="/path/to/the/project" />{/snippet}
    </Field>
    <div class="grid grid-cols-2 gap-3">
      <Field label="Also include (patterns)">
        {#snippet children(f)}<Input id={f.id} mono bind:value={include} placeholder="*.log" />{/snippet}
      </Field>
      <Field label="Leave out (patterns)">
        {#snippet children(f)}<Input id={f.id} mono bind:value={exclude} placeholder="drafts/*" />{/snippet}
      </Field>
    </div>
    <Switch bind:checked={auto}>Look for changes whenever the app starts</Switch>
    {#if formError}<Callout tone="err">{formError}</Callout>{/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => (adding = false)}>Cancel</Button>
    <Button variant="primary" icon="check" loading={busy} disabled={!path.trim()} onclick={add}>Add and index</Button>
  {/snippet}
</Modal>

<Modal bind:open={confirm} title="Remove this folder?" icon="trash" size="sm">
  <p class="text-base text-muted">
    <strong class="text-fg">{doomed?.path}</strong> is removed from the index, along with every piece and vector made from it. The files stay where they are.
  </p>
  {#snippet footer()}
    <Button onclick={() => (confirm = false)}>Cancel</Button>
    <Button variant="danger" icon="trash" onclick={remove}>Remove</Button>
  {/snippet}
</Modal>

<Modal bind:open={filesOpen} title={problemsOnly ? "Files that were not indexed" : "Files"} icon="list" size="lg">
  <div class="mb-3 flex items-center gap-3">
    <Switch checked={problemsOnly} onchange={(e) => looking && showFiles(looking, e.currentTarget.checked)}>Only the ones with a problem</Switch>
  </div>
  <ul class="max-h-96 space-y-1 overflow-y-auto">
    {#each files as f}
      <li class="flex items-start gap-2 text-sm">
        <Badge tone={f.status === "indexed" ? "ok" : f.status === "failed" ? "err" : "muted"}>{f.status}</Badge>
        <span class="selectable mono min-w-0 flex-1 break-all">{f.file}</span>
        {#if f.reason}<span class="max-w-[45%] text-xs text-muted">{f.reason}</span>{/if}
      </li>
    {:else}
      <li class="text-sm text-muted">{problemsOnly ? "Every file was indexed." : "No files."}</li>
    {/each}
  </ul>
  {#snippet footer()}<Button onclick={() => (filesOpen = false)}>Close</Button>{/snippet}
</Modal>
