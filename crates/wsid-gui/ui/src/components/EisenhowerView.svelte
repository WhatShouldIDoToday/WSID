<script lang="ts">
  import {
    type EisenhowerClassification,
    type EisenhowerQuadrant,
    QUADRANT_META,
    STATUS_LABELS,
    formatDurationMinutes,
  } from "../lib/ipc";

  interface Props {
    items: EisenhowerClassification[];
    activeUser: string;
    filterByUser: boolean;
    onToggleUserFilter: (enabled: boolean) => void;
    onSelectTask: (id: string) => void;
  }

  let {
    items,
    activeUser,
    filterByUser,
    onToggleUserFilter,
    onSelectTask,
  }: Props = $props();

  const QUADRANTS: EisenhowerQuadrant[] = [
    "q1_do_first",
    "q2_schedule",
    "q3_delegate_quick",
    "q4_later",
  ];

  function itemsForQuadrant(q: EisenhowerQuadrant): EisenhowerClassification[] {
    return items.filter((item) => item.quadrant === q);
  }
</script>

<section aria-labelledby="eisenhower-view-heading">
  <div
    style="display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 1rem; margin-bottom: 1rem;"
  >
    <h2 id="eisenhower-view-heading" style="margin: 0; font-size: 1.35rem;">
      Eisenhower Matrix
    </h2>

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

  <div class="eisenhower-grid">
    {#each QUADRANTS as q (q)}
      {@const qItems = itemsForQuadrant(q)}
      {@const meta = QUADRANT_META[q]}
      <section class="quadrant-panel" aria-labelledby={`quadrant-heading-${q}`}>
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
                <article class="task-card" style="padding: 0.75rem;">
                  <div class="task-card-header">
                    <div>
                      <div class="meta-row" style="margin-bottom: 0.25rem;">
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

                    <button
                      type="button"
                      class="btn-sm"
                      onclick={() => onSelectTask(entry.task.id)}
                    >
                      Inspect
                    </button>
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
