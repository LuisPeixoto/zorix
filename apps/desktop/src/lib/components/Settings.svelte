<script lang="ts">
  import { type Mode, MODE_LABEL } from "../theme.ts";
  import { store } from "../stores/runs.svelte.ts";
  import { theme } from "../stores/theme.svelte.ts";
  import { BrandMark, Card, KeyValue, PageHeader, Segmented, Tabs } from "../ui/index.ts";

  let tab = $state("general");
  let mode = $state<Mode>(theme.mode);
  $effect(() => {
    if (mode !== theme.mode) theme.set(mode);
  });
</script>

<div class="mx-auto flex h-full max-w-4xl flex-col gap-5 overflow-y-auto p-6 [&>*]:shrink-0">
  <PageHeader title="Settings" subtitle="How ZORIX looks and where it keeps things. What an agent may do is set by each run and each agent." glow class="mb-1" />

  <Tabs
    tabs={[
      { value: "general", label: "General" },
      { value: "appearance", label: "Appearance" },
      { value: "about", label: "About" },
    ]}
    bind:value={tab}
    label="Settings"
  />

  {#if tab === "general"}
    <Card title="Data" icon="database">
      <KeyValue
        items={[
          { key: "Data folder", value: store.info?.data_dir, mono: true },
          { key: "Version", value: store.info?.version, mono: true },
        ]}
      />
      <p class="mt-3 text-sm text-muted">Runs, memory, agents and settings live in this folder, on this computer. Keys are kept in the system keychain, never in it.</p>
    </Card>
  {:else if tab === "appearance"}
    <Card title="Theme" icon="sun">
      <p class="mb-3 text-base text-muted">Light is the default. System follows your computer's setting and changes with it.</p>
      <Segmented
        label="Theme"
        bind:value={mode}
        options={[
          { value: "light", label: MODE_LABEL.light, icon: "sun" },
          { value: "dark", label: MODE_LABEL.dark, icon: "moon" },
          { value: "system", label: MODE_LABEL.system, icon: "monitor" },
        ]}
      />
    </Card>
  {:else}
    <Card>
      <div class="flex flex-col items-center gap-3 px-4 py-8 text-center">
        <BrandMark size="lg" />
        <p class="text-2xl font-semibold tracking-tight">ZORIX</p>
        <p class="max-w-[46ch] text-base text-muted">A harness for AI: it decides what a model may see and do, stops to ask a person before anything changes, and records every step.</p>
        <p class="mono text-sm text-muted">Version {store.info?.version ?? ""}</p>
        <p class="text-xs text-muted">MIT licence. Fonts and brand marks keep their own licences; see THIRD-PARTY.md in the repository.</p>
      </div>
    </Card>
  {/if}
</div>
