<script lang="ts">
  import { t as tr } from "../../i18n/index.ts";
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
      toast.ok(editing ? tr("agents.teamEditor.saved", { name: saved.team.name }) : tr("agents.teamEditor.madeTheTeam", { name: saved.team.name }));
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

<Modal bind:open title={editing ? tr("agents.teamEditor.editTheTeam") : tr("agents.teamEditor.newTeam")} icon="users" size="lg">
  <div class="space-y-4">
    <div class="flex items-center gap-4">
      <Avatar {emoji} {color} size="xl" />
      <div class="min-w-0 flex-1 space-y-2">
        <div class="flex flex-wrap gap-1.5" role="group" aria-label={tr("agents.teamEditor.face")}>
          {#each emojis as e (e)}
            <button type="button" class="grid size-8 place-items-center rounded-lg text-lg transition hover:bg-hover {emoji === e ? 'bg-accent-soft ring-2 ring-accent' : ''}" aria-pressed={emoji === e} aria-label={tr("agents.teamEditor.face2", { e })} onclick={() => (emoji = e)}>{e}</button>
          {/each}
        </div>
        <div class="flex flex-wrap gap-1.5" role="group" aria-label={tr("agents.teamEditor.colour")}>
          {#each colors as c (c)}<Chip selected={color === c} onclick={() => (color = c)}>{c}</Chip>{/each}
        </div>
      </div>
    </div>
    <div class="grid gap-4 sm:grid-cols-2">
      <Field label={tr("agents.teamEditor.nameForTheCommands")} required hint={tr("agents.teamEditor.lowercaseLettersDigitsAnd")} error={error?.toLowerCase().includes("slug") ? error : null}>
        {#snippet children(f)}<Input id={f.id} bind:value={slug} maxlength={48} placeholder="research-desk" disabled={!!editing} mono />{/snippet}
      </Field>
      <Field label={tr("agents.teamEditor.displayName")}>{#snippet children(f)}<Input id={f.id} bind:value={name} maxlength={80} placeholder={tr("agents.teamEditor.researchDesk")} />{/snippet}</Field>
    </div>
    <Field label={tr("common.whatItIsFor")}>{#snippet children(f)}<Input id={f.id} bind:value={description} maxlength={280} placeholder={tr("agents.teamEditor.findsThingsOutAnd")} />{/snippet}</Field>
    <Field label={tr("agents.teamEditor.lead")} hint={tr("agents.teamEditor.theAgentThatReceives")}>
      {#snippet children(f)}
        <Select id={f.id} bind:value={lead}>
          {#each agents.cards as c (c.slug)}<option value={c.slug}>{c.emoji} {c.name} ({c.slug})</option>{/each}
        </Select>
      {/snippet}
    </Field>
    <fieldset>
      <legend class="mb-1.5 text-sm font-medium">{tr("agents.teamEditor.membersTheLeadMay")}</legend>
      {#if choices.length === 0}
        <p class="text-sm text-muted">{tr("agents.teamEditor.addMoreAgentsFirst")}</p>
      {:else}
        <div class="flex flex-wrap gap-2">
          {#each choices as c (c.slug)}
            <Chip selected={members.includes(c.slug)} onclick={() => toggle(c.slug)}>{c.emoji} {c.name}</Chip>
          {/each}
        </div>
      {/if}
      <p class="mt-2 text-xs text-faint">{tr("agents.teamEditor.membersOnlyGetWhat")}</p>
    </fieldset>
    {#if error}<p class="text-sm text-err" role="alert">{error}</p>{/if}
  </div>
  {#snippet footer()}
    <Button onclick={() => (open = false)}>{tr("common.cancel")}</Button>
    <Button variant="primary" icon="check" loading={saving} disabled={!slug.trim() || !lead} onclick={save}>{editing ? tr("common.save") : tr("agents.teamEditor.makeTheTeam")}</Button>
  {/snippet}
</Modal>
