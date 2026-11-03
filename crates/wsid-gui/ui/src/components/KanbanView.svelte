<script lang="ts">
  import Select from "./ui/Select.svelte";
  import { untrack } from "svelte";
  import {
    type Task,
    type TaskStatus,
    STATUS_LABELS,
    STATUS_ORDER,
    bulkEditTasks,
  } from "../lib/ipc";
  import { Selection } from "../lib/selection.svelte";
  import {
    advancePointerDrag,
    beginPointerDrag,
    type PointerDragState,
  } from "../lib/pointerdrag";
  import BoardTaskCard from "./BoardTaskCard.svelte";
  import BulkActions from "./BulkActions.svelte";
  import DragGhost from "./DragGhost.svelte";

  interface Props {
    tasks: Task[];
    activeUser: string;
    activeProject: string;
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
    activeProject,
    knownTags,
    knownUsers,
    knownProjects,
    onSelectTask,
    onDataChanged,
    announce,
  }: Props = $props();

  const PROJECT_ALL = "all";

  /** Project shown on the board; starts on the active project, not `all`. */
  let projectFilter = $state<string>(untrack(() => activeProject));

  let followedProject = untrack(() => activeProject);
  $effect(() => {
    const nextProject = activeProject;
    if (nextProject !== followedProject) {
      if (projectFilter === followedProject) projectFilter = nextProject;
      followedProject = nextProject;
    }
  });

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

  const STATUS_OPTIONS = STATUS_ORDER.map((status) => ({
    value: status,
    label: STATUS_LABELS[status],
  }));

  $effect(() => {
    selection.prune(tasks.map((t) => t.id));
  });

  function dragPayloadFor(task: Task): string[] {
    return selection.has(task.id) ? [...selection.ids] : [task.id];
  }

  const USER_ALL = "all";
  /** Assignee shown on the board; starts on all users. */
  let userFilter = $state<string>(USER_ALL);
  let followedUser = untrack(() => activeUser);

  // A user filter that still names the previous active user follows a switch; other choices are kept.
  $effect(() => {
    const nextUser = activeUser;
    if (nextUser !== followedUser) {
      if (userFilter === followedUser) userFilter = nextUser;
      followedUser = nextUser;
    }
  });

  let userOptions = $derived(
    [...new Set([activeUser, ...knownUsers])].filter(Boolean).sort(),
  );

  let dropTargetStatus = $state<TaskStatus | null>(null);
  let pointerDrag = $state<PointerDragState<Task> | null>(null);

  let filteredTasks = $derived(
    tasks
      .filter(
        (t) => projectFilter === PROJECT_ALL || t.project === projectFilter,
      )
      .filter(
        (t) =>
          userFilter === USER_ALL ||
          t.assignee.toLowerCase() === userFilter.toLowerCase(),
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

  function cancelPointerDrag() {
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
  onpointercancel={cancelPointerDrag}
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
      <Select
        id="kanban-project-select"
        size="sm"
        value={projectFilter}
        options={[
          ...projectOptions.map((project) => ({
            value: project,
            label: project,
          })),
          { value: PROJECT_ALL, label: "All projects" },
        ]}
        onValueChange={(v) => {
          projectFilter = v;
          announce(`Kanban project filter: ${projectFilter}`);
        }}
      />
    </div>

    <div class="control-inline">
      <label for="kanban-user-select" class="field-label" style="margin: 0;">
        User
      </label>
      <Select
        id="kanban-user-select"
        size="sm"
        value={userFilter}
        options={[
          { value: USER_ALL, label: "All users" },
          ...userOptions.map((user) => ({ value: user, label: `@${user}` })),
        ]}
        onValueChange={(v) => {
          userFilter = v;
          announce(
            "Kanban user filter: " +
              (userFilter === USER_ALL ? "all users" : "@" + userFilter),
          );
        }}
      />
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
      >
        <!-- HTML5 dragover/drop handlers removed: Tauri's file-drop interceptor prevents them from firing. -->
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
                <BoardTaskCard
                  {task}
                  titleId={`kanban-card-${task.id}`}
                  selected={selection.has(task.id)}
                  dragging={dragIds.includes(task.id)}
                  showProject={projectFilter === PROJECT_ALL}
                  urgentLabel={task.urgent_override ? "Urgent" : ""}
                  moveLabel={`Move ${task.id} to column:`}
                  moveOptions={STATUS_OPTIONS}
                  moveValue={task.status}
                  onMove={(status) =>
                    void moveTasksToStatus([task.id], status as TaskStatus)}
                  onOpen={() => onSelectTask(task.id)}
                  onPointerDown={(e) => handlePointerDown(e, task)}
                  onSelectClick={(e) =>
                    selection.click(
                      e,
                      task.id,
                      colTasks.map((t) => t.id),
                    )}
                />
              </li>
            {/each}
          </ul>
        {/if}
      </section>
    {/each}
  </div>
</section>
