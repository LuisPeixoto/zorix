<script lang="ts">
  import { agents } from "../../stores/agents.svelte.ts";
  import { ago } from "../../format.ts";
  import { store } from "../../stores/runs.svelte.ts";
  import { Avatar, Badge, Button, Callout, Card, EmptyState, Icon, Modal, PageHeader, Skeleton, Field, Input, Textarea } from "../../ui/index.ts";
  import AgentPage from "./AgentPage.svelte";

  let adding = $state<string | null>(null);
  let importOpen = $state(false);
  let file = $state("");
  let rename = $state("");
  let importing = $state(false);

  async function add(id: string) {
    adding = id;
    const slug = await agents.addTemplate(id);
    adding = null;
    if (slug) void agents.open(slug);
  }

  async function doImport() {
    importing = true;
    const slug = await agents.importFile(file, rename);
    importing = false;
    if (slug) {
      importOpen = false;
      file = "";
      rename = "";
      void agents.open(slug);
    }
  }

  const free = $derived(agents.templates.filter((t) => !t.installed));
</script>

{#if agents.slug}
  <AgentPage />
{:else}
  <div class="h-full overflow-y-auto">
    <div class="mx-auto max-w-[1100px] p-8">
      <PageHeader title="Agents" subtitle="Reusable AI workers. Each has its own instructions, skills, tools, memory and versions, and is checked by evaluations." glow>
        {#snippet actions()}
          <Button icon="download" onclick={() => (importOpen = true)}>Import a file</Button>
        {/snippet}
      </PageHeader>

      {#if agents.error}<Callout tone="err" title="Agents could not be read">{agents.error}</Callout>{/if}

      {#if !agents.loaded}
        <div class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">{#each [0, 1, 2] as i (i)}<Skeleton class="h-40 rounded-2xl" />{/each}</div>
      {:else}
        {#if agents.cards.length === 0}
          <Card padding="none">
            <EmptyState icon="bot" title="No agents yet" text="Start from a ready-made agent below, or import a file. Each one comes with a skill and a few checks you can run." />
          </Card>
        {:else}
          <ul class="grid gap-4 sm:grid-cols-2 lg:grid-cols-3" aria-label="Your agents">
            {#each agents.cards as card (card.slug)}
              <li>
              <button
                type="button"
                onclick={() => agents.open(card.slug)}
                class="group raised flex h-full w-full flex-col rounded-2xl border border-line bg-panel p-4 text-left transition duration-150 ease-out hover:-translate-y-px hover:border-line-strong hover:shadow-3"
              >
                <span class="flex items-start gap-3">
                  <Avatar emoji={card.emoji} color={card.color} size="lg" />
                  <span class="min-w-0 flex-1">
                    <span class="block truncate text-lg font-semibold">{card.name}</span>
                    <span class="mono block truncate text-xs text-faint">{card.slug}</span>
                  </span>
                  <Icon name="chevron-right" size={16} class="mt-1 text-faint transition group-hover:translate-x-0.5 group-hover:text-accent" />
                </span>
                <span class="mt-3 line-clamp-2 min-h-[2.6em] text-base text-muted">{card.description || "No description yet."}</span>
                <span class="mt-4 flex flex-wrap items-center gap-1.5">
                  <Badge tone="accent">v{card.version}</Badge>
                  <Badge tone="muted">{card.default_model}</Badge>
                  {#if card.skills.length}<Badge tone="muted">{card.skills.length} skill{card.skills.length === 1 ? "" : "s"}</Badge>{/if}
                </span>
              </button>
              </li>
            {/each}
          </ul>
        {/if}

        {#if free.length}
          <section class="mt-10" aria-labelledby="templates-title">
            <div class="mb-3 flex items-baseline gap-3">
              <h2 id="templates-title" class="text-xl font-semibold">Add from templates</h2>
              <p class="text-sm text-muted">Ready-made agents. They are ordinary agents once added: edit, version and remove them freely.</p>
            </div>
            <div class="grid gap-4 sm:grid-cols-2">
              {#each free as t (t.id)}
                <Card interactive>
                  <div class="flex items-start gap-3">
                    <Avatar emoji={t.emoji} color={t.color} size="lg" />
                    <div class="min-w-0 flex-1">
                      <h3 class="text-md font-semibold">{t.name}</h3>
                      <p class="mt-0.5 text-sm text-muted">{t.description}</p>
                      <p class="mt-2 flex flex-wrap items-center gap-1.5 text-xs text-faint">
                        <Icon name="file-text" size={12} /> {t.skills.join(", ")} <span aria-hidden="true">·</span> <Icon name="flask" size={12} /> {t.cases} checks
                      </p>
                    </div>
                    <Button size="sm" variant="subtle" icon="plus" loading={adding === t.id} onclick={() => add(t.id)} aria-label={`Add ${t.name}`}>Add</Button>
                  </div>
                </Card>
              {/each}
            </div>
          </section>
        {/if}

        {#if store.runs.some((r) => r.agent)}
          <p class="mt-8 text-xs text-faint">Runs made by agents also appear in the run list, marked with the agent and its version.</p>
        {/if}
      {/if}
    </div>
  </div>

  <Modal bind:open={importOpen} title="Import an agent file" icon="download" size="md">
    <div class="space-y-4">
      <p class="text-sm text-muted">Paste the contents of an agent file (Markdown that starts with <span class="mono">---</span>). A new name makes an agent; an existing one gets a new version. Skills it names must already be in the library.</p>
      <Field label="Agent file" required>
        {#snippet children(f)}<Textarea id={f.id} rows={12} mono bind:value={file} placeholder={"---\nzorix: agent/1\nslug: my-agent\nname: \"My agent\"\n..."} />{/snippet}
      </Field>
      <Field label="Import under another name" hint="Optional: lowercase letters, digits and dashes.">
        {#snippet children(f)}<Input id={f.id} bind:value={rename} placeholder="my-agent-copy" />{/snippet}
      </Field>
    </div>
    {#snippet footer()}
      <Button onclick={() => (importOpen = false)}>Cancel</Button>
      <Button variant="primary" icon="check" loading={importing} disabled={!file.trim()} onclick={doImport}>Import</Button>
    {/snippet}
  </Modal>
{/if}
