<script lang="ts">
  import { t } from "../i18n/index.ts";
  import { tick } from "svelte";
  import { fade, scale } from "svelte/transition";

  import Icon from "./Icon.svelte";
  import Kbd from "./Kbd.svelte";
  import { cn } from "./cn.ts";
  import { fuzzyFilter } from "./fuzzy.ts";
  import { trapFocus } from "./actions.ts";

  export type Command = {
    id: string;
    label: string;
    group: string;
    icon?: string;
    hint?: string;
    keywords?: string;
    run: () => void;
  };

  /** ⌘K: jump to a screen, a run or a server, or do something, by typing a few letters. */
  let { open = $bindable(false), commands }: { open?: boolean; commands: Command[] } = $props();

  let query = $state("");
  let active = $state(0);
  let input = $state<HTMLInputElement | null>(null);
  const uid = $props.id();

  const matches = $derived(fuzzyFilter(commands, query, (c) => `${c.label} ${c.keywords ?? ""} ${c.group}`).slice(0, 40));
  const groups = $derived(
    matches.reduce<{ name: string; items: { command: Command; index: number }[] }[]>((acc, command, index) => {
      const last = acc[acc.length - 1];
      if (last && last.name === command.group) last.items.push({ command, index });
      else acc.push({ name: command.group, items: [{ command, index }] });
      return acc;
    }, []),
  );

  $effect(() => {
    if (open) {
      query = "";
      active = 0;
      void tick().then(() => input?.focus());
    }
  });
  $effect(() => {
    // Keep the highlighted row valid and in view when the list changes.
    if (active >= matches.length) active = Math.max(0, matches.length - 1);
    void tick().then(() => document.getElementById(`${uid}-opt-${active}`)?.scrollIntoView({ block: "nearest" }));
  });

  function choose(command: Command | undefined) {
    if (!command) return;
    open = false;
    command.run();
  }

  function onkeydown(event: KeyboardEvent) {
    if (!open) return;
    if (event.key === "Escape") {
      event.preventDefault();
      open = false;
    } else if (event.key === "ArrowDown") {
      event.preventDefault();
      active = matches.length ? (active + 1) % matches.length : 0;
    } else if (event.key === "ArrowUp") {
      event.preventDefault();
      active = matches.length ? (active - 1 + matches.length) % matches.length : 0;
    } else if (event.key === "Enter") {
      event.preventDefault();
      choose(matches[active]);
    }
  }
</script>

<svelte:window {onkeydown} />

{#if open}
  <div transition:fade={{ duration: 120 }} class="fixed inset-0 z-[60] grid items-start justify-items-center bg-black/35 px-6 pt-[14vh] backdrop-blur-[2px]">
    <button type="button" class="absolute inset-0 cursor-default" aria-label={t("common.close")} tabindex="-1" onclick={() => (open = false)}></button>
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t("ui.commandPalette.commandPalette")}
      transition:scale={{ start: 0.97, duration: 150 }}
      use:trapFocus
      class="glass relative flex max-h-[min(460px,70vh)] w-full max-w-xl flex-col overflow-hidden rounded-2xl border shadow-3"
    >
      <div class="flex items-center gap-2.5 border-b border-line-soft px-4">
        <Icon name="search" size={16} class="text-faint" />
        <input
          bind:this={input}
          bind:value={query}
          role="combobox"
          aria-expanded="true"
          aria-controls="{uid}-list"
          aria-activedescendant={matches.length ? `${uid}-opt-${active}` : undefined}
          aria-label={t("ui.commandPalette.typeACommand")}
          placeholder={t("ui.commandPalette.searchRunsWorkflowsServers")}
          autocomplete="off"
          spellcheck="false"
          class="h-12 min-w-0 flex-1 bg-transparent text-lg outline-none placeholder:text-faint"
        />
        <Kbd>esc</Kbd>
      </div>
      <div id="{uid}-list" role="listbox" aria-label={t("ui.commandPalette.commands")} class="min-h-0 flex-1 overflow-y-auto p-2">
        {#each groups as group (group.name)}
          <div role="group" aria-label={group.name}>
          <p class="px-2.5 pt-2 pb-1 text-2xs font-semibold tracking-wider text-muted uppercase" aria-hidden="true">{group.name}</p>
          {#each group.items as { command, index } (command.id)}
            <div
              id="{uid}-opt-{index}"
              role="option"
              tabindex="-1"
              aria-selected={index === active}
              onmousemove={() => (active = index)}
              onclick={() => choose(command)}
              onkeydown={(e) => e.key === "Enter" && choose(command)}
              class={cn("flex h-9 cursor-pointer items-center gap-2.5 rounded-lg px-2.5 text-md", index === active ? "bg-accent-soft text-fg" : "text-fg/90")}
            >
              <Icon name={command.icon ?? "chevron-right"} size={15} class={index === active ? "text-accent" : "text-faint"} />
              <span class="min-w-0 flex-1 truncate">{command.label}</span>
              {#if command.hint}<span class="truncate text-xs text-muted">{command.hint}</span>{/if}
            </div>
          {/each}
          </div>
        {:else}
          <p class="px-3 py-8 text-center text-base text-muted" role="presentation">{t("ui.commandPalette.nothingMatches", { query })}</p>
        {/each}
      </div>
    </div>
  </div>
{/if}
