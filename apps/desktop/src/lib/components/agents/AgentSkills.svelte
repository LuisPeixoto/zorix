<script lang="ts">
  import { open as pickFolder } from "@tauri-apps/plugin-dialog";

  import { type AgentDetail, type SkillRecord, agentSave, errorText, skillFile, skillImport, skillRemove, skillsList } from "../../api.ts";
  import { agents } from "../../stores/agents.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Badge, Button, Callout, Card, CodeBlock, EmptyState, Icon, IconButton, Markdown } from "../../ui/index.ts";

  let { detail }: { detail: AgentDetail } = $props();
  const spec = $derived(detail.version.spec);

  let library = $state<SkillRecord[]>([]);
  let openFile = $state<{ hash: string; path: string; text: string } | null>(null);
  let busy = $state<string | null>(null);
  let error = $state<string | null>(null);

  const pinned = $derived(detail.skills);
  const missing = $derived(spec.skills.filter((p) => !pinned.some((s) => s.name === p.name && s.hash === p.hash)));
  const available = $derived(library.filter((s) => !spec.skills.some((p) => p.name === s.name)));

  async function loadLibrary() {
    try {
      library = await skillsList();
    } catch (e) {
      error = errorText(e);
    }
  }
  $effect(() => {
    void detail.version.hash;
    void loadLibrary();
  });

  async function rewrite(skills: { name: string; hash: string }[], note: string, key: string) {
    busy = key;
    error = null;
    try {
      await agentSave({ ...spec, skills }, note);
      await agents.reload();
    } catch (e) {
      error = errorText(e);
    } finally {
      busy = null;
    }
  }
  const attach = (s: SkillRecord) => rewrite([...spec.skills, { name: s.name, hash: s.hash }], `Added the skill ${s.name}`, s.name);
  const detach = (name: string) => rewrite(spec.skills.filter((p) => p.name !== name), `Removed the skill ${name}`, name);
  const refresh = (s: SkillRecord) => rewrite(spec.skills.map((p) => (p.name === s.name ? { name: s.name, hash: s.hash } : p)), `Updated the skill ${s.name}`, s.name);

  async function importSkill() {
    try {
      const folder = await pickFolder({ directory: true, multiple: false, title: "Choose a skill folder (it holds a SKILL.md)" });
      if (typeof folder !== "string") return;
      const record = await skillImport(folder);
      toast.ok(`Skill ${record.name} imported.`);
      await loadLibrary();
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  async function forget(name: string) {
    try {
      await skillRemove(name);
      toast.ok(`${name} was removed from the library. Agents that pin it keep working.`);
      await loadLibrary();
    } catch (e) {
      toast.err(errorText(e));
    }
  }

  async function view(hash: string, path: string) {
    if (openFile?.hash === hash && openFile.path === path) {
      openFile = null;
      return;
    }
    try {
      openFile = { hash, path, text: await skillFile(hash, path) };
    } catch (e) {
      toast.err(errorText(e));
    }
  }
  const newer = (s: SkillRecord) => library.find((l) => l.name === s.name && l.hash !== s.hash);
  const size = (n: number) => (n < 1024 ? `${n} B` : `${(n / 1024).toFixed(1)} KB`);
</script>

<div class="h-full overflow-y-auto">
  <div class="mx-auto flex max-w-3xl flex-col gap-6 p-8">
    {#if error}<Callout tone="err" title="That did not work">{error}</Callout>{/if}
    {#if missing.length}
      <Callout tone="warn" title="Some files are missing">
        {missing.map((m) => m.name).join(", ")} {missing.length === 1 ? "is" : "are"} pinned by this agent, but the files are not in the library (they were cleaned up). Import the skill again.
      </Callout>
    {/if}

    <section aria-labelledby="skills-title">
      <div class="mb-3 flex items-center gap-3">
        <h2 id="skills-title" class="text-lg font-semibold">This agent's skills</h2>
        <p class="flex-1 text-sm text-muted">The agent sees only each skill's name and description, and loads the rest when the task fits. Attaching one saves a new version.</p>
      </div>
      {#if pinned.length === 0}
        <Card padding="none"><EmptyState icon="file-text" title="No skills attached" text="A skill is a folder of instructions and scripts. Attach one from the library below, or import a folder." /></Card>
      {:else}
        <div class="space-y-3">
          {#each pinned as s (s.name)}
            <Card>
              <div class="flex items-start gap-3">
                <span class="grid size-9 shrink-0 place-items-center rounded-xl bg-accent-soft text-accent"><Icon name="file-text" size={18} /></span>
                <div class="min-w-0 flex-1">
                  <div class="flex flex-wrap items-center gap-2"><h3 class="mono text-md font-semibold">{s.name}</h3><Badge tone="muted">{s.hash.slice(0, 8)}</Badge>{#if s.license}<Badge tone="muted">{s.license}</Badge>{/if}</div>
                  <p class="mt-1 text-base text-muted">{s.description}</p>
                  {#if s.allowed_tools.length}<p class="mt-1.5 text-xs text-faint">Asks for {s.allowed_tools.join(" ")}. Shown only: a skill never grants anything.</p>{/if}
                  <ul class="mt-3 flex flex-wrap gap-1.5" aria-label="Files of {s.name}">
                    {#each s.files as f (f.path)}
                      <li>
                        <button type="button" class="mono inline-flex h-7 items-center gap-1.5 rounded-lg border px-2 text-xs transition hover:bg-hover {openFile?.hash === s.hash && openFile.path === f.path ? 'border-accent/50 bg-accent-soft text-accent' : 'border-line bg-panel-2 text-muted'}" aria-pressed={openFile?.hash === s.hash && openFile.path === f.path} onclick={() => view(s.hash, f.path)}>
                          {f.path}<span class="text-faint">{size(f.size)}</span>
                        </button>
                      </li>
                    {/each}
                  </ul>
                  {#if openFile && openFile.hash === s.hash}
                    <div class="mt-3 rounded-xl border border-line bg-panel-2 p-3">
                      {#if openFile.path.endsWith(".md")}<Markdown text={openFile.text} class="max-h-96 overflow-auto" />
                      {:else}<CodeBlock code={openFile.text} language={openFile.path.endsWith(".py") ? "py" : openFile.path.endsWith(".js") ? "js" : "text"} maxHeight={360} label={openFile.path} />{/if}
                    </div>
                  {/if}
                </div>
                <div class="flex shrink-0 flex-col items-end gap-1.5">
                  {#if newer(s)}<Button size="sm" variant="subtle" icon="refresh" loading={busy === s.name} onclick={() => refresh(newer(s)!)}>Use newer</Button>{/if}
                  <Button size="sm" loading={busy === s.name} onclick={() => detach(s.name)}>Detach</Button>
                </div>
              </div>
            </Card>
          {/each}
        </div>
      {/if}
    </section>

    <section aria-labelledby="library-title">
      <div class="mb-3 flex items-center gap-3">
        <h2 id="library-title" class="flex-1 text-lg font-semibold">Library</h2>
        <Button size="sm" icon="folder" onclick={importSkill}>Import a folder</Button>
      </div>
      {#if available.length === 0}
        <p class="text-sm text-muted">Every skill in the library is already attached. Import a folder with a SKILL.md to add another.</p>
      {:else}
        <div class="space-y-2">
          {#each available as s (s.name)}
            <div class="raised flex items-center gap-3 rounded-xl border border-line bg-panel px-3.5 py-2.5">
              <Icon name="file-text" size={16} class="text-faint" />
              <div class="min-w-0 flex-1">
                <p class="mono text-md font-medium">{s.name} <span class="text-xs font-normal text-faint">{s.hash.slice(0, 8)}</span></p>
                <p class="truncate text-sm text-muted">{s.description}</p>
              </div>
              <Button size="sm" variant="subtle" icon="plus" loading={busy === s.name} onclick={() => attach(s)}>Attach</Button>
              <IconButton icon="trash" label={`Remove ${s.name} from the library`} onclick={() => forget(s.name)} />
            </div>
          {/each}
        </div>
      {/if}
    </section>
  </div>
</div>
