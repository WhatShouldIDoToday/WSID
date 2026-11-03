<script lang="ts">
  import {
    type EisenhowerClassification,
    type EisenhowerQuadrant,
    QUADRANT_META,
    STATUS_LABELS,
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
    items: EisenhowerClassification[];
    activeUser: string;
    filterByUser: boolean;
    knownTags: string[];
    knownUsers: string[];
    knownProjects: string[];
    onToggleUserFilter: (enabled: boolean) => void;
    onSelectTask: (id: string) => void;
    onDataChanged: () => void;
    announce: (msg: string) => void;
  }

  let {
    items,
    activeUser,
    filterByUser,
    knownTags,
    knownUsers,
    knownProjects,
    onToggleUserFilter,
    onSelectTask,
    onDataChanged,
    announce,
  }: Props = $props();

  const PROJECT_ALL = "all";
  /** Project shown on the matrix; Eisenhower starts on `default`, not `all`. */
  let projectFilter = $state<string>("default");
  let projectOptions = $derived(
    [
      ...new Set([
        "default",
        ...knownProjects,
        ...items.map((i) => i.task.project),
      ]),
    ]
      .filter(Boolean)
      .sort((a, b) =>
        a === "default" ? -1 : b === "default" ? 1 : a.localeCompare(b),
      ),
  );

  const QUADRANTS: EisenhowerQuadrant[] = [
    "q1_do_first",
    "q2_schedule",
    "q3_delegate_quick",
    "q4_later",
  ];

  const selection = new Selection();
  /** Ids being dragged (the whole selection when the grabbed card is selected). */
  let dragIds = $state<string[]>([]);
  let dropTargetQuadrant = $state<EisenhowerQuadrant | null>(null);
  let deadlineWarning = $state<string | null>(null);
  let pointerDrag = $state<PointerDragState<EisenhowerClassification> | null>(
    null,
  );

  $effect(() => {
    selection.prune(items.map((i) => i.task.id));
  });

  function dragPayloadFor(entry: EisenhowerClassification): string[] {
    return selection.has(entry.task.id) ? [...selection.ids] : [entry.task.id];
  }

  function itemsForQuadrant(q: EisenhowerQuadrant): EisenhowerClassification[] {
    return items.filter(
      (item) =>
        item.quadrant === q &&
        (projectFilter === PROJECT_ALL || item.task.project === projectFilter),
    );
  }

  function isInteractiveTarget(target: EventTarget | null): boolean {
    if (!(target instanceof Element)) return false;
    return Boolean(target.closest("button, select, input, textarea, a, label"));
  }

  async function moveIdsToQuadrant(
    ids: string[],
    targetQuadrant: EisenhowerQuadrant,
  ) {
    const entries = ids
      .map((id) => items.find((i) => i.task.id === id))
      .filter((e): e is EisenhowerClassification => !!e)
      .filter((e) => e.quadrant !== targetQuadrant);
    if (entries.length === 0) return;

    const targetImportant =
      targetQuadrant === "q1_do_first" || targetQuadrant === "q2_schedule";
    const targetUrgent =
      targetQuadrant === "q1_do_first" ||
      targetQuadrant === "q3_delegate_quick";
    const title = QUADRANT_META[targetQuadrant].title;

    // Deadline-driven urgency keeps tasks urgent regardless of the override.
    const forced = entries.filter(
      (e) => !targetUrgent && e.slack_days !== null && e.slack_days <= 2.0,
    );

    try {
      const outcome = await bulkEditTasks(
        entries.map((e) => e.task.id),
        {
          ...(targetImportant
            ? { importance_at_least: 3 }
            : { importance_at_most: 2 }),
          urgent_override: targetUrgent,
        },
      );
      const failed = outcome.failures.length;
      const ok = outcome.updated.length;
      let msg: string;
      if (entries.length === 1 && failed === 0) {
        const e = entries[0];
        const t = outcome.updated[0];
        msg = forced.length
          ? `Task ${e.task.id} (${e.task.title}) updated (Importance ${t.importance}/5, Urgent Override cleared), but it remains in ${targetImportant ? QUADRANT_META.q1_do_first.title : QUADRANT_META.q3_delegate_quick.title} because its deadline (${e.effective_deadline ?? "upcoming"}, ${e.slack_days?.toFixed(1)}d slack) is within the 2.0-day urgent horizon.`
          : `Moved ${e.task.id} (${e.task.title}) to ${title} (Importance ${t.importance}/5, Urgent Override ${targetUrgent ? "on" : "off"})`;
      } else {
        msg = `Moved ${ok} ${ok === 1 ? "task" : "tasks"} to ${title}`;
        if (forced.length) {
          msg += `; ${forced.length} remain urgent because their deadlines are within 2.0 days (${forced.map((e) => e.task.id).join(", ")})`;
        }
        if (failed) {
          msg += `; ${failed} failed (${outcome.failures
            .slice(0, 3)
            .map((f) => `${f.id}: ${f.error}`)
            .join("; ")})`;
        }
      }
      deadlineWarning = forced.length ? msg : null;
      announce(msg);
      if (failed === 0) selection.clear();
      onDataChanged();
    } catch (err) {
      announce(`Cannot move tasks: ${String(err)}`);
    }
  }

  function cancelPointerDrag() {
    dragIds = [];
    dropTargetQuadrant = null;
    pointerDrag = null;
  }

  function handlePointerDown(e: PointerEvent, entry: EisenhowerClassification) {
    if (e.button !== 0 || isInteractiveTarget(e.target)) return;
    pointerDrag = beginPointerDrag(entry, e, e.currentTarget as Element | null);
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
      const quadEl = el?.closest("[data-drop-quadrant]") as HTMLElement | null;
      const quadAttr = quadEl?.dataset.dropQuadrant as
        EisenhowerQuadrant | undefined;
      dropTargetQuadrant = quadAttr ?? null;
    }
  }

  async function handleWindowPointerUp() {
    if (!pointerDrag) return;
    const activeDrag = pointerDrag;
    const targetQuad = dropTargetQuadrant;
    const ids = dragIds;
    pointerDrag = null;
    dragIds = [];
    dropTargetQuadrant = null;
    if (activeDrag.active && targetQuad) {
      await moveIdsToQuadrant(ids, targetQuad);
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
    taskId={pointerDrag.item.task.id}
    title={pointerDrag.item.task.title}
    subtitle={`@${pointerDrag.item.task.assignee} · ${QUADRANT_META[pointerDrag.item.quadrant].badge}`}
  />
{/if}

<section aria-labelledby="eisenhower-view-heading">
  <div
    style="display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 1rem; margin-bottom: 1rem;"
  >
    <h2 id="eisenhower-view-heading" style="margin: 0; font-size: 1.35rem;">
      Eisenhower Matrix
    </h2>

    <div class="control-inline">
      <label for="matrix-project-select" class="field-label" style="margin: 0;">
        Project
      </label>
      <select
        id="matrix-project-select"
        bind:value={projectFilter}
        onchange={() => announce(`Matrix project filter: ${projectFilter}`)}
      >
        {#each projectOptions as project (project)}
          <option value={project}>{project}</option>
        {/each}
        <option value={PROJECT_ALL}>All projects</option>
      </select>
    </div>

    <div class="control-inline">
      <input
        id="matrix-user-toggle"
        type="checkbox"
        checked={filterByUser}
        onchange={(e) =>
          onToggleUserFilter((e.currentTarget as HTMLInputElement).checked)}
      />
      <label for="matrix-user-toggle">
        Only show tasks assigned to @{activeUser}
      </label>
    </div>
  </div>

  {#if deadlineWarning}
    <div
      class="card"
      role="alert"
      style="margin-bottom: 1rem; border-color: var(--warning); background: rgba(251, 191, 36, 0.12); display: flex; justify-content: space-between; align-items: center; gap: 1rem;"
    >
      <span style="color: var(--warning-text); font-weight: 600;">
        {deadlineWarning}
      </span>
      <button
        type="button"
        class="btn-sm"
        onclick={() => (deadlineWarning = null)}
      >
        Dismiss
      </button>
    </div>
  {/if}

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
    >
      <select
        aria-label="Move to quadrant"
        onchange={(e) => {
          const el = e.currentTarget as HTMLSelectElement;
          const q = el.value as EisenhowerQuadrant;
          el.value = "";
          if (q) void moveIdsToQuadrant(selection.ids, q);
        }}
      >
        <option value="">Quadrant…</option>
        {#each QUADRANTS as q}
          <option value={q}>{QUADRANT_META[q].title}</option>
        {/each}
      </select>
    </BulkActions>
  {/if}

  <div class="eisenhower-grid">
    {#each QUADRANTS as q (q)}
      {@const qItems = itemsForQuadrant(q)}
      {@const meta = QUADRANT_META[q]}
      <section
        class="quadrant-panel"
        class:drop-target-active={dropTargetQuadrant === q}
        data-drop-quadrant={q}
        aria-labelledby={`quadrant-heading-${q}`}
      >
        <!-- HTML5 dragover/drop handlers removed: Tauri's file-drop interceptor prevents them from firing. -->
        <header
          style="border-bottom: 1px solid var(--border-subtle); padding-bottom: 0.5rem;"
        >
          <div
            style="display: flex; justify-content: space-between; align-items: center;"
          >
            <h3
              id={`quadrant-heading-${q}`}
              style="margin: 0; font-size: 1.05rem;"
            >
              {meta.title}
            </h3>
            <span class="badge">{qItems.length}</span>
          </div>
          <p
            style="margin: 0.2rem 0 0 0; font-size: 0.82rem; color: var(--text-secondary);"
          >
            {meta.subtitle}
          </p>
        </header>

        {#if qItems.length === 0}
          <p class="field-hint" style="margin: 0;">
            No tasks in this quadrant.
          </p>
        {:else}
          <ul
            style="list-style: none; padding: 0; margin: 0; display: flex; flex-direction: column; gap: 0.6rem;"
          >
            {#each qItems as entry (entry.task.id)}
              <li>
                <article
                  class="task-card draggable-card"
                  class:is-dragging={dragIds.includes(entry.task.id)}
                  class:is-selected={selection.has(entry.task.id)}
                  style="padding: 0.75rem;"
                  onpointerdown={(e) => handlePointerDown(e, entry)}
                >
                  <!-- HTML5 dragstart/dragend removed: Tauri's file-drop interceptor prevents them from firing. -->
                  <div class="task-card-header">
                    <div>
                      <div class="meta-row" style="margin-bottom: 0.25rem;">
                        <input
                          type="checkbox"
                          class="select-check"
                          aria-label={`Select ${entry.task.id}`}
                          checked={selection.has(entry.task.id)}
                          onclick={(e) =>
                            selection.click(
                              e,
                              entry.task.id,
                              qItems.map((i) => i.task.id),
                            )}
                        />
                        <span class="task-id-badge">{entry.task.id}</span>
                        <span class="badge">
                          {STATUS_LABELS[entry.task.status]}
                        </span>
                        <span>@{entry.task.assignee}</span>
                        {#if entry.task.urgent_override}
                          <span class="badge badge-urgent">Override</span>
                        {/if}
                      </div>
                      <h4 style="margin: 0; font-size: 0.98rem;">
                        {entry.task.title}
                      </h4>
                    </div>

                    <div
                      style="display: flex; flex-direction: column; gap: 0.35rem; align-items: stretch;"
                    >
                      <button
                        type="button"
                        class="btn-sm"
                        onclick={() => onSelectTask(entry.task.id)}
                      >
                        Inspect
                      </button>
                      <select
                        aria-label={`Move ${entry.task.id} to quadrant`}
                        onchange={(e) => {
                          const el = e.currentTarget as HTMLSelectElement;
                          const q = el.value as EisenhowerQuadrant;
                          el.value = "";
                          if (q) void moveIdsToQuadrant([entry.task.id], q);
                        }}
                      >
                        <option value="">Move…</option>
                        {#each QUADRANTS.filter((x) => x !== entry.quadrant) as q}
                          <option value={q}>{QUADRANT_META[q].title}</option>
                        {/each}
                      </select>
                    </div>
                  </div>

                  <div class="meta-row" style="font-size: 0.8rem;">
                    <span>Imp: <strong>{entry.task.importance}/5</strong></span>
                    <span
                      >· Diff: <strong>{entry.task.difficulty}/5</strong></span
                    >
                    <span>
                      · Est:
                      <strong>
                        {formatDurationMinutes(
                          entry.task.time_estimate_minutes,
                        )}
                      </strong>
                    </span>
                    {#if entry.effective_deadline}
                      <span>
                        · Deadline: <strong>{entry.effective_deadline}</strong>
                        {#if entry.slack_days !== null}
                          ({entry.slack_days.toFixed(1)}d slack)
                        {/if}
                      </span>
                    {:else}
                      <span>· Undated</span>
                    {/if}
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
