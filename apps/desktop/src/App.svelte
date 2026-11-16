<script lang="ts">
  import { onMount } from "svelte";

  import { store } from "./lib/stores/runs.svelte.ts";
  import { theme } from "./lib/stores/theme.svelte.ts";
  import NewRun from "./lib/components/NewRun.svelte";
  import Providers from "./lib/components/Providers.svelte";
  import RunView from "./lib/components/RunView.svelte";
  import Sidebar from "./lib/components/shell/Sidebar.svelte";
  import Toolbar from "./lib/components/shell/Toolbar.svelte";
  import { Toasts } from "./lib/ui/index.ts";

  onMount(() => {
    theme.init();
    void store.init();
  });
</script>

<div class="flex h-full flex-col">
  <Toolbar />
  <div class="flex min-h-0 flex-1">
    <Sidebar />
    <main class="min-w-0 flex-1 bg-bg">
      {#if store.info?.error}
        <div class="mx-auto max-w-xl p-10" role="alert">
          <h2 class="text-2xl font-semibold text-err">ZORIX could not start</h2>
          <p class="selectable mt-2">{store.info.error}</p>
        </div>
      {:else if store.view === "providers"}
        <Providers />
      {:else if store.selectedId}
        {#if store.error}<p class="border-b border-line bg-err/10 px-6 py-2 text-err" role="alert">{store.error}</p>{/if}
        <RunView />
      {:else}
        <NewRun />
      {/if}
    </main>
  </div>
</div>

<Toasts />
