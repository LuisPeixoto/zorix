<script lang="ts">
  import type { Overview } from "../../api.ts";
  import { Avatar, Badge } from "../../ui/index.ts";
  import { STATUS_TEXT, evalText, levelLabel, money, onTimeText } from "../../workforce.ts";

  let { overview, selected, onselect }: { overview: Overview; selected: string | null; onselect: (id: string) => void } = $props();

  type Row = { v: Overview["workers"][number]; depth: number };
  // Each worker under who it reports to, depth first, so the table reads like the chart.
  const rows = $derived.by(() => {
    const out: Row[] = [];
    const walk = (parent: string | null, depth: number, seen: Set<string>) => {
      for (const v of overview.workers.filter((w) => w.worker.reports_to === parent)) {
        if (seen.has(v.worker.agent)) continue;
        seen.add(v.worker.agent);
        out.push({ v, depth });
        walk(v.worker.agent, depth + 1, seen);
      }
    };
    walk(null, 0, new Set());
    return out;
  });
  const dept = (slug: string) => overview.settings.departments.find((d) => d.slug === slug)?.name ?? slug;
  const nameOf = (slug: string | null) => (slug === null ? "You" : (overview.workers.find((w) => w.worker.agent === slug)?.name ?? slug));
</script>

<div class="overflow-hidden rounded-2xl border border-line bg-panel shadow-card">
  <div class="overflow-x-auto">
    <table class="w-full min-w-[760px] text-left text-base">
      <thead>
        <tr class="border-b border-line-soft bg-panel-2 text-2xs font-semibold tracking-wider text-faint uppercase">
          <th scope="col" class="px-3.5 py-2.5 font-semibold">Worker</th>
          <th scope="col" class="px-3.5 py-2.5 font-semibold">Department</th>
          <th scope="col" class="px-3.5 py-2.5 font-semibold">Reports to</th>
          <th scope="col" class="px-3.5 py-2.5 font-semibold">Autonomy</th>
          <th scope="col" class="px-3.5 py-2.5 font-semibold">Tasks</th>
          <th scope="col" class="px-3.5 py-2.5 font-semibold">On time</th>
          <th scope="col" class="px-3.5 py-2.5 font-semibold">Eval</th>
          <th scope="col" class="px-3.5 py-2.5 font-semibold">Week</th>
        </tr>
      </thead>
      <tbody>
        {#each rows as { v, depth } (v.worker.agent)}
          {@const w = v.worker}
          <tr class="cursor-pointer border-b border-line-soft last:border-b-0 hover:bg-hover {selected === w.agent ? 'bg-hover' : ''}" onclick={() => onselect(w.agent)}>
            <td class="px-3.5 py-2">
              <span class="flex items-center gap-2" style:padding-left="{depth * 18}px">
                <Avatar emoji={v.avatar.emoji} color={v.avatar.color} status={v.status} size="sm" label={v.status ? `${v.name}: ${STATUS_TEXT[v.status]}` : undefined} />
                <span class="min-w-0 leading-tight">
                  <button type="button" class="block rounded font-medium outline-none focus-visible:ring-2 focus-visible:ring-accent/60" onclick={(e) => { e.stopPropagation(); onselect(w.agent); }}>{v.name}</button>
                  <span class="block text-xs text-muted">{w.manager ? "Virtual manager" : w.role}</span>
                </span>
              </span>
            </td>
            <td class="px-3.5 py-2">{dept(w.department)}</td>
            <td class="px-3.5 py-2">
              <span class="flex items-center gap-1.5">
                {#if w.reports_to === null}<Avatar human size="xs" label="You" />{:else}<Avatar emoji={overview.workers.find((o) => o.worker.agent === w.reports_to)?.avatar.emoji ?? "🤖"} color={overview.workers.find((o) => o.worker.agent === w.reports_to)?.avatar.color ?? "accent"} size="xs" />{/if}
                {nameOf(w.reports_to)}
              </span>
            </td>
            <td class="px-3.5 py-2"><Badge tone={w.autonomy === "suggest" ? "muted" : w.autonomy === "ask" ? "info" : "accent"}>{levelLabel(w.autonomy)}</Badge></td>
            <td class="tnum px-3.5 py-2">{v.figures.tasks}</td>
            <td class="tnum px-3.5 py-2">{onTimeText(v.figures.on_time_done, v.figures.on_time_all)}</td>
            <td class="tnum px-3.5 py-2">{evalText(v.own.eval)}</td>
            <td class="tnum px-3.5 py-2">{money(v.figures.week_micros)}</td>
          </tr>
        {/each}
      </tbody>
    </table>
  </div>
</div>
