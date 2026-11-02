<script lang="ts">
  import type { Snippet } from "svelte";
  import {
    type BulkEditOutcome,
    type BulkTaskPatch,
    type TaskStatus,
    STATUS_LABELS,
    STATUS_ORDER,
    bulkEditTasks,
    bulkRemoveTasks,
    summarizeBulk,
  } from "../lib/ipc";
  import ConfirmModal from "./ConfirmModal.svelte";

  interface Props {
    ids: string[];
    knownTags: string[];
    knownUsers: string[];
    onClear: () => void;
    onDone: (outcome: BulkEditOutcome) => void;
    announce: (msg: string) => void;
    /** Extra view-specific controls (e.g. quadrant move). */
    children?: Snippet;
  }

  let {
    ids,
    knownTags,
    knownUsers,
    onClear,
    onDone,
    announce,
    children,
  }: Props = $props();

  let busy = $state(false);
  let confirmDelete = $state(false);

  async function apply(patch: BulkTaskPatch) {
    if (busy || ids.length === 0) return;
    busy = true;
    try {
      const outcome = await bulkEditTasks(ids, patch);
      announce(summarizeBulk("Updated", outcome));
      onDone(outcome);
    } catch (err) {
      announce(`Bulk edit failed: ${String(err)}`);
    } finally {
      busy = false;
    }
  }

  async function removeAll() {
    if (busy) return;
    busy = true;
    try {
      const outcome = await bulkRemoveTasks(ids);
      announce(summarizeBulk("Deleted", outcome));
      onDone(outcome);
    } catch (err) {
      announce(`Bulk delete failed: ${String(err)}`);
    } finally {
      busy = false;
    }
  }

  /** Runs `fn` with the chosen value, then resets the select to its placeholder. */
  function pick(e: Event, fn: (value: string) => void) {
    const el = e.currentTarget as HTMLSelectElement;
    const value = el.value;
    el.value = "";
    if (value) fn(value);
  }

  function handleKeydown(e: KeyboardEvent) {
    if (e.key !== "Escape" || e.defaultPrevented || confirmDelete) return;
    const t = e.target as HTMLElement | null;
    if (t?.closest("input[type=search], input[type=text], textarea")) return;
    onClear();
  }
</script>

<svelte:window onkeydown={handleKeydown} />

<div class="bulk-bar" role="region" aria-label="Bulk actions">
  <strong class="bulk-count" aria-live="polite">{ids.length} selected</strong>

  <select
    aria-label="Set status"
    disabled={busy}
    onchange={(e) => pick(e, (v) => void apply({ status: v as TaskStatus }))}
  >
    <option value="">Status…</option>
    {#each STATUS_ORDER as st}
      <option value={st}>{STATUS_LABELS[st]}</option>
    {/each}
  </select>

  <select
    aria-label="Set assignee"
    disabled={busy}
    onchange={(e) => pick(e, (v) => void apply({ assignee: v }))}
  >
    <option value="">Assignee…</option>
    {#each knownUsers as u}
      <option value={u}>@{u}</option>
    {/each}
  </select>

  <select
    aria-label="Set importance"
    disabled={busy}
    onchange={(e) => pick(e, (v) => void apply({ importance: Number(v) }))}
  >
    <option value="">Importance…</option>
    {#each [1, 2, 3, 4, 5] as n}
      <option value={n}>{n}/5</option>
    {/each}
  </select>

  <select
    aria-label="Set urgent override"
    disabled={busy}
    onchange={(e) =>
      pick(e, (v) => void apply({ urgent_override: v === "on" }))}
  >
    <option value="">Urgent…</option>
    <option value="on">Urgent on</option>
    <option value="off">Urgent off</option>
  </select>

  {#if knownTags.length > 0}
    <select
      aria-label="Add tag"
      disabled={busy}
      onchange={(e) => pick(e, (v) => void apply({ add_tags: [v] }))}
    >
      <option value="">Add tag…</option>
      {#each knownTags as t}
        <option value={t}>#{t}</option>
      {/each}
    </select>

    <select
      aria-label="Remove tag"
      disabled={busy}
      onchange={(e) => pick(e, (v) => void apply({ remove_tags: [v] }))}
    >
      <option value="">Remove tag…</option>
      {#each knownTags as t}
        <option value={t}>#{t}</option>
      {/each}
    </select>
  {/if}

  {@render children?.()}

  <span class="bulk-spacer"></span>
  <button
    type="button"
    class="btn-sm btn-danger"
    disabled={busy}
    onclick={() => (confirmDelete = true)}
  >
    Delete…
  </button>
  <button
    type="button"
    class="btn-sm"
    aria-keyshortcuts="Escape"
    title="Clear selection (Esc)"
    onclick={onClear}
  >
    Clear
  </button>
</div>

<ConfirmModal
  bind:open={confirmDelete}
  title={`Delete ${ids.length} ${ids.length === 1 ? "task" : "tasks"}?`}
  description="The selected tasks and their notes will be permanently deleted. This cannot be undone."
  confirmLabel="Delete"
  onConfirm={() => void removeAll()}
  onCancel={() => (confirmDelete = false)}
/>
