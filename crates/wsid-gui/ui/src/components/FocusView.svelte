<script lang="ts">
  import {
    type FocusQueueResponse,
    type BlockedTaskEntry,
    type FocusPreset,
    type TaskStatus,
    STATUS_LABELS,
    PRESET_LABELS,
    QUADRANT_META,
    transitionStatus,
    formatDurationMinutes,
  } from "../lib/ipc";

  interface Props {
    focusData: FocusQueueResponse | null;
    blockedTasks: BlockedTaskEntry[];
    onSelectTask: (id: string) => void;
    onPresetChange: (preset: FocusPreset) => void;
    onDataChanged: () => void;
    announce: (msg: string) => void;
  }

  let {
    focusData,
    blockedTasks,
    onSelectTask,
    onPresetChange,
    onDataChanged,
    announce,
  }: Props = $props();

  async function handleQuickStatus(id: string, status: TaskStatus) {
    try {
      const outcome = await transitionStatus(id, status);
      if (outcome.spawned_review_task) {
        announce(
          `Task ${id} moved to In Review and spawned review task ${outcome.spawned_review_task.id} for @${outcome.spawned_review_task.assignee}`,
        );
      } else {
        announce(`Task ${id} marked ${STATUS_LABELS[outcome.task.status]}`);
      }
      onDataChanged();
    } catch (err) {
      announce(`Status error: ${String(err)}`);
    }
  }
</script>

<section aria-labelledby="focus-view-heading">
  <div
    style="display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 1rem; margin-bottom: 1rem;"
  >
    <div>
      <h2 id="focus-view-heading" style="margin: 0; font-size: 1.35rem;">
        Focus Queue ("What Should I Do?")
      </h2>
      {#if focusData}
        <p style="margin: 0.2rem 0 0 0; color: var(--text-secondary); font-size: 0.9rem;">
          Showing top actionable unblocked tasks for <strong>@{focusData.persona}</strong>
          under preset <strong>{PRESET_LABELS[focusData.preset]}</strong>.
        </p>
      {/if}
    </div>

    {#if focusData}
      <div class="control-inline">
        <label for="focus-preset-select">Prioritization Preset:</label>
        <select
          id="focus-preset-select"
          value={focusData.preset}
          onchange={(e) =>
            onPresetChange(
              (e.currentTarget as HTMLSelectElement).value as FocusPreset,
            )}
        >
          {#each Object.entries(PRESET_LABELS) as [key, label]}
            <option value={key}>{label}</option>
          {/each}
        </select>
      </div>
    {/if}
  </div>

  {#if !focusData}
    <p>Loading focus queue...</p>
  {:else if focusData.actionable_tasks.length === 0}
    <div class="card" style="margin-bottom: 1.25rem;">
      <h3 style="margin: 0 0 0.4rem 0;">No Actionable Unblocked Tasks</h3>
      <p style="margin: 0; color: var(--text-secondary);">
        @{focusData.persona} currently has no unblocked To Do or In Progress tasks.
        {#if focusData.blocked_summary.length > 0}
          See the Blocked Tasks & Review Queue summary below for what you are waiting on.
        {/if}
      </p>
    </div>
  {:else}
    <ol
      style="list-style: none; padding: 0; margin: 0 0 1.5rem 0; display: flex; flex-direction: column; gap: 0.85rem;"
      aria-label="Prioritized Focus Tasks"
    >
      {#each focusData.actionable_tasks as item, idx (item.task.id)}
        <li>
          <article class="task-card" aria-labelledby={`focus-title-${item.task.id}`}>
            <div class="task-card-header">
              <div>
                <div class="meta-row" style="margin-bottom: 0.3rem;">
                  <span class="badge" style="background: var(--accent-primary); color: var(--accent-text-dark); font-weight: 800;">
                    #{idx + 1} · Score {item.breakdown.total_score.toFixed(1)}
                  </span>
                  <span class="task-id-badge">{item.task.id}</span>
                  <span class="badge">{STATUS_LABELS[item.task.status]}</span>
                  <span class="badge">{QUADRANT_META[item.quadrant].badge}</span>
                  {#if item.task.urgent_override}
                    <span class="badge badge-urgent">Urgent</span>
                  {/if}
                  {#if item.task.attachments.length > 0}
                    <span class="badge">
                      {item.task.attachments.length} attachment(s)
                    </span>
                  {/if}
                </div>
                <h3 id={`focus-title-${item.task.id}`} style="margin: 0; font-size: 1.1rem;">
                  {item.task.title}
                </h3>
              </div>

              <div class="actions-row">
                {#if item.task.status === "todo"}
                  <button
                    type="button"
                    class="btn-sm"
                    onclick={() => handleQuickStatus(item.task.id, "in_progress")}
                  >
                    Start Work
                  </button>
                {/if}
                <button
                  type="button"
                  class="btn-sm btn-primary"
                  onclick={() => handleQuickStatus(item.task.id, "completed")}
                >
                  {item.task.assignee.toLowerCase() !==
                  item.task.evaluator.toLowerCase()
                    ? `Submit Review (@${item.task.evaluator})`
                    : "Complete"}
                </button>
                <button
                  type="button"
                  class="btn-sm"
                  onclick={() => onSelectTask(item.task.id)}
                >
                  Open Details & Notes
                </button>
              </div>
            </div>

            <div class="meta-row">
              <span>Importance: <strong>{item.task.importance}/5</strong></span>
              <span>· Difficulty: <strong>{item.task.difficulty}/5</strong></span>
              <span>
                · Estimate:
                <strong>{formatDurationMinutes(item.task.time_estimate_minutes)}</strong>
              </span>
              {#if item.breakdown.effective_deadline}
                <span>
                  · Effective Deadline:
                  <strong>{item.breakdown.effective_deadline}</strong>
                  {#if item.breakdown.slack_days !== null}
                    ({item.breakdown.slack_days.toFixed(1)}d slack)
                  {/if}
                </span>
              {/if}
              {#if item.breakdown.unblocks_count > 0}
                <span>
                  · Unblocks <strong>{item.breakdown.unblocks_count}</strong> downstream task(s)
                </span>
              {/if}
            </div>

            {#if item.task.tags.length > 0}
              <div class="meta-row">
                {#each item.task.tags as tag}
                  <span class="badge badge-tag">#{tag}</span>
                {/each}
              </div>
            {/if}
          </article>
        </li>
      {/each}
    </ol>
  {/if}

  <!-- Blocked Summary & Detailed Blocked Queue -->
  <section class="card" aria-labelledby="blocked-summary-heading">
    <h3 id="blocked-summary-heading" style="margin: 0 0 0.65rem 0; font-size: 1.08rem;">
      Blocked Tasks & Situational Awareness ({blockedTasks.length})
    </h3>

    {#if focusData && focusData.blocked_summary.length > 0}
      <div style="margin-bottom: 0.85rem;">
        <p style="margin: 0 0 0.4rem 0; font-size: 0.88rem; color: var(--text-secondary);">
          Grouped summary by project/tag and who is holding the blocker:
        </p>
        <ul style="margin: 0; padding-left: 1.2rem; font-size: 0.9rem;">
          {#each focusData.blocked_summary as grp}
            <li style="margin-bottom: 0.25rem;">
              <strong>#{grp.project_or_tag}</strong>: {grp.count} task(s) ({grp.task_ids.join(
                ", ",
              )}) waiting on <strong>@{grp.waiting_on_assignee}</strong>
              (blocker kind: <code>{grp.blocker_kind}</code>)
            </li>
          {/each}
        </ul>
      </div>
    {/if}

    {#if blockedTasks.length === 0}
      <p class="field-hint" style="margin: 0;">
        No blocked or in-review tasks for this persona.
      </p>
    {:else}
      <div style="display: flex; flex-direction: column; gap: 0.5rem;">
        {#each blockedTasks as bt (bt.task_id)}
          <div
            style="background: var(--bg-input); border: 1px solid var(--border-subtle); border-radius: 0.375rem; padding: 0.65rem 0.85rem; display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 0.5rem;"
          >
            <div>
              <span class="task-id-badge">{bt.task_id}</span>
              <strong style="margin-left: 0.4rem;">{bt.title}</strong>
              <span class="badge" style="margin-left: 0.4rem;">
                {STATUS_LABELS[bt.status]}
              </span>
              {#if bt.blocked_by.length > 0}
                <div style="font-size: 0.82rem; color: var(--text-secondary); margin-top: 0.25rem;">
                  Waiting on:
                  {#each bt.blocked_by as b, i}
                    {i > 0 ? ", " : ""}
                    <code>{b.blocker_id}</code> ({b.origin_kind} assigned to @{b.assignee})
                  {/each}
                </div>
              {/if}
            </div>
            <button
              type="button"
              class="btn-sm"
              onclick={() => onSelectTask(bt.task_id)}
            >
              Inspect Task
            </button>
          </div>
        {/each}
      </div>
    {/if}
  </section>
</section>
