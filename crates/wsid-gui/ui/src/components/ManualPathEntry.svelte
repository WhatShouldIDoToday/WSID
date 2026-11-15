<script lang="ts">
  import { onMount } from "svelte";

  interface Props {
    /** Visible, programmatically associated label for the path field. */
    label: string;
    /** Why the native picker is unavailable; shown inline and as an alert. */
    reason: string;
    submitLabel?: string;
    onSubmit: (path: string) => void | Promise<void>;
    onCancel: () => void;
  }

  let {
    label,
    reason,
    submitLabel = "Use path",
    onSubmit,
    onCancel,
  }: Props = $props();

  const inputId = `manual-path-${Math.random().toString(36).slice(2)}`;
  let value = $state("");
  let input: HTMLInputElement | undefined = $state();

  onMount(() => input?.focus());

  function submit() {
    const trimmed = value.trim();
    if (!trimmed) {
      input?.focus();
      return;
    }
    void onSubmit(trimmed);
  }

  function onKeydown(event: KeyboardEvent) {
    if (event.key === "Enter") {
      event.preventDefault();
      event.stopPropagation();
      submit();
    } else if (event.key === "Escape") {
      event.preventDefault();
      event.stopPropagation();
      onCancel();
    }
  }
</script>

<div class="manual-path-entry" role="group" aria-labelledby="{inputId}-label">
  <p class="field-error" role="alert">{reason}</p>
  <label for={inputId} id="{inputId}-label" class="field-label">{label}</label>
  <div class="actions-row">
    <input
      id={inputId}
      type="text"
      bind:this={input}
      bind:value
      onkeydown={onKeydown}
      autocomplete="off"
      spellcheck="false"
      placeholder="Absolute path"
    />
    <button type="button" class="btn-sm" onclick={submit}>{submitLabel}</button>
    <button type="button" class="btn-sm" onclick={onCancel}>Cancel</button>
  </div>
</div>

<style>
  .manual-path-entry {
    display: flex;
    flex-direction: column;
    gap: 0.4rem;
    margin-top: 0.5rem;
  }
  .manual-path-entry input {
    flex: 1 1 14rem;
    min-width: 0;
    font-family: var(--font-mono);
  }
</style>
