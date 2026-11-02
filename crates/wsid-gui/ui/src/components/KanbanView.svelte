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
    activePersona: string;
    onSelectTask: (id: string) => void;
    onDataChanged: () => void;
    announce: (msg: string) => void;
  }

  let { tasks, activePersona, onSelectTask, onDataChanged, announce }: Props =
    $props();

  let filterMyTasksOnly = $state<boolean>(false);

  let filteredTasks = $derived(
    filterMyTasksOnly
      ? tasks.filter(
          (t) => t.assignee.toLowerCase() === activePersona.toLowerCase(),
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
</script>

<section aria-labelledby="kanban-view-heading">
  <div
    style="display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 1rem; margin-bottom: 1rem;"
  >
    <div>
      <h2 id="kanban-view-heading" style="margin: 0; font-size: 1.35rem;">
        Kanban Board
      </h2>
      <p style="margin: 0.2rem 0 0 0; color: var(--text-secondary); font-size: 0.88rem;">
        Every card includes a keyboard-accessible "Move to column" selector so
        moving tasks never requires pointer drag-and-drop.
      </p>
    </div>

    <div class="control-inline">
      <input
        id="kanban-persona-filter"
        type="checkbox"
        bind:checked={filterMyTasksOnly}
      />
      <label for="kanban-persona-filter">
        Only show tasks assigned to @{activePersona}
      </label>
    </div>
  </div>

  <div class="kanban-board">
    {#each STATUS_ORDER as colStatus (colStatus)}
      {@const colTasks = tasksForColumn(colStatus)}
      <section
        class="kanban-column"
        aria-labelledby={`kanban-col-${colStatus}`}
      >
        <h3 id={`kanban-col-${colStatus}`} class="kanban-col-header">
          <span>{STATUS_LABELS[colStatus]}</span>
          <span class="badge">{colTasks.length}</span>
        </h3>

        {#if colTasks.length === 0}
          <p class="field-hint" style="margin: 0.25rem 0;">
            No tasks in {STATUS_LABELS[colStatus]}.
          </p>
        {:else}
          <ul
            style="list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0.65rem;"
          >
            {#each colTasks as task (task.id)}
              <li>
                <article
                  class="task-card"
                  style="padding: 0.75rem;"
                  aria-labelledby={`kanban-card-${task.id}`}
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
