<script lang="ts">
  import {
    type Task,
    type TaskStatus,
    STATUS_LABELS,
    STATUS_ORDER,
    transitionStatus,
    formatDurationMinutes,
  } from "../lib/ipc";

  interface Props {
    tasks: Task[];
    activeUser: string;
    onSelectTask: (id: string) => void;
    onDataChanged: () => void;
    announce: (msg: string) => void;
  }

  let { tasks, activeUser, onSelectTask, onDataChanged, announce }: Props =
    $props();

  let filterMyTasksOnly = $state<boolean>(false);
  let draggingTaskId = $state<string | null>(null);
  let dropTargetStatus = $state<TaskStatus | null>(null);
  let pointerDrag = $state<{
    task: Task;
    startX: number;
    startY: number;
    active: boolean;
  } | null>(null);

  let filteredTasks = $derived(
    filterMyTasksOnly
      ? tasks.filter(
          (t) => t.assignee.toLowerCase() === activeUser.toLowerCase(),
        )
      : tasks,
  );

  function tasksForColumn(status: TaskStatus): Task[] {
    return filteredTasks.filter((t) => t.status === status);
  }

  async function moveTaskToStatus(task: Task, newStatus: TaskStatus) {
    if (task.status === newStatus) return;
    try {
      const outcome = await transitionStatus(task.id, newStatus);
      if (outcome.spawned_review_task) {
        announce(
          `Task ${task.id} moved to In Review and spawned review subtask ${outcome.spawned_review_task.id} for @${outcome.spawned_review_task.assignee}`,
        );
      } else {
        announce(
          `Moved ${task.id} (${task.title}) to ${STATUS_LABELS[outcome.task.status]} column`,
        );
      }
      onDataChanged();
    } catch (err) {
      announce(`Cannot move task ${task.id}: ${String(err)}`);
    }
  }

  function isInteractiveTarget(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false;
    return Boolean(target.closest("button, select, input, textarea, a, label"));
  }

  function handleDragStart(e: DragEvent, task: Task) {
    if (isInteractiveTarget(e.target)) {
      e.preventDefault();
      return;
    }
    pointerDrag = null;
    draggingTaskId = task.id;
    if (e.dataTransfer) {
      e.dataTransfer.effectAllowed = "move";
      e.dataTransfer.setData("text/plain", task.id);
    }
  }

  function handleDragOver(e: DragEvent, colStatus: TaskStatus) {
    if (!draggingTaskId) return;
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
    const id =
      draggingTaskId || e.dataTransfer?.getData("text/plain")?.trim() || null;
    draggingTaskId = null;
    dropTargetStatus = null;
    pointerDrag = null;
    if (!id) return;
    const task = tasks.find((t) => t.id === id);
    if (task && task.status !== colStatus) {
      await moveTaskToStatus(task, colStatus);
    }
  }

  function handleDragEnd() {
    draggingTaskId = null;
    dropTargetStatus = null;
    pointerDrag = null;
  }

  function handlePointerDown(e: PointerEvent, task: Task) {
    if (e.button !== 0 || isInteractiveTarget(e.target)) return;
    pointerDrag = {
      task,
      startX: e.clientX,
      startY: e.clientY,
      active: false,
    };
  }

  function handleWindowPointerMove(e: PointerEvent) {
    if (!pointerDrag) return;
    const dx = e.clientX - pointerDrag.startX;
    const dy = e.clientY - pointerDrag.startY;
    if (!pointerDrag.active && Math.hypot(dx, dy) > 6) {
      pointerDrag.active = true;
      draggingTaskId = pointerDrag.task.id;
    }
    if (pointerDrag.active) {
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
    pointerDrag = null;
    draggingTaskId = null;
    dropTargetStatus = null;
    if (
      activeDrag.active &&
      targetCol &&
      activeDrag.task.status !== targetCol
    ) {
      await moveTaskToStatus(activeDrag.task, targetCol);
    }
  }
</script>

<svelte:window
  onpointermove={handleWindowPointerMove}
  onpointerup={handleWindowPointerUp}
  onpointercancel={handleDragEnd}
/>

<section aria-labelledby="kanban-view-heading" class="kanban-view">
  <div
    style="display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 1rem; margin-bottom: 0.85rem;"
  >
    <h2 id="kanban-view-heading" style="margin: 0; font-size: 1.35rem;">
      Kanban Board
    </h2>

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
                  class:is-dragging={draggingTaskId === task.id}
                  style="padding: 0.75rem;"
                  aria-labelledby={`kanban-card-${task.id}`}
                  draggable="true"
                  ondragstart={(e) => handleDragStart(e, task)}
                  ondragend={handleDragEnd}
                  onpointerdown={(e) => handlePointerDown(e, task)}
                >
                  <div class="meta-row" style="justify-content: space-between;">
                    <span class="task-id-badge">{task.id}</span>
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
                        moveTaskToStatus(
                          task,
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
