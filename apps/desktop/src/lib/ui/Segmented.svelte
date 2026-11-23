<script lang="ts">
  import { t } from "../i18n/index.ts";
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  type Option = { value: string; label: string; icon?: string };
  let { options, value = $bindable(), label = t("ui.segmented.chooseOne"), class: klass = "" }: { options: Option[]; value: string; label?: string; class?: string } = $props();

  const index = $derived(Math.max(0, options.findIndex((o) => o.value === value)));

  function onkey(event: KeyboardEvent) {
    const step = event.key === "ArrowRight" ? 1 : event.key === "ArrowLeft" ? -1 : 0;
    if (!step) return;
    event.preventDefault();
    value = options[(index + step + options.length) % options.length].value;
  }
</script>

<div
  role="radiogroup"
  aria-label={label}
  tabindex="-1"
  onkeydown={onkey}
  class={cn("relative grid rounded-xl border border-line-soft bg-hover p-0.75", klass)}
  style:grid-template-columns="repeat({options.length}, minmax(0, 1fr))"
>
  <span
    class="pointer-events-none absolute top-0.75 bottom-0.75 left-0.75 rounded-md bg-panel shadow-raised transition-transform duration-200 ease-out"
    style:width="calc((100% - 1.5px * 4) / {options.length})"
    style:transform="translateX({index * 100}%)"
    aria-hidden="true"
  ></span>
  {#each options as option, i (option.value)}
    <button
      type="button"
      role="radio"
      aria-checked={value === option.value}
      tabindex={value === option.value ? 0 : -1}
      onclick={() => (value = option.value)}
      class={cn(
        "relative z-10 inline-flex h-6.5 items-center justify-center gap-1.5 rounded-md px-2.5 text-sm font-medium transition-colors duration-150",
        value === option.value ? "text-fg" : "text-muted hover:text-fg",
      )}
      data-index={i}
    >
      {#if option.icon}<Icon name={option.icon} size={14} />{/if}{option.label}
    </button>
  {/each}
</div>
