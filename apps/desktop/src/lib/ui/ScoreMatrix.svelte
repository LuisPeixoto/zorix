<script lang="ts">
  import Icon from "./Icon.svelte";
  import { cn } from "./cn.ts";

  export type Cell = { state: "pass" | "fail" | "none"; note?: string };
  export type MatrixRun = { id: string; label: string; sub?: string };

  /** Cases down the side, runs across the top. Each cell says pass or fail in words and in shape, never in colour alone. */
  let {
    cases,
    runs,
    cell,
    onselect,
    selected,
    class: klass = "",
  }: {
    cases: string[];
    runs: MatrixRun[];
    cell: (caseName: string, runId: string) => Cell;
    onselect?: (caseName: string, runId: string) => void;
    selected?: { caseName: string; runId: string } | null;
    class?: string;
  } = $props();

  const passed = (runId: string) => cases.filter((c) => cell(c, runId).state === "pass").length;
  const ran = (runId: string) => cases.filter((c) => cell(c, runId).state !== "none").length;
</script>

<div class={cn("overflow-x-auto rounded-xl border border-line bg-panel", klass)}>
  <table class="w-full border-collapse text-sm">
    <thead>
      <tr class="bg-panel-2">
        <th scope="col" class="sticky left-0 z-10 min-w-48 bg-panel-2 px-3 py-2 text-left text-xs font-semibold text-muted">Case</th>
        {#each runs as run (run.id)}
          <th scope="col" class="min-w-28 px-3 py-2 text-left align-bottom">
            <span class="block text-xs font-semibold">{run.label}</span>
            {#if run.sub}<span class="block text-2xs font-normal text-faint">{run.sub}</span>{/if}
          </th>
        {/each}
      </tr>
    </thead>
    <tbody>
      {#each cases as name (name)}
        <tr class="border-t border-line-soft">
          <th scope="row" class="sticky left-0 z-10 bg-panel px-3 py-2 text-left font-medium"><span class="block max-w-64 truncate" title={name}>{name}</span></th>
          {#each runs as run (run.id)}
            {@const c = cell(name, run.id)}
            {@const on = selected?.caseName === name && selected?.runId === run.id}
            <td class="px-2 py-1.5">
              <button
                type="button"
                disabled={c.state === "none" || !onselect}
                onclick={() => onselect?.(name, run.id)}
                aria-pressed={onselect ? on : undefined}
                class={cn(
                  "inline-flex h-7 min-w-18 items-center gap-1.5 rounded-lg px-2 text-xs font-medium transition duration-150",
                  c.state === "pass" && "bg-ok/12 text-ok",
                  c.state === "fail" && "bg-err/12 text-err",
                  c.state === "none" && "text-faint",
                  c.state !== "none" && onselect && "hover:brightness-95",
                  on && "ring-2 ring-accent",
                )}
                title={c.note}
              >
                {#if c.state === "pass"}<Icon name="check" size={13} stroke={2.4} />Pass{:else if c.state === "fail"}<Icon name="x" size={13} stroke={2.4} />Fail{:else}<span aria-hidden="true">–</span><span class="sr-only">Not run</span>{/if}
              </button>
            </td>
          {/each}
        </tr>
      {/each}
    </tbody>
    <tfoot>
      <tr class="border-t border-line bg-panel-2">
        <th scope="row" class="sticky left-0 bg-panel-2 px-3 py-2 text-left text-xs font-semibold text-muted">Passed</th>
        {#each runs as run (run.id)}
          <td class="tnum px-3 py-2 text-xs font-semibold">{passed(run.id)} / {ran(run.id)}</td>
        {/each}
      </tr>
    </tfoot>
  </table>
</div>
