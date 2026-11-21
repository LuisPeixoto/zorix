<script lang="ts">
  import type { Snippet } from "svelte";

  import Icon from "./Icon.svelte";
  import Kbd from "./Kbd.svelte";
  import { cn } from "./cn.ts";
  import { tooltip } from "./actions.ts";

  /** Where a person writes to an agent. Enter sends, Shift+Enter starts a new line, and it grows with the text. */
  let {
    value = $bindable(""),
    onsend,
    onstop,
    busy = false,
    disabled = false,
    placeholder = "Write a message…",
    label = "Message",
    maxRows = 8,
    tools,
    class: klass = "",
  }: {
    value?: string;
    onsend: (text: string) => void;
    onstop?: () => void;
    busy?: boolean;
    disabled?: boolean;
    placeholder?: string;
    label?: string;
    maxRows?: number;
    tools?: Snippet;
    class?: string;
  } = $props();

  const uid = $props.id();
  let field: HTMLTextAreaElement | undefined = $state();
  const ready = $derived(value.trim().length > 0 && !disabled && !busy);

  $effect(() => {
    void value;
    if (!field) return;
    field.style.height = "auto";
    const line = parseFloat(getComputedStyle(field).lineHeight) || 20;
    field.style.height = `${Math.min(field.scrollHeight, line * maxRows)}px`;
  });

  function send() {
    if (!ready) return;
    const text = value.trim();
    value = "";
    onsend(text);
  }

  function onkeydown(event: KeyboardEvent) {
    // Enter while an input method is composing a character must not send.
    if (event.key === "Enter" && !event.shiftKey && !event.isComposing) {
      event.preventDefault();
      send();
    }
  }
</script>

<div class={cn("raised rounded-2xl border border-line bg-panel transition duration-150 focus-within:border-accent focus-within:ring-3 focus-within:ring-accent-soft", klass)}>
  <label class="sr-only" for={uid}>{label}</label>
  <textarea
    id={uid}
    bind:this={field}
    bind:value
    {onkeydown}
    {placeholder}
    {disabled}
    rows="1"
    class="block max-h-60 w-full resize-none bg-transparent px-4 pt-3 pb-1 text-base leading-relaxed text-fg outline-none placeholder:text-faint"
  ></textarea>
  <div class="flex items-center gap-2 px-2.5 pb-2.5">
    <div class="flex min-w-0 flex-1 items-center gap-2">{@render tools?.()}</div>
    <span class="hidden items-center gap-1 text-xs text-faint sm:flex"><Kbd>↵</Kbd> send <Kbd>⇧↵</Kbd> new line</span>
    {#if busy && onstop}
      <button type="button" onclick={onstop} use:tooltip={"Stop"} aria-label="Stop" class="grid size-8 place-items-center rounded-full border border-line bg-panel-2 text-fg transition hover:bg-hover active:scale-95">
        <Icon name="stop" size={14} />
      </button>
    {:else}
      <button
        type="button"
        onclick={send}
        disabled={!ready}
        aria-label="Send"
        class={cn(
          "grid size-8 place-items-center rounded-xl transition duration-150 active:scale-95",
          ready ? "surface-send hover:brightness-105" : "bg-hover text-faint",
        )}
      >
        <Icon name="arrow-up" size={16} stroke={2.2} />
      </button>
    {/if}
  </div>
</div>
