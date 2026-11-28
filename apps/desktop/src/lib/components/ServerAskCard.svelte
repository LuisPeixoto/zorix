<script lang="ts">
  import { untrack } from "svelte";
  import { t } from "../i18n/index.ts";
  import type { PendingAsk } from "../api.ts";
  import { type Typed, emptyForm, formValues, hostOf } from "../askForm.ts";
  import { asks } from "../stores/asks.svelte.ts";
  import { Badge, Button, Callout, Checkbox, Field, Input, KeyValue, Select } from "../ui/index.ts";

  /** One request of an MCP server, waiting for you. Everything the server wrote is shown as text, never as markup. */
  let { ask }: { ask: PendingAsk } = $props();

  const request = $derived(ask.ask);
  // Keyed by the request's id: the form is filled once and kept while the list is read again.
  let typed = $state<Typed>(untrack(() => (ask.ask.kind === "person" && ask.ask.mode === "form" ? emptyForm(ask.ask.fields) : {})));
  let problem = $state<string | null>(null);

  const yes = (values: Record<string, unknown> = {}) => void asks.answer(ask, { answer: "yes", values });
  const no = () => void asks.answer(ask, { answer: "no" });

  function send() {
    if (request.kind !== "person" || request.mode !== "form") return;
    const filled = formValues(request.fields, typed);
    if (filled.problem) {
      problem = filled.problem.name;
      return;
    }
    yes(filled.values);
  }

  const title = $derived(
    request.kind === "model"
      ? t("serverAsk.modelTitle", { server: ask.server })
      : request.mode === "form"
        ? t("serverAsk.formTitle", { server: ask.server })
        : t("serverAsk.linkTitle", { server: ask.server }),
  );
  const cut = (text: string) => (text.length > 2000 ? `${text.slice(0, 2000)}…` : text);
</script>

<Callout tone="warn" icon="shield" {title}>
  <div class="flex flex-wrap items-center gap-2">
    <Badge tone={request.kind === "person" && request.mode === "form" ? "accent" : "warn"}>{request.kind === "model" ? t("serverAsk.fromTheServer") : request.mode === "form" ? t("serverAsk.question") : t("serverAsk.opensAPage")}</Badge>
    <span class="mono min-w-0 truncate text-xs text-muted">{ask.tool}</span>
  </div>

  {#if request.kind === "model"}
    <p class="mt-2">{t("serverAsk.modelWhy")}</p>
    <div class="mt-3">
      <KeyValue
        items={[
          { key: t("serverAsk.model"), value: ask.model, mono: true },
          { key: t("serverAsk.mostTokens"), value: String(request.max_tokens) },
        ]}
      />
    </div>
    <pre class="mono selectable mt-3 max-h-56 overflow-auto rounded-lg border border-line bg-panel p-3 text-sm break-words whitespace-pre-wrap text-fg">{#if request.system}{t("serverAsk.system")}: {cut(request.system)}
{/if}{#each request.messages as message, i (i)}{message.role === "user" ? t("serverAsk.user") : t("serverAsk.assistant")}: {cut(message.text)}
{/each}</pre>
  {:else if request.mode === "form"}
    <p class="mt-2">{t("serverAsk.formWhy")}</p>
    <p class="selectable mt-3 text-md break-words text-fg">{request.message}</p>
    <div class="mt-3 flex flex-col gap-3">
      {#each request.fields as field (field.name)}
        {@const label = field.title ?? field.name}
        {#if field.kind.type === "boolean"}
          <Checkbox checked={typed[field.name] === true} onchange={(e) => (typed[field.name] = e.currentTarget.checked)} description={field.description}>{label}</Checkbox>
        {:else}
          <Field {label} hint={field.description} error={problem === field.name ? t("serverAsk.doesNotFit") : undefined}>
            {#snippet children(f)}
              {#if field.kind.type === "choice"}
                <Select id={f.id} value={String(typed[field.name] ?? "")} onchange={(e) => (typed[field.name] = e.currentTarget.value)}>
                  <option value="">{t("serverAsk.choose")}</option>
                  {#each field.kind.options as option (option)}<option value={option}>{option}</option>{/each}
                </Select>
              {:else}
                <Input id={f.id} value={String(typed[field.name] ?? "")} inputmode={field.kind.type === "text" ? "text" : "decimal"} autocomplete="off" oninput={(e) => (typed[field.name] = e.currentTarget.value)} />
              {/if}
            {/snippet}
          </Field>
        {/if}
      {/each}
    </div>
    <p class="mt-2 text-xs text-muted">{t("serverAsk.neverSecrets")}</p>
  {:else}
    <p class="mt-2">{t("serverAsk.linkWhy")}</p>
    <p class="selectable mt-3 text-md break-words text-fg">{request.message}</p>
    <div class="mt-3">
      <KeyValue
        items={[
          { key: t("serverAsk.opens"), value: request.url, mono: true },
          { key: t("serverAsk.domain"), value: hostOf(request.url) },
        ]}
      />
    </div>
  {/if}

  {#snippet actions()}
    {#if request.kind === "model"}
      <Button variant="primary" icon="check" disabled={asks.busy} onclick={() => yes()}>{t("serverAsk.allowOnce")}</Button>
      <Button icon="x" disabled={asks.busy} onclick={no}>{t("common.refuse")}</Button>
    {:else if request.mode === "form"}
      <Button variant="primary" disabled={asks.busy} onclick={send}>{t("serverAsk.send")}</Button>
      <Button disabled={asks.busy} onclick={no}>{t("serverAsk.decline")}</Button>
    {:else}
      <Button variant="primary" icon="external" disabled={asks.busy} onclick={() => yes()}>{t("serverAsk.openInBrowser")}</Button>
      <Button disabled={asks.busy} onclick={no}>{t("serverAsk.decline")}</Button>
    {/if}
  {/snippet}
</Callout>
