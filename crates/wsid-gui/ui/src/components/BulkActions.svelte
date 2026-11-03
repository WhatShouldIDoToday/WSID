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
  import Select from "./ui/Select.svelte";

  interface Props {
    ids: string[];
    knownTags: string[];
    knownUsers: string[];
    knownProjects: string[];
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
    knownProjects,
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

  /** Runs `fn` with the chosen value. The action selects stay controlled at "" so they reset to their placeholder. */
  function pick(value: string, fn: (value: string) => void) {
    if (value) fn(value);
  }

  let statusOptions = $derived(
    STATUS_ORDER.map((st) => ({ value: st, label: STATUS_LABELS[st] })),
  );
  let userOptions = $derived(
    knownUsers.map((u) => ({ value: u, label: `@${u}` })),
  );
  let projectOptions = $derived(
    knownProjects.map((p) => ({ value: p, label: p })),
  );
  let importanceOptions = [1, 2, 3, 4, 5].map((n) => ({
    value: String(n),
    label: `${n}/5`,
  }));
  let urgentOptions = [
    { value: "on", label: "Urgent on" },
    { value: "off", label: "Urgent off" },
  ];
  let tagOptions = $derived(
    knownTags.map((t) => ({ value: t, label: `#${t}` })),
  );

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

  <Select
    size="sm"
    ariaLabel="Set status"
    placeholder="Status…"
    value=""
    disabled={busy}
    options={statusOptions}
    onValueChange={(v) =>
      pick(v, (x) => void apply({ status: x as TaskStatus }))}
  />

  <Select
    size="sm"
    ariaLabel="Set assignee"
    placeholder="Assignee…"
    value=""
    disabled={busy}
    options={userOptions}
    onValueChange={(v) => pick(v, (x) => void apply({ assignee: x }))}
  />

  <Select
    size="sm"
    ariaLabel="Set project"
    placeholder="Project…"
    value=""
    disabled={busy}
    options={projectOptions}
    onValueChange={(v) => pick(v, (x) => void apply({ project: x }))}
  />

  <Select
    size="sm"
    ariaLabel="Set importance"
    placeholder="Importance…"
    value=""
    disabled={busy}
    options={importanceOptions}
    onValueChange={(v) => pick(v, (x) => void apply({ importance: Number(x) }))}
  />

  <Select
    size="sm"
    ariaLabel="Set urgent override"
    placeholder="Urgent…"
    value=""
    disabled={busy}
    options={urgentOptions}
    onValueChange={(v) =>
      pick(v, (x) => void apply({ urgent_override: x === "on" }))}
  />

  {#if knownTags.length > 0}
    <Select
      size="sm"
      ariaLabel="Add tag"
      placeholder="Add tag…"
      value=""
      disabled={busy}
      options={tagOptions}
      onValueChange={(v) => pick(v, (x) => void apply({ add_tags: [x] }))}
    />

    <Select
      size="sm"
      ariaLabel="Remove tag"
      placeholder="Remove tag…"
      value=""
      disabled={busy}
      options={tagOptions}
      onValueChange={(v) => pick(v, (x) => void apply({ remove_tags: [x] }))}
    />
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
