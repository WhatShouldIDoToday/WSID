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
    describeUserNotice,
    summarizeBulk,
    userKindMismatch,
    validateProject,
    validateUserRef,
    withUserNotices,
  } from "../lib/ipc";
  import ConfirmModal from "./ConfirmModal.svelte";
  import Select from "./ui/Select.svelte";
  import UserRefInput from "./UserRefInput.svelte";

  interface Props {
    ids: string[];
    knownTags: string[];
    knownUsers: string[];
    knownProjects: string[];
    userKinds?: Record<string, string>;
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
    userKinds = {},
    onClear,
    onDone,
    announce,
    children,
  }: Props = $props();

  let busy = $state(false);
  let confirmDelete = $state(false);

  // Assignee and project accept new names: typed text, suggestions from the datalists.
  let assigneeDraft = $state("");
  let projectDraft = $state("");
  let assigneeError = $state<string | null>(null);
  /** Typed-reference notices from the last apply (e.g. a kind that was ignored). */
  let userNotice = $state<{ text: string; warning: boolean } | null>(null);
  let projectError = $state<string | null>(null);

  /** The mismatched text the user has already confirmed with a second Enter. */
  let confirmedMismatch = $state<string | null>(null);

  function commitAssignee() {
    const raw = assigneeDraft.trim();
    if (!raw || busy) return;
    const check = validateUserRef(raw);
    if (check.error || !check.normalized) {
      assigneeError = check.error;
      announce(`Assignee: ${check.error ?? "invalid"}`);
      return;
    }
    const mismatch = userKindMismatch(raw, knownUsers, userKinds);
    if (mismatch && confirmedMismatch !== raw) {
      confirmedMismatch = raw;
      assigneeError = `${mismatch} Press Enter again to assign anyway.`;
      announce(assigneeError);
      return;
    }
    assigneeError = null;
    confirmedMismatch = null;
    assigneeDraft = "";
    void apply({ assignee: check.normalized });
  }

  function commitProject() {
    const raw = projectDraft.trim();
    if (!raw || busy) return;
    const check = validateProject(raw);
    if (check.error || !check.normalized) {
      projectError = check.error;
      announce(`Project: ${check.error ?? "invalid"}`);
      return;
    }
    projectError = null;
    projectDraft = "";
    void apply({ project: check.normalized });
  }

  /** Enter applies; Escape clears the draft without clearing the selection. */
  function handleTextKeydown(
    e: KeyboardEvent,
    commit: () => void,
    clear: () => void,
  ) {
    if (e.key === "Enter") {
      e.preventDefault();
      commit();
    } else if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      clear();
    }
  }

  async function apply(patch: BulkTaskPatch) {
    if (busy || ids.length === 0) return;
    busy = true;
    try {
      const outcome = await bulkEditTasks(ids, patch);
      const notices = outcome.user_notices ?? [];
      userNotice = notices.length
        ? {
            text: notices.map(describeUserNotice).join(" "),
            warning: notices.some((n) => n.type === "kind_mismatch"),
          }
        : null;
      announce(withUserNotices(summarizeBulk("Updated", outcome), notices));
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

  <div class="bulk-text-field">
    <UserRefInput
      id="bulk-assignee"
      className="bulk-text-input"
      placeholder="Assignee…"
      ariaLabel="Set assignee (type a new name, or claude:name)"
      disabled={busy}
      {knownUsers}
      {userKinds}
      error={assigneeError}
      bind:value={assigneeDraft}
      onInput={() => {
        assigneeError = null;
        confirmedMismatch = null;
        userNotice = null;
      }}
      onEnter={() => commitAssignee()}
      onEscape={() => {
        assigneeDraft = "";
        assigneeError = null;
      }}
    />
  </div>

  <div class="bulk-text-field">
    <input
      autocorrect="off"
      autocapitalize="off"
      type="text"
      class="bulk-text-input"
      list="bulk-project-options"
      placeholder="Project…"
      aria-label="Set project (type a new name)"
      autocomplete="off"
      spellcheck="false"
      disabled={busy}
      bind:value={projectDraft}
      oninput={() => (projectError = null)}
      onkeydown={(e) =>
        handleTextKeydown(e, commitProject, () => {
          projectDraft = "";
          projectError = null;
        })}
    />
    <datalist id="bulk-project-options">
      {#each knownProjects as p (p)}
        <option value={p}></option>
      {/each}
    </datalist>
    {#if projectError}
      <span class="field-error" role="alert">{projectError}</span>
    {/if}
  </div>

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

  {#if userNotice}
    <span
      class="bulk-notice"
      class:is-warning={userNotice.warning}
      role="status"
    >
      {userNotice.text}
    </span>
  {/if}

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
