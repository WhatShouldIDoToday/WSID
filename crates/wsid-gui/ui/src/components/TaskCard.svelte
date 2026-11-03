<script lang="ts">
  import { DropdownMenu } from "bits-ui";
  import UserPill from "./UserPill.svelte";
  import {
    type Task,
    STATUS_LABELS,
    formatDeadlineDisplay,
    formatDurationMinutes,
    plainTextPreview,
    userKindOf,
    type CardMoveDirection,
    type CardMoveOption,
  } from "../lib/ipc";

  interface Props {
    task: Task;
    /** Id for the title button, referenced by the card's accessible name. */
    titleId: string;
    /** 1-based rank shown in the score pill (Focus). */
    rank?: number;
    score?: number;
    quadrantLabel?: string;
    /** Effective deadline (Eisenhower); falls back to the task's own due date. */
    deadline?: string | null;
    slackDays?: number | null;
    /** Number of tasks this one unblocks (Focus). */
    unblocksCount?: number;
    showStatus?: boolean;
    showProject?: boolean;
    userKinds: Record<string, string>;
    /** Bulk-select checkbox, shown on Kanban and Eisenhower only. */
    selectable?: boolean;
    selected?: boolean;
    dragging?: boolean;
    onOpen: () => void;
    onSelectClick?: (e: MouseEvent) => void;
    /** Round completion button (Focus). The label doubles as its accessible name. */
    onComplete?: () => void;
    completeLabel?: string;
    onPointerDown?: (e: PointerEvent) => void;
    /** Targets for the "Move to…" menu. Omit for cards that cannot be moved. */
    moveOptions?: CardMoveOption[];
    moveLabel?: string;
    onMove?: (value: string) => void;
    /** Alt+Arrow keyboard movement; the parent decides what each direction means. */
    onKeyMove?: (direction: CardMoveDirection) => void;
  }

  let {
    task,
    titleId,
    rank = undefined,
    score = undefined,
    quadrantLabel = "",
    deadline = undefined,
    slackDays = null,
    unblocksCount = 0,
    showStatus = false,
    showProject = false,
    userKinds,
    selectable = false,
    selected = false,
    dragging = false,
    onOpen,
    onSelectClick,
    onComplete,
    completeLabel = "Mark complete",
    onPointerDown,
    moveOptions = [],
    moveLabel = "Move to…",
    onMove,
    onKeyMove,
  }: Props = $props();

  const DIRECTION_BY_KEY: Record<string, CardMoveDirection> = {
    ArrowLeft: "left",
    ArrowRight: "right",
    ArrowUp: "up",
    ArrowDown: "down",
  };

  let preview = $derived(plainTextPreview(task.description, 240));
  let dueLabel = $derived(deadline ?? task.due_date);
  let hasTemporal = $derived(
    task.time_estimate_minutes !== null ||
      task.sgdb_date !== null ||
      dueLabel !== null,
  );

  function handleKeydown(e: KeyboardEvent) {
    if (!e.altKey || !onKeyMove) return;
    const direction = DIRECTION_BY_KEY[e.key];
    if (!direction) return;
    e.preventDefault();
    onKeyMove(direction);
  }

  function openMoveMenuFromKey(e: KeyboardEvent, card: HTMLElement) {
    if (e.key !== "m" || e.altKey || e.ctrlKey || e.metaKey) return;
    const trigger = card.querySelector<HTMLElement>("[data-card-menu-trigger]");
    if (!trigger) return;
    e.preventDefault();
    trigger.click();
  }
</script>

<article
  class="task-card"
  class:is-selected={selected}
  class:is-dragging={dragging}
  class:draggable-card={onPointerDown !== undefined}
  aria-labelledby={titleId}
  onpointerdown={(e) => {
    // Controls inside the card handle their own pointer input; only the card body starts a drag.
    if (!(e.target as HTMLElement).closest(".card-control")) onPointerDown?.(e);
  }}
>
  <div class="card-top">
    <div class="card-lead">
      {#if selectable}
        <span class="card-control">
          <input
            type="checkbox"
            class="select-check"
            aria-label={`Select ${task.id}`}
            checked={selected}
            onclick={(e) => {
              e.stopPropagation();
              onSelectClick?.(e);
            }}
          />
        </span>
      {/if}
      {#if onComplete}
        <span class="card-control">
          <button
            type="button"
            class="card-complete-btn"
            aria-label={completeLabel}
            title={completeLabel}
            onclick={(e) => {
              e.stopPropagation();
              onComplete();
            }}
          >
            <span aria-hidden="true">✓</span>
          </button>
        </span>
      {/if}
      {#if rank !== undefined && score !== undefined}
        <span class="badge badge-score">#{rank} · Score {score.toFixed(1)}</span
        >
      {/if}
      <span class="task-id-badge">{task.id}</span>
      {#if showStatus}
        <span class="badge">{STATUS_LABELS[task.status]}</span>
      {/if}
      {#if quadrantLabel}
        <span class="card-quadrant">{quadrantLabel}</span>
      {/if}
      {#if task.urgent_override}
        <span class="badge badge-urgent">Urgent</span>
      {/if}
      {#if showProject}
        <span class="badge">{task.project}</span>
      {/if}
    </div>

    <div class="card-personas">
      <UserPill
        user={task.assignee}
        kind={userKindOf(userKinds, task.assignee)}
        role="Assignee"
      />
      {#if task.evaluator !== task.assignee}
        <UserPill
          user={task.evaluator}
          kind={userKindOf(userKinds, task.evaluator)}
          role="Evaluator"
        />
      {/if}
      {#if moveOptions.length > 0 && onMove}
        <span class="card-control">
          <DropdownMenu.Root>
            <DropdownMenu.Trigger
              class="card-menu-btn"
              aria-label={`${moveLabel} for ${task.id}`}
              aria-keyshortcuts="m"
              data-card-menu-trigger
            >
              <span aria-hidden="true">⋯</span>
            </DropdownMenu.Trigger>
            <DropdownMenu.Portal>
              <DropdownMenu.Content class="ui-select-content" sideOffset={4}>
                {#each moveOptions as option (option.value)}
                  <DropdownMenu.Item
                    class="ui-select-item"
                    onSelect={() => onMove(option.value)}
                  >
                    {option.label}
                  </DropdownMenu.Item>
                {/each}
              </DropdownMenu.Content>
            </DropdownMenu.Portal>
          </DropdownMenu.Root>
        </span>
      {/if}
    </div>
  </div>

  <h3 class="card-title">
    <button
      type="button"
      class="card-title-btn"
      id={titleId}
      aria-keyshortcuts="Enter"
      onclick={onOpen}
      onkeydown={(e) => {
        handleKeydown(e);
        if (!e.defaultPrevented)
          openMoveMenuFromKey(
            e,
            e.currentTarget.closest(".task-card") as HTMLElement,
          );
      }}
    >
      {task.title}
    </button>
  </h3>

  {#if preview}
    <p class="card-preview">{preview}</p>
  {/if}

  {#if task.tags.length > 0}
    <div class="card-tags">
      {#each task.tags as tag (tag)}
        <span class="badge badge-tag">#{tag}</span>
      {/each}
    </div>
  {/if}

  {#if unblocksCount > 0}
    <p class="card-unblocks">Unblocks {unblocksCount}</p>
  {/if}

  {#if hasTemporal}
    <div class="card-temporal">
      {#if task.time_estimate_minutes !== null}
        <span class="meta-estimate">
          <span aria-hidden="true">⏱</span>
          {formatDurationMinutes(task.time_estimate_minutes)}
        </span>
      {/if}
      {#if task.sgdb_date}
        <span class="meta-sgdb">
          SGDB: {formatDeadlineDisplay(task.sgdb_date)}
        </span>
      {/if}
      {#if dueLabel}
        <span class="meta-due">
          Due: {formatDeadlineDisplay(dueLabel)}
          {#if slackDays !== null}
            ({slackDays.toFixed(1)}d slack)
          {/if}
        </span>
      {/if}
    </div>
  {/if}
</article>
