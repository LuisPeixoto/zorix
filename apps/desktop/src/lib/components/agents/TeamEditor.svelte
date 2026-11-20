<script lang="ts">
  import { type SaveTeam, errorText, teamSave } from "../../api.ts";
  import { agents } from "../../stores/agents.svelte.ts";
  import { toast } from "../../stores/toast.svelte.ts";
  import { Avatar, Button, Chip, Field, Input, Modal, Select } from "../../ui/index.ts";

  let { open = $bindable(false), editing = null }: { open: boolean; editing: string | null } = $props();

  let slug = $state("");
  let name = $state("");
  let description = $state("");
  let emoji = $state("🗂️");
  let color = $state("accent");
  let lead = $state("");
  let members = $state<string[]>([]);
  let error = $state<string | null>(null);
  let saving = $state(false);

  const emojis = ["🗂️", "🧭", "🔎", "📊", "✍️", "🛠️", "🧪", "🚀"];
  const colors = ["accent", "info", "ok", "warn", "err", "muted"];
  const choices = $derived(agents.cards.filter((c) => c.slug !== lead));

  // Load the team being edited each time the dialog opens.
  $effect(() => {
    if (!open) return;
    const t = editing ? agents.team(editing)?.team : null;
    slug = t?.slug ?? "";
    name = t?.name ?? "";
    description = t?.description ?? "";
    emoji = t?.avatar.emoji ?? "🗂️";
    color = t?.avatar.color ?? "accent";
    lead = t?.lead ?? agents.cards[0]?.slug ?? "";
    members = t?.members ?? [];
    error = null;
  });

  const toggle = (s: string) => (members = members.includes(s) ? members.filter((m) => m !== s) : [...members, s]);

  async function save() {
    saving = true;
    error = null;
    const team: SaveTeam = { slug: slug.trim(), name: name.trim() || slug.trim(), description, emoji, color, lead, members: members.filter((m) => m !== lead) };
    try {
      const saved = await teamSave(team);
      toast.ok(editing ? `Saved ${saved.team.name}.` : `Made the team ${saved.team.name}.`);
      await agents.refresh();
      open = false;
      if (!editing) await agents.openTeams(saved.team.slug);
    } catch (e) {
      error = errorText(e);
    } finally {
      saving = false;
    }
  }
</script>

<Modal bind:open title={editing ? "Edit the team" : "New team"} icon="users" size="lg">
  <div class="space-y-4">
    <div class="flex items-center gap-4">
      <Avatar {emoji} {color} size="xl" />
      <div class="min-w-0 flex-1 space-y-2">
        <div class="flex flex-wrap gap-1.5" role="group" aria-label="Face">
          {#each emojis as e (e)}
            <button type="button" class="grid size-8 place-items-center rounded-lg text-lg transition hover:bg-hover {emoji === e ? 'bg-accent-soft ring-2 ring-accent' : ''}" aria-pressed={emoji === e} aria-label={`Face ${e}`} onclick={() => (emoji = e)}>{e}</button>
          {/each}
        </div>
        <div class="flex flex-wrap gap-1.5" role="group" aria-label="Colour">
          {#each colors as c (c)}<Chip selected={color === c} onclick={() => (color = c)}>{c}</Chip>{/each}
        </div>
      </div>
    </div>
    <div class="grid gap-4 sm:grid-cols-2">
      <Field label="Name (for the commands)" required hint="Lowercase letters, digits and dashes." error={error?.toLowerCase().includes("slug") ? error : null}>
        {#snippet children(f)}<Input id={f.id} bind:value={slug} maxlength={48} placeholder="research-desk" disabled={!!editing} mono />{/snippet}
      </Field>
      <Field label="Display name">{#snippet children(f)}<Input id={f.id} bind:value={name} maxlength={80} placeholder="Research desk" />{/snippet}</Field>
    </div>
    <Field label="What it is for">{#snippet children(f)}<Input id={f.id} bind:value={description} maxlength={280} placeholder="Finds things out and writes them up." />{/snippet}</Field>
    <Field label="Lead" hint="The agent that receives the work and decides whom to ask.">
      {#snippet children(f)}
        <Select id={f.id} bind:value={lead}>
          {#each agents.cards as c (c.slug)}<option value={c.slug}>{c.emoji} {c.name} ({c.slug})</option>{/each}
        </Select>
      {/snippet}
    </Field>
    <fieldset>
      <legend class="mb-1.5 text-sm font-medium">Members the lead may ask</legend>
      {#if choices.length === 0}
        <p class="text-sm text-muted">Add more agents first: a team needs somebody to ask.</p>
      {:else}
        <div class="flex flex-wrap gap-2">
          {#each choices as c (c.slug)}
            <Chip selected={members.includes(c.slug)} onclick={() => toggle(c.slug)}>{c.emoji} {c.name}</Chip>
          {/each}
        </div>
      {/if}
      <p class="mt-2 text-xs text-faint">Members only get what the lead has: an agent asked for help can never do more than the one who asked.</p>
    </fieldset>
    {#if error}<p class="text-sm text-err" role="alert">{error}</p>{/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>Cancel</Button>
    <Button variant="primary" icon="check" loading={saving} disabled={!slug.trim() || !lead} onclick={save}>{editing ? "Save" : "Make the team"}</Button>
  {/snippet}
</Modal>
