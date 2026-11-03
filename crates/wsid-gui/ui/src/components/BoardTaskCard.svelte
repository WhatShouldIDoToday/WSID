<script lang="ts">
  import { type Task, STATUS_LABELS, formatDurationMinutes } from "../lib/ipc";

  interface MoveOption {
    value: string;
    label: string;
  }

  interface Props {
    task: Task;
    titleId?: string;
    selected: boolean;
    dragging: boolean;
    /** Show the project badge (when a board is filtered to "All projects"). */
    showProject?: boolean;
    /** Show the task status badge (Eisenhower has no status columns). */
    showStatus?: boolean;
    urgentLabel?: string;
    /** Effective deadline for the card; omit to hide the deadline line. */
    deadline?: string | null;
    slackDays?: number | null;
    /** Always render an estimate slot, using "—" when absent. */
    estimatePlaceholder?: boolean;
    moveLabel: string;
    moveOptions: MoveOption[];
    /** Currently selected move target; leave unset for an action-style menu. */
    moveValue?: string;
    movePlaceholder?: string;
    onMove: (value: string) => void;
    onOpen: () => void;
    onPointerDown?: (e: PointerEvent) => void;
    onSelectClick: (e: MouseEvent) => void;
  }

  let {
    task,
    titleId = undefined,
    selected,
    dragging,
    showProject = false,
    showStatus = false,
    urgentLabel = "",
    deadline = undefined,
    slackDays = null,
    estimatePlaceholder = false,
    moveLabel,
    moveOptions,
    moveValue = undefined,
    movePlaceholder = undefined,
    onMove,
    onOpen,
    onPointerDown,
    onSelectClick,
  }: Props = $props();

  function handleMove(e: Event) {
    const el = e.currentTarget as HTMLSelectElement;
    const next = el.value;
    if (!next) return;
    onMove(next);
    // Action-style menus reset to their placeholder; status selects snap back to
    // the current value until the refreshed task data arrives.
    el.value = moveValue ?? "";
  }
</script>

<article
  class="task-card draggable-card"
  class:is-dragging={dragging}
  class:is-selected={selected}
  style="padding: 0.75rem;"
  aria-labelledby={titleId}
  onpointerdown={(e) => onPointerDown?.(e)}
>
  <!-- HTML5 dragstart/dragend removed: Tauri's file-drop interceptor prevents them from firing. -->
  <div class="meta-row" style="justify-content: space-between;">
    <span style="display: inline-flex; align-items: center; gap: 0.5rem;">
      <input
        type="checkbox"
        class="select-check"
        aria-label={`Select ${task.id}`}
        checked={selected}
        onclick={onSelectClick}
      />
      <span class="task-id-badge">{task.id}</span>
      {#if showStatus}
        <span class="badge">{STATUS_LABELS[task.status]}</span>
      {/if}
      {#if showProject}
        <span class="project-badge">{task.project}</span>
      {/if}
    </span>
    <span style="font-size: 0.78rem;">@{task.assignee}</span>
  </div>

  <h4 id={titleId} style="margin: 0; font-size: 0.96rem;">
    {task.title}
  </h4>

  <div class="meta-row" style="font-size: 0.78rem;">
    <span>Imp {task.importance}/5</span>
    <span>· Diff {task.difficulty}/5</span>
    {#if task.time_estimate_minutes !== null}
      <span>· {formatDurationMinutes(task.time_estimate_minutes)}</span>
    {:else if estimatePlaceholder}
      <span>· Est —</span>
    {/if}
    {#if urgentLabel}
      <span class="badge badge-urgent">{urgentLabel}</span>
    {/if}
    {#if deadline !== undefined}
      {#if deadline}
        <span>
          · Deadline <strong>{deadline}</strong>
          {#if slackDays !== null}
            ({slackDays.toFixed(1)}d slack)
          {/if}
        </span>
      {:else}
        <span>· Undated</span>
      {/if}
    {/if}
    {#if task.attachments.length > 0}
      <span class="badge">{task.attachments.length} file(s)</span>
    {/if}
  </div>

  {#if task.tags.length > 0}
    <div class="meta-row">
      {#each task.tags as tag}
        <span class="badge badge-tag">#{tag}</span>
      {/each}
    </div>
  {/if}

  <div
    style="display: flex; flex-direction: column; gap: 0.4rem; margin-top: 0.25rem;"
  >
    <label
      for={`move-select-${task.id}`}
      style="font-size: 0.76rem; color: var(--text-secondary);"
    >
      {moveLabel}
    </label>
    <select
      id={`move-select-${task.id}`}
      value={moveValue ?? ""}
      onchange={handleMove}
    >
      {#if movePlaceholder !== undefined}
        <option value="">{movePlaceholder}</option>
      {/if}
      {#each moveOptions as opt (opt.value)}
        <option value={opt.value}>{opt.label}</option>
      {/each}
    </select>

    <button type="button" class="btn-sm" onclick={onOpen}>
      Open Details ({task.id})
    </button>
  </div>
</article>
