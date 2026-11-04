<script lang="ts">
  import Select from "./ui/Select.svelte";
  import TaskCard from "./TaskCard.svelte";
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
    /** Account kind per user (`human` unless set); drives persona pills. */
    userKinds?: Record<string, string>;
    blockedTasks: BlockedTaskEntry[];
    knownProjects: string[];
    projectFilter: string;
    onProjectChange: (project: string) => void;
    onSelectTask: (id: string) => void;
    onPresetChange: (preset: FocusPreset) => void;
    onDataChanged: () => void;
    announce: (msg: string) => void;
  }

  let {
    focusData,
    userKinds = {},
    blockedTasks,
    knownProjects,
    projectFilter,
    onProjectChange,
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
    <h2 id="focus-view-heading" style="margin: 0; font-size: 1.35rem;">
      Focus Queue
    </h2>

    {#if focusData}
      <div class="control-inline">
        <label for="focus-project-select">Project:</label>
        <Select
          id="focus-project-select"
          size="sm"
          value={projectFilter}
          options={[
            ...[...new Set(["default", ...knownProjects])].map((project) => ({
              value: project,
              label: project,
            })),
            { value: "all", label: "All projects" },
          ]}
          onValueChange={(v) => onProjectChange(v)}
        />
      </div>

      <div class="control-inline">
        <label for="focus-preset-select">Preset:</label>
        <Select
          id="focus-preset-select"
          size="sm"
          value={focusData.preset}
          options={Object.entries(PRESET_LABELS).map(([key, label]) => ({
            value: key,
            label,
          }))}
          onValueChange={(v) => onPresetChange(v as FocusPreset)}
        />
      </div>
    {/if}
  </div>

  {#if !focusData}
    <p class="empty-state-message">Loading focus queue...</p>
  {:else if focusData.actionable_tasks.length === 0}
    <p class="empty-state-message">All caught up. No pending tasks in focus.</p>
  {:else}
    <ol
      style="list-style: none; padding: 0; margin: 0 0 1.5rem 0; display: flex; flex-direction: column; gap: 0.85rem;"
      aria-label="Prioritized Focus Tasks"
    >
      {#each focusData.actionable_tasks as item, idx (item.task.id)}
        <li>
          <TaskCard
            task={item.task}
            titleId={`focus-title-${item.task.id}`}
            rank={idx + 1}
            score={item.breakdown.total_score}
            quadrantLabel={QUADRANT_META[item.quadrant].badge}
            unblocksCount={item.breakdown.unblocks_count}
            deadline={item.breakdown.effective_deadline}
            slackDays={item.breakdown.slack_days}
            showStatus
            {userKinds}
            completeLabel={item.task.assignee.toLowerCase() !==
            item.task.evaluator.toLowerCase()
              ? `Submit for review (@${item.task.evaluator})`
              : "Mark complete"}
            onComplete={() => handleQuickStatus(item.task.id, "completed")}
            onOpen={() => onSelectTask(item.task.id)}
          />
        </li>
      {/each}
    </ol>
  {/if}

  <!-- Blocked Summary & Detailed Blocked Queue (collapsed when 0 items) -->
  {#if blockedTasks.length > 0 || (focusData && focusData.blocked_summary.length > 0)}
    <section class="card" aria-labelledby="blocked-summary-heading">
      <h3
        id="blocked-summary-heading"
        style="margin: 0 0 0.65rem 0; font-size: 1.08rem;"
      >
        Blocked Tasks ({blockedTasks.length})
      </h3>

      {#if focusData && focusData.blocked_summary.length > 0}
        <div style="margin-bottom: 0.85rem;">
          <ul style="margin: 0; padding-left: 1.2rem; font-size: 0.9rem;">
            {#each focusData.blocked_summary as grp}
              <li style="margin-bottom: 0.25rem;">
                <strong>#{grp.project_or_tag}</strong>: {grp.count} task(s) ({grp.task_ids.join(
                  ", ",
                )}) waiting on <strong>@{grp.waiting_on_assignee}</strong>
                (<code>{grp.blocker_kind}</code>)
              </li>
            {/each}
          </ul>
        </div>
      {/if}

      {#if blockedTasks.length > 0}
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
                  <div
                    style="font-size: 0.82rem; color: var(--text-secondary); margin-top: 0.25rem;"
                  >
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
  {/if}
</section>
