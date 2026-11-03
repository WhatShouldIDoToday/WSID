<script lang="ts">
  import {
    type Task,
    type TaskStatus,
    STATUS_LABELS,
    STATUS_ORDER,
    bulkEditTasks,
    formatDurationMinutes,
  } from "../lib/ipc";
  import { Selection } from "../lib/selection.svelte";
  import {
    advancePointerDrag,
    beginPointerDrag,
    type PointerDragState,
  } from "../lib/pointerdrag";
  import BulkActions from "./BulkActions.svelte";
  import DragGhost from "./DragGhost.svelte";

  interface Props {
    tasks: Task[];
    activeUser: string;
    knownTags: string[];
    knownUsers: string[];
    knownProjects: string[];
    onSelectTask: (id: string) => void;
    onDataChanged: () => void;
    announce: (msg: string) => void;
  }

  let {
    tasks,
    activeUser,
    knownTags,
    knownUsers,
    knownProjects,
    onSelectTask,
    onDataChanged,
    announce,
  }: Props = $props();

  const PROJECT_ALL = "all";

  /** Project shown on the board; Kanban starts on `default`, not `all`. */
  let projectFilter = $state<string>("default");

  let projectOptions = $derived(
    [...new Set(["default", ...knownProjects, ...tasks.map((t) => t.project)])]
      .filter(Boolean)
      .sort((a, b) =>
        a === "default" ? -1 : b === "default" ? 1 : a.localeCompare(b),
      ),
  );

  const selection = new Selection();
  /** Ids being dragged (the whole selection when the grabbed card is selected). */
  let dragIds = $state<string[]>([]);

  $effect(() => {
    selection.prune(tasks.map((t) => t.id));
  });

  function dragPayloadFor(task: Task): string[] {
    return selection.has(task.id) ? [...selection.ids] : [task.id];
  }

  let filterMyTasksOnly = $state<boolean>(false);
  let dropTargetStatus = $state<TaskStatus | null>(null);
  let pointerDrag = $state<PointerDragState<Task> | null>(null);

  let filteredTasks = $derived(
    tasks
      .filter(
        (t) => projectFilter === PROJECT_ALL || t.project === projectFilter,
      )
      .filter((t) =>
        filterMyTasksOnly
          ? t.assignee.toLowerCase() === activeUser.toLowerCase()
          : true,
      ),
  );

  function tasksForColumn(status: TaskStatus): Task[] {
    return filteredTasks.filter((t) => t.status === status);
  }

  async function moveTasksToStatus(ids: string[], newStatus: TaskStatus) {
    const moving = ids.filter(
      (id) => tasks.find((t) => t.id === id)?.status !== newStatus,
    );
    if (moving.length === 0) return;
    try {
      const outcome = await bulkEditTasks(moving, { status: newStatus });
      const failed = outcome.failures.length;
      if (moving.length === 1 && failed === 0) {
        const task = outcome.updated[0];
        const spawned = outcome.spawned_review_tasks[0];
        announce(
          spawned
            ? `Task ${task.id} moved to In Review and spawned review subtask ${spawned.id} for @${spawned.assignee}`
            : `Moved ${task.id} (${task.title}) to ${STATUS_LABELS[task.status]} column`,
        );
      } else {
        const ok = outcome.updated.length;
        announce(
          `Moved ${ok} ${ok === 1 ? "task" : "tasks"} to ${STATUS_LABELS[newStatus]}` +
            (failed
              ? `; ${failed} failed (${outcome.failures
                  .slice(0, 3)
                  .map((f) => `${f.id}: ${f.error}`)
                  .join("; ")})`
              : ""),
        );
      }
      if (failed === 0) selection.clear();
      onDataChanged();
    } catch (err) {
      announce(`Cannot move tasks: ${String(err)}`);
    }
  }

  function isInteractiveTarget(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false;
    return Boolean(target.closest("button, select, input, textarea, a, label"));
  }

  function handleDragStart(e: DragEvent, task: Task) {
    if (pointerDrag || isInteractiveTarget(e.target)) {
      e.preventDefault();
      return;
    }
    dragIds = dragPayloadFor(task);
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", dragIds.join(","));
    }
  }

  function handleDragOver(e: DragEvent, colStatus: TaskStatus) {
    if (dragIds.length === 0) return;
    e.preventDefault();
    if (e.dataTransfer) {
      e.dataTransfer.dropEffect = "move";
    }
    if (dropTargetStatus !== colStatus) {
      dropTargetStatus = colStatus;
    }
  }

  function handleDragLeave(e: DragEvent, colStatus: TaskStatus) {
    const currentTarget = e.currentTarget as HTMLElement | null;
    const related = e.relatedTarget as Node | null;
    if (currentTarget && related && currentTarget.contains(related)) {
      return;
    }
    if (dropTargetStatus === colStatus) {
      dropTargetStatus = null;
    }
  }

  async function handleDrop(e: DragEvent, colStatus: TaskStatus) {
    e.preventDefault();
    const ids = dragIds.length
      ? dragIds
      : (e.dataTransfer?.getData("text/plain") ?? "")
          .split(",")
          .map((x) => x.trim())
          .filter(Boolean);
    dragIds = [];
    dropTargetStatus = null;
    pointerDrag = null;
    await moveTasksToStatus(ids, colStatus);
  }

  function handleDragEnd() {
    dragIds = [];
    dropTargetStatus = null;
    pointerDrag = null;
  }

  function handlePointerDown(e: PointerEvent, task: Task) {
    if (e.button !== 0 || isInteractiveTarget(e.target)) return;
    pointerDrag = beginPointerDrag(task, e, e.currentTarget as Element | null);
  }

  function handleWindowPointerMove(e: PointerEvent) {
    if (!pointerDrag) return;
    const wasActive = pointerDrag.active;
    const active = advancePointerDrag(pointerDrag, e);
    if (!wasActive && active) {
      dragIds = dragPayloadFor(pointerDrag.item);
    }
    if (active) {
      e.preventDefault();
      const el = document.elementFromPoint(e.clientX, e.clientY);
      const colEl = el?.closest("[data-drop-column]") as HTMLElement | null;
      const statusAttr = colEl?.dataset.dropColumn as TaskStatus | undefined;
      dropTargetStatus = statusAttr ?? null;
    }
  }

  async function handleWindowPointerUp() {
    if (!pointerDrag) return;
    const activeDrag = pointerDrag;
    const targetCol = dropTargetStatus;
    const ids = dragIds;
    pointerDrag = null;
    dragIds = [];
    dropTargetStatus = null;
    if (activeDrag.active && targetCol) {
      await moveTasksToStatus(ids, targetCol);
    }
  }
</script>

<svelte:window
  onpointermove={handleWindowPointerMove}
  onpointerup={handleWindowPointerUp}
  onpointercancel={handleDragEnd}
/>

{#if pointerDrag?.active}
  <DragGhost
    x={pointerDrag.x}
    y={pointerDrag.y}
    offsetX={pointerDrag.offsetX}
    offsetY={pointerDrag.offsetY}
    width={pointerDrag.width}
    taskId={pointerDrag.item.id}
    title={pointerDrag.item.title}
    subtitle={`@${pointerDrag.item.assignee} · ${STATUS_LABELS[pointerDrag.item.status]}`}
  />
{/if}

<section aria-labelledby="kanban-view-heading" class="kanban-view">
  <div
    style="display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 1rem; margin-bottom: 0.85rem;"
  >
    <h2 id="kanban-view-heading" style="margin: 0; font-size: 1.35rem;">
      Kanban Board
    </h2>

    <div class="control-inline">
      <label for="kanban-project-select" class="field-label" style="margin: 0;">
        Project
      </label>
      <select
        id="kanban-project-select"
        bind:value={projectFilter}
        onchange={() => announce(`Kanban project filter: ${projectFilter}`)}
      >
        {#each projectOptions as project (project)}
          <option value={project}>{project}</option>
        {/each}
        <option value={PROJECT_ALL}>All projects</option>
      </select>
    </div>

    <div class="control-inline">
      <input
        id="kanban-user-filter"
        type="checkbox"
        bind:checked={filterMyTasksOnly}
      />
      <label for="kanban-user-filter">
        Only show tasks assigned to @{activeUser}
      </label>
    </div>
  </div>

  {#if selection.count > 0}
    <BulkActions
      ids={selection.ids}
      {knownTags}
      {knownUsers}
      {knownProjects}
      {announce}
      onClear={() => selection.clear()}
      onDone={(outcome) => {
        if (outcome.failures.length === 0) selection.clear();
        onDataChanged();
      }}
    />
  {/if}

  <div class="kanban-board">
    {#each STATUS_ORDER as colStatus (colStatus)}
      {@const colTasks = tasksForColumn(colStatus)}
      <section
        class="kanban-column"
        class:drop-target-active={dropTargetStatus === colStatus}
        data-drop-column={colStatus}
        aria-labelledby={`kanban-col-${colStatus}`}
        ondragover={(e) => handleDragOver(e, colStatus)}
        ondragleave={(e) => handleDragLeave(e, colStatus)}
        ondrop={(e) => handleDrop(e, colStatus)}
      >
        <h3 id={`kanban-col-${colStatus}`} class="kanban-col-header">
          <span>{STATUS_LABELS[colStatus]}</span>
          <span class="badge">{colTasks.length}</span>
        </h3>

        {#if colTasks.length === 0}
          <p class="empty-state-message" style="padding: 1.5rem 0.5rem;">
            No tasks
          </p>
        {:else}
          <ul class="kanban-column-list">
            {#each colTasks as task (task.id)}
              <li>
                <article
                  class="task-card draggable-card"
                  class:is-dragging={dragIds.includes(task.id)}
                  class:is-selected={selection.has(task.id)}
                  style="padding: 0.75rem;"
                  aria-labelledby={`kanban-card-${task.id}`}
                  draggable="true"
                  ondragstart={(e) => handleDragStart(e, task)}
                  ondragend={handleDragEnd}
                  onpointerdown={(e) => handlePointerDown(e, task)}
                >
                  <div class="meta-row" style="justify-content: space-between;">
                    <span
                      style="display: inline-flex; align-items: center; gap: 0.5rem;"
                    >
                      <input
                        type="checkbox"
                        class="select-check"
                        aria-label={`Select ${task.id}`}
                        checked={selection.has(task.id)}
                        onclick={(e) =>
                          selection.click(
                            e,
                            task.id,
                            colTasks.map((t) => t.id),
                          )}
                      />
                      <span class="task-id-badge">{task.id}</span>
                      {#if projectFilter === PROJECT_ALL}
                        <span class="project-badge">{task.project}</span>
                      {/if}
                    </span>
                    <span style="font-size: 0.78rem;">@{task.assignee}</span>
                  </div>

                  <h4
                    id={`kanban-card-${task.id}`}
                    style="margin: 0; font-size: 0.96rem;"
                  >
                    {task.title}
                  </h4>

                  <div class="meta-row" style="font-size: 0.78rem;">
                    <span>Imp {task.importance}/5</span>
                    <span>· Diff {task.difficulty}/5</span>
                    {#if task.time_estimate_minutes !== null}
                      <span>
                        · {formatDurationMinutes(task.time_estimate_minutes)}
                      </span>
                    {/if}
                    {#if task.urgent_override}
                      <span class="badge badge-urgent">Urgent</span>
                    {/if}
                    {#if task.attachments.length > 0}
                      <span class="badge"
                        >{task.attachments.length} file(s)</span
                      >
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
                      Move {task.id} to column:
                    </label>
                    <select
                      id={`move-select-${task.id}`}
                      value={task.status}
                      onchange={(e) =>
                        moveTasksToStatus(
                          [task.id],
                          (e.currentTarget as HTMLSelectElement)
                            .value as TaskStatus,
                        )}
                    >
                      {#each STATUS_ORDER as targetStatus}
                        <option value={targetStatus}>
                          {STATUS_LABELS[targetStatus]}
                        </option>
                      {/each}
                    </select>

                    <button
                      type="button"
                      class="btn-sm"
                      onclick={() => onSelectTask(task.id)}
                    >
                      Open Details ({task.id})
                    </button>
                  </div>
                </article>
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/each}
  </div>
</section>
