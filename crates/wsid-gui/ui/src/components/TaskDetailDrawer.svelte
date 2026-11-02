<script lang="ts">
  import { onMount } from "svelte";
  import AttachmentGallery from "./AttachmentGallery.svelte";
  import {
    type TaskFullDetail,
    type TaskStatus,
    type RelationInputKind,
    type AttachmentInput,
    STATUS_LABELS,
    STATUS_ORDER,
    getTaskDetailData,
    transitionStatus,
    updateExistingTask,
    removeTask,
    addCommentNote,
    removeCommentNote,
    addRelation,
    removeRelation,
    dismissSubtaskBlocker,
    pickAttachmentFileDialog,
    attachFileFromDisk,
    attachClipboardBytes,
    saveClipboardBytesForDraft,
    fileToBase64,
    parseDurationInput,
    formatDurationMinutes,
  } from "../lib/ipc";

  interface Props {
    taskId: string;
    activePersona: string;
    onClose: () => void;
    onSelectTask: (id: string) => void;
    onDataChanged: () => void;
    announce: (msg: string) => void;
  }

  let {
    taskId,
    activePersona,
    onClose,
    onSelectTask,
    onDataChanged,
    announce,
  }: Props = $props();

  let detail = $state<TaskFullDetail | null>(null);
  let loading = $state<boolean>(true);
  let errorMsg = $state<string | null>(null);

  // Inline edit state
  let editingTask = $state<boolean>(false);
  let editTitle = $state<string>("");
  let editDescription = $state<string>("");
  let editDueDate = $state<string>("");
  let editSgdbDate = $state<string>("");
  let editEstimateInput = $state<string>("");
  let editDifficulty = $state<number>(3);
  let editImportance = $state<number>(3);
  let editUrgentOverride = $state<boolean>(false);
  let editManualPriority = $state<number>(0);
  let editTagsInput = $state<string>("");
  let editAssignee = $state<string>("");
  let editEvaluator = $state<string>("");
  let editReporter = $state<string>("");

  // Relation linker state
  let newRelKind = $state<RelationInputKind>("blocks");
  let newRelTargetId = $state<string>("");

  // Task-level attachment state
  let newTaskAttachmentDesc = $state<string>("");
  let isDraggingTaskFile = $state<boolean>(false);

  // Comment composer state
  let commentBody = $state<string>("");
  let commentSpawnSubtasks = $state<boolean>(true);
  let pendingCommentAttachments = $state<AttachmentInput[]>([]);
  let pendingCommentDesc = $state<string>("");

  let drawerElement = $state<HTMLElement | null>(null);

  async function refreshDetail(targetId: string) {
    loading = true;
    errorMsg = null;
    try {
      const loaded = await getTaskDetailData(targetId);
      detail = loaded;
      syncEditFields(loaded);
    } catch (err) {
      errorMsg = String(err);
    } finally {
      loading = false;
    }
  }

  function syncEditFields(d: TaskFullDetail) {
    editTitle = d.task.title;
    editDescription = d.task.description;
    editDueDate = d.task.due_date ?? "";
    editSgdbDate = d.task.sgdb_date ?? "";
    editEstimateInput =
      d.task.time_estimate_minutes !== null
        ? formatDurationMinutes(d.task.time_estimate_minutes)
        : "";
    editDifficulty = d.task.difficulty;
    editImportance = d.task.importance;
    editUrgentOverride = d.task.urgent_override;
    editManualPriority = d.task.manual_priority;
    editTagsInput = d.task.tags.join(", ");
    editAssignee = d.task.assignee;
    editEvaluator = d.task.evaluator;
    editReporter = d.task.reporter;
  }

  $effect(() => {
    void refreshDetail(taskId);
  });

  onMount(() => {
    drawerElement?.focus();
  });

  async function handleStatusChange(newStatus: TaskStatus) {
    if (!detail) return;
    try {
      const outcome = await transitionStatus(detail.task.id, newStatus);
      if (outcome.spawned_review_task) {
        announce(
          `Task ${detail.task.id} moved to In Review and spawned review subtask ${outcome.spawned_review_task.id} for @${outcome.spawned_review_task.assignee}`,
        );
      } else if (outcome.auto_completed_parent_tasks.length > 0) {
        const parentIds = outcome.auto_completed_parent_tasks
          .map((t) => t.id)
          .join(", ");
        announce(
          `Completed review ${detail.task.id} and automatically completed parent task ${parentIds}`,
        );
      } else {
        announce(
          `Task ${detail.task.id} status changed to ${STATUS_LABELS[outcome.task.status]}`,
        );
      }
      await refreshDetail(detail.task.id);
      onDataChanged();
    } catch (err) {
      errorMsg = String(err);
      announce(`Status transition error: ${String(err)}`);
    }
  }

  async function handleSaveEdits(e: SubmitEvent) {
    e.preventDefault();
    if (!detail) return;
    errorMsg = null;

    if (
      editSgdbDate.trim() &&
      editDueDate.trim() &&
      editSgdbDate.trim() > editDueDate.trim()
    ) {
      errorMsg =
        "Should Get Done By (SGDB) date must be on or before Due Date.";
      announce(errorMsg);
      return;
    }

    const parsedDur = parseDurationInput(editEstimateInput);
    if (parsedDur.error) {
      errorMsg = parsedDur.error;
      announce(errorMsg);
      return;
    }

    const tags = editTagsInput
      .split(/[,\s]+/)
      .map((s) => s.trim())
      .filter(Boolean);

    try {
      await updateExistingTask({
        id: detail.task.id,
        title: editTitle,
        description: editDescription,
        due_date: editDueDate.trim() || null,
        sgdb_date: editSgdbDate.trim() || null,
        time_estimate_minutes: parsedDur.minutes,
        difficulty: Number(editDifficulty),
        importance: Number(editImportance),
        urgent_override: editUrgentOverride,
        manual_priority: Number(editManualPriority),
        tags,
        assignee: editAssignee,
        evaluator: editEvaluator,
        reporter: editReporter,
      });
      editingTask = false;
      announce(`Saved changes to task ${detail.task.id}`);
      await refreshDetail(detail.task.id);
      onDataChanged();
    } catch (err) {
      errorMsg = String(err);
      announce(`Error saving task: ${String(err)}`);
    }
  }

  async function handleDeleteTask() {
    if (!detail) return;
    const id = detail.task.id;
    try {
      await removeTask(id);
      announce(`Deleted task ${id}`);
      onDataChanged();
      onClose();
    } catch (err) {
      errorMsg = String(err);
      announce(`Failed to delete task: ${String(err)}`);
    }
  }

  async function handlePickTaskAttachment() {
    if (!detail) return;
    try {
      const path = await pickAttachmentFileDialog();
      if (!path) return;
      await attachFileFromDisk({
        taskId: detail.task.id,
        filePath: path,
        description: newTaskAttachmentDesc.trim() || null,
      });
      newTaskAttachmentDesc = "";
      announce(`Attached file to task ${detail.task.id}`);
      await refreshDetail(detail.task.id);
      onDataChanged();
    } catch (err) {
      errorMsg = String(err);
      announce(`Attachment error: ${String(err)}`);
    }
  }

  async function handleTaskPasteOrDropFiles(
    files: FileList | File[],
    fromClipboard: boolean,
  ) {
    if (!detail) return;
    for (const file of Array.from(files)) {
      try {
        const nativePath = (file as File & { path?: string }).path;
        if (!fromClipboard && nativePath) {
          await attachFileFromDisk({
            taskId: detail.task.id,
            filePath: nativePath,
            description: newTaskAttachmentDesc.trim() || file.name,
          });
        } else {
          const b64 = await fileToBase64(file);
          await attachClipboardBytes({
            taskId: detail.task.id,
            base64Data: b64,
            suggestedName: file.name || "clipboard.png",
            description: newTaskAttachmentDesc.trim() || null,
          });
        }
        announce(
          `Attached ${file.name || "clipboard image"} to ${detail.task.id}`,
        );
      } catch (err) {
        errorMsg = String(err);
        announce(`Failed to attach file: ${String(err)}`);
      }
    }
    newTaskAttachmentDesc = "";
    await refreshDetail(detail.task.id);
    onDataChanged();
  }

  async function handleAddRelation(e: SubmitEvent) {
    e.preventDefault();
    if (!detail || !newRelTargetId.trim()) return;
    try {
      await addRelation(detail.task.id, newRelKind, newRelTargetId.trim());
      announce(
        `Linked ${detail.task.id} (${newRelKind}) to ${newRelTargetId.trim().toUpperCase()}`,
      );
      newRelTargetId = "";
      await refreshDetail(detail.task.id);
      onDataChanged();
    } catch (err) {
      errorMsg = String(err);
      announce(`Relation error: ${String(err)}`);
    }
  }

  async function handleUnlink(kind: RelationInputKind, targetId: string) {
    if (!detail) return;
    try {
      await removeRelation(detail.task.id, kind, targetId);
      announce(`Removed ${kind} link with ${targetId}`);
      await refreshDetail(detail.task.id);
      onDataChanged();
    } catch (err) {
      errorMsg = String(err);
    }
  }

  async function handleUnblockSubtask(subtaskId: string, dismiss: boolean) {
    if (!detail) return;
    try {
      await dismissSubtaskBlocker(subtaskId, dismiss);
      announce(
        dismiss
          ? `Dismissed and completed blocking subtask ${subtaskId}`
          : `Converted ${subtaskId} to non-blocking related task`,
      );
      await refreshDetail(detail.task.id);
      onDataChanged();
    } catch (err) {
      errorMsg = String(err);
      announce(`Unblock error: ${String(err)}`);
    }
  }

  async function handlePickCommentAttachment() {
    try {
      const path = await pickAttachmentFileDialog();
      if (!path) return;
      pendingCommentAttachments = [
        ...pendingCommentAttachments,
        {
          path,
          description: pendingCommentDesc.trim() || null,
        },
      ];
      pendingCommentDesc = "";
      announce(`Added file ${path} to pending comment`);
    } catch (err) {
      errorMsg = String(err);
    }
  }

  async function handleCommentPaste(e: ClipboardEvent) {
    const items = e.clipboardData?.files;
    if (!items || items.length === 0) return;
    for (const file of Array.from(items)) {
      try {
        const b64 = await fileToBase64(file);
        const savedPath = await saveClipboardBytesForDraft(
          b64,
          file.name || "comment-paste.png",
        );
        pendingCommentAttachments = [
          ...pendingCommentAttachments,
          {
            path: savedPath,
            description:
              pendingCommentDesc.trim() || file.name || "Pasted image",
          },
        ];
        announce(`Pasted image staged for comment`);
      } catch (err) {
        errorMsg = String(err);
      }
    }
  }

  async function handlePostComment(e: SubmitEvent) {
    e.preventDefault();
    if (!detail || !commentBody.trim()) return;
    errorMsg = null;
    try {
      const outcome = await addCommentNote({
        taskId: detail.task.id,
        author: activePersona,
        body: commentBody.trim(),
        spawnSubtasks: commentSpawnSubtasks,
        attachments: pendingCommentAttachments,
      });
      commentBody = "";
      pendingCommentAttachments = [];
      pendingCommentDesc = "";

      const spawnedCount = outcome.spawned_tasks.length;
      const reusedCount = outcome.reused_tasks.length;
      announce(
        `Posted note on ${detail.task.id}${
          spawnedCount > 0 ? ` (${spawnedCount} subtask(s) spawned)` : ""
        }${reusedCount > 0 ? ` (${reusedCount} open subtask(s) updated)` : ""}`,
      );
      await refreshDetail(detail.task.id);
      onDataChanged();
    } catch (err) {
      errorMsg = String(err);
      announce(`Failed to post comment: ${String(err)}`);
    }
  }

  async function handleDeleteNote(noteId: number) {
    if (!detail) return;
    try {
      await removeCommentNote(noteId);
      announce(`Deleted comment #${noteId}`);
      await refreshDetail(detail.task.id);
      onDataChanged();
    } catch (err) {
      errorMsg = String(err);
    }
  }
</script>

<div
  class="modal-backdrop"
  role="presentation"
  onclick={(e) => {
    if (e.target === e.currentTarget) onClose();
  }}
>
  <div
    bind:this={drawerElement}
    class="detail-drawer"
    role="dialog"
    aria-modal="true"
    aria-labelledby="drawer-task-title"
    tabindex="-1"
    onkeydown={(e) => {
      if (e.key === "Escape") onClose();
    }}
  >
    {#if loading && !detail}
      <p>Loading task {taskId}...</p>
    {:else if detail}
      <header
        style="display: flex; justify-content: space-between; align-items: flex-start; gap: 1rem; border-bottom: 1px solid var(--border-subtle); padding-bottom: 0.85rem;"
      >
        <div>
          <div class="meta-row" style="margin-bottom: 0.35rem;">
            <span class="task-id-badge">{detail.task.id}</span>
            <span class="badge">{STATUS_LABELS[detail.task.status]}</span>
            <span class="badge">Origin: {detail.task.origin_kind}</span>
            {#if detail.task.urgent_override}
              <span class="badge badge-urgent">Urgent Override</span>
            {/if}
          </div>
          <h2 id="drawer-task-title" style="margin: 0; font-size: 1.3rem;">
            {detail.task.title}
          </h2>
        </div>

        <div class="actions-row">
          <button
            type="button"
            class="btn-sm"
            onclick={() => (editingTask = !editingTask)}
          >
            {editingTask ? "Cancel Edit" : "Edit Fields"}
          </button>
          <button
            type="button"
            class="btn-sm btn-danger"
            onclick={handleDeleteTask}
          >
            Delete Task
          </button>
          <button
            type="button"
            onclick={onClose}
            aria-label="Close task detail drawer"
          >
            Close (Esc)
          </button>
        </div>
      </header>

      {#if errorMsg}
        <div class="banner-alert" role="alert">
          <span>{errorMsg}</span>
          <button
            type="button"
            class="btn-sm"
            onclick={() => (errorMsg = null)}
          >
            Dismiss
          </button>
        </div>
      {/if}

      <!-- Accessible Status Action Bar -->
      <section aria-label="Task Status Transition">
        <div class="control-inline" style="flex-wrap: wrap; gap: 0.5rem;">
          <label for="drawer-status-select">Transition Status:</label>
          <select
            id="drawer-status-select"
            value={detail.task.status}
            onchange={(e) =>
              handleStatusChange(
                (e.currentTarget as HTMLSelectElement).value as TaskStatus,
              )}
          >
            {#each STATUS_ORDER as st}
              <option value={st}>{STATUS_LABELS[st]}</option>
            {/each}
          </select>

          {#if detail.task.status !== "completed"}
            <button
              type="button"
              class="btn-primary btn-sm"
              onclick={() => handleStatusChange("completed")}
            >
              {detail.task.assignee.toLowerCase() !==
              detail.task.evaluator.toLowerCase()
                ? `Submit for Review (@${detail.task.evaluator})`
                : "Mark Completed"}
            </button>
          {/if}
        </div>
      </section>

      {#if editingTask}
        <form
          onsubmit={handleSaveEdits}
          class="card"
          style="display: flex; flex-direction: column; gap: 0.85rem;"
        >
          <h3 style="margin: 0; font-size: 1.05rem;">Edit Task Fields</h3>
          <div class="form-field">
            <label for="edit-title">Title *</label>
            <input
              id="edit-title"
              type="text"
              bind:value={editTitle}
              required
            />
          </div>

          <div class="form-field">
            <label for="edit-desc">Description</label>
            <textarea id="edit-desc" rows="4" bind:value={editDescription}
            ></textarea>
          </div>

          <div class="form-grid">
            <div class="form-field">
              <label for="edit-sgdb">Should Get Done By (SGDB Date)</label>
              <input id="edit-sgdb" type="date" bind:value={editSgdbDate} />
            </div>
            <div class="form-field">
              <label for="edit-due">Hard Due Date</label>
              <input id="edit-due" type="date" bind:value={editDueDate} />
            </div>
            <div class="form-field">
              <label for="edit-est">Time Estimate (e.g. 45m, 2h, 1d)</label>
              <input id="edit-est" type="text" bind:value={editEstimateInput} />
            </div>
            <div class="form-field">
              <label for="edit-imp">Importance (1–5)</label>
              <input
                id="edit-imp"
                type="number"
                min="1"
                max="5"
                bind:value={editImportance}
              />
            </div>
            <div class="form-field">
              <label for="edit-diff">Difficulty (1–5)</label>
              <input
                id="edit-diff"
                type="number"
                min="1"
                max="5"
                bind:value={editDifficulty}
              />
            </div>
            <div class="form-field">
              <label for="edit-prio">Manual Priority (-5 to +5)</label>
              <input
                id="edit-prio"
                type="number"
                min="-5"
                max="5"
                bind:value={editManualPriority}
              />
            </div>
            <div class="form-field">
              <label for="edit-assignee">Assignee</label>
              <input id="edit-assignee" type="text" bind:value={editAssignee} />
            </div>
            <div class="form-field">
              <label for="edit-evaluator">Evaluator</label>
              <input
                id="edit-evaluator"
                type="text"
                bind:value={editEvaluator}
              />
            </div>
            <div class="form-field">
              <label for="edit-reporter">Reporter</label>
              <input id="edit-reporter" type="text" bind:value={editReporter} />
            </div>
          </div>

          <div class="form-field">
            <label for="edit-tags"
              >Tags (single-word tokens, comma or space separated)</label
            >
            <input id="edit-tags" type="text" bind:value={editTagsInput} />
          </div>

          <div class="control-inline">
            <input
              id="edit-urgent"
              type="checkbox"
              bind:checked={editUrgentOverride}
            />
            <label for="edit-urgent">Urgent Override</label>
          </div>

          <div class="actions-row">
            <button type="submit" class="btn-primary">Save Changes</button>
            <button type="button" onclick={() => (editingTask = false)}>
              Cancel
            </button>
          </div>
        </form>
      {:else}
        <section class="card" aria-label="Task Details">
          <div class="meta-row" style="margin-bottom: 0.75rem;">
            <span><strong>Assignee:</strong> @{detail.task.assignee}</span>
            <span>· <strong>Evaluator:</strong> @{detail.task.evaluator}</span>
            <span>· <strong>Reporter:</strong> @{detail.task.reporter}</span>
            <span
              >· <strong>Importance:</strong> {detail.task.importance}/5</span
            >
            <span
              >· <strong>Difficulty:</strong> {detail.task.difficulty}/5</span
            >
            <span>
              · <strong>Estimate:</strong>
              {formatDurationMinutes(detail.task.time_estimate_minutes)}
            </span>
            {#if detail.task.sgdb_date}
              <span>· <strong>SGDB:</strong> {detail.task.sgdb_date}</span>
            {/if}
            {#if detail.task.due_date}
              <span>· <strong>Due:</strong> {detail.task.due_date}</span>
            {/if}
          </div>

          {#if detail.task.tags.length > 0}
            <div class="meta-row" style="margin-bottom: 0.75rem;">
              {#each detail.task.tags as tag}
                <span class="badge badge-tag">#{tag}</span>
              {/each}
            </div>
          {/if}

          <div style="white-space: pre-wrap; color: var(--text-primary);">
            {detail.task.description || "No description provided."}
          </div>
        </section>
      {/if}

      <!-- Task-Level Attachments -->
      <section class="card" aria-labelledby="drawer-attachments-heading">
        <h3
          id="drawer-attachments-heading"
          style="margin: 0 0 0.75rem 0; font-size: 1.02rem;"
        >
          Task Attachments ({detail.task.attachments.length})
        </h3>

        <AttachmentGallery
          attachments={detail.task.attachments}
          onChanged={() => {
            if (detail) void refreshDetail(detail.task.id);
            onDataChanged();
          }}
          {announce}
        />

        <div
          class={`dropzone ${isDraggingTaskFile ? "dragging" : ""}`}
          style="margin-top: 0.75rem;"
          role="region"
          aria-label="Attach files to task by drag-and-drop, clipboard paste, or file picker"
          ondragover={(e) => {
            e.preventDefault();
            isDraggingTaskFile = true;
          }}
          ondragleave={() => (isDraggingTaskFile = false)}
          ondrop={(e) => {
            e.preventDefault();
            isDraggingTaskFile = false;
            if (e.dataTransfer?.files?.length) {
              void handleTaskPasteOrDropFiles(e.dataTransfer.files, false);
            }
          }}
          onpaste={(e) => {
            if (e.clipboardData?.files?.length) {
              void handleTaskPasteOrDropFiles(e.clipboardData.files, true);
            }
          }}
        >
          <div class="form-grid" style="align-items: end; text-align: left;">
            <div class="form-field">
              <label for="task-att-desc">
                Optional Alt Text / Description for New Attachment
              </label>
              <input
                id="task-att-desc"
                type="text"
                bind:value={newTaskAttachmentDesc}
                placeholder="Defaults to filename if left blank"
              />
            </div>
            <div class="actions-row">
              <button
                type="button"
                class="btn-sm"
                onclick={handlePickTaskAttachment}
              >
                Choose File from Disk...
              </button>
              <span class="field-hint">
                Or drag-and-drop / paste (Cmd/Ctrl+V) an image here
              </span>
            </div>
          </div>
        </div>
      </section>

      <!-- Task Relations & Blocker Cleanup -->
      <section class="card" aria-labelledby="drawer-relations-heading">
        <h3
          id="drawer-relations-heading"
          style="margin: 0 0 0.75rem 0; font-size: 1.02rem;"
        >
          Task Dependencies & Graph Links
        </h3>

        <div
          style="display: flex; flex-direction: column; gap: 0.65rem; margin-bottom: 0.85rem;"
        >
          {#if detail.blockers.length > 0}
            <div>
              <strong>Blocked By ({detail.blockers.length}):</strong>
              <ul style="margin: 0.35rem 0 0 1.1rem; padding: 0;">
                {#each detail.blockers as b}
                  <li style="margin-bottom: 0.35rem;">
                    <button
                      type="button"
                      class="btn-sm"
                      onclick={() => onSelectTask(b.id)}
                    >
                      {b.id}: {b.title} ({STATUS_LABELS[b.status]}, @{b.assignee})
                    </button>
                    {#if b.origin_kind === "mention" || b.origin_kind === "review"}
                      <button
                        type="button"
                        class="btn-sm"
                        onclick={() => handleUnblockSubtask(b.id, true)}
                      >
                        Dismiss & Complete Subtask
                      </button>
                      <button
                        type="button"
                        class="btn-sm"
                        onclick={() => handleUnblockSubtask(b.id, false)}
                      >
                        Convert to Non-Blocking Related
                      </button>
                    {:else}
                      <button
                        type="button"
                        class="btn-sm btn-danger"
                        onclick={() => handleUnlink("blocked_by", b.id)}
                      >
                        Unlink
                      </button>
                    {/if}
                  </li>
                {/each}
              </ul>
            </div>
          {/if}

          {#if detail.blocking.length > 0}
            <div>
              <strong>Blocks ({detail.blocking.length}):</strong>
              <div class="actions-row">
                {#each detail.blocking as t}
                  <button
                    type="button"
                    class="btn-sm"
                    onclick={() => onSelectTask(t.id)}
                  >
                    {t.id}: {t.title}
                  </button>
                  <button
                    type="button"
                    class="btn-sm btn-danger"
                    aria-label={`Unlink blocks relation to ${t.id}`}
                    onclick={() => handleUnlink("blocks", t.id)}
                  >
                    ×
                  </button>
                {/each}
              </div>
            </div>
          {/if}

          {#if detail.parents.length > 0}
            <div>
              <strong>Sub-task Of ({detail.parents.length}):</strong>
              <div class="actions-row">
                {#each detail.parents as t}
                  <button
                    type="button"
                    class="btn-sm"
                    onclick={() => onSelectTask(t.id)}
                  >
                    {t.id}: {t.title}
                  </button>
                  <button
                    type="button"
                    class="btn-sm btn-danger"
                    onclick={() => handleUnlink("sub_task_of", t.id)}
                  >
                    ×
                  </button>
                {/each}
              </div>
            </div>
          {/if}

          {#if detail.subtasks.length > 0}
            <div>
              <strong>Subtasks ({detail.subtasks.length}):</strong>
              <div class="actions-row">
                {#each detail.subtasks as t}
                  <button
                    type="button"
                    class="btn-sm"
                    onclick={() => onSelectTask(t.id)}
                  >
                    {t.id}: {t.title} ({STATUS_LABELS[t.status]})
                  </button>
                {/each}
              </div>
            </div>
          {/if}

          {#if detail.parallel.length > 0}
            <div>
              <strong>Parallel Tasks ({detail.parallel.length}):</strong>
              <div class="actions-row">
                {#each detail.parallel as t}
                  <button
                    type="button"
                    class="btn-sm"
                    onclick={() => onSelectTask(t.id)}
                  >
                    {t.id}: {t.title}
                  </button>
                  <button
                    type="button"
                    class="btn-sm btn-danger"
                    onclick={() => handleUnlink("parallel", t.id)}
                  >
                    ×
                  </button>
                {/each}
              </div>
            </div>
          {/if}

          {#if detail.related.length > 0}
            <div>
              <strong>Related Tasks ({detail.related.length}):</strong>
              <div class="actions-row">
                {#each detail.related as t}
                  <button
                    type="button"
                    class="btn-sm"
                    onclick={() => onSelectTask(t.id)}
                  >
                    {t.id}: {t.title}
                  </button>
                  <button
                    type="button"
                    class="btn-sm btn-danger"
                    onclick={() => handleUnlink("related", t.id)}
                  >
                    ×
                  </button>
                {/each}
              </div>
            </div>
          {/if}
        </div>

        <form onsubmit={handleAddRelation} class="actions-row">
          <label for="rel-kind-select" class="sr-only">Relation Type</label>
          <select
            id="rel-kind-select"
            bind:value={newRelKind}
            style="width: auto;"
          >
            <option value="blocks">Blocks</option>
            <option value="blocked_by">Blocked By</option>
            <option value="sub_task_of">Sub-task Of</option>
            <option value="parent_task_of">Parent Task Of</option>
            <option value="parallel">Parallel With</option>
            <option value="related">Related To</option>
          </select>

          <label for="rel-target-id" class="sr-only">Target Task ID</label>
          <input
            id="rel-target-id"
            type="text"
            bind:value={newRelTargetId}
            placeholder="Target ID (e.g. AB1234)"
            style="width: 11rem;"
          />
          <button type="submit" class="btn-sm">Link Task</button>
        </form>
      </section>

      <!-- Notes & Comment Mention Thread -->
      <section class="card" aria-labelledby="drawer-notes-heading">
        <h3
          id="drawer-notes-heading"
          style="margin: 0 0 0.75rem 0; font-size: 1.02rem;"
        >
          Comments & Review Thread ({detail.notes.length})
        </h3>

        {#if detail.notes.length === 0}
          <p class="field-hint">
            No comments yet. Mention <code>@persona</code> to spawn an urgent
            blocking subtask, <code>@takenote:persona</code> for a non-blocking
            related note, or <code>#AB1234</code> to cross-link another task.
          </p>
        {:else}
          <div
            style="display: flex; flex-direction: column; gap: 0.75rem; margin-bottom: 1rem;"
          >
            {#each detail.notes as note (note.id)}
              <article
                style="background: var(--bg-input); border: 1px solid var(--border-subtle); border-radius: 0.5rem; padding: 0.75rem;"
              >
                <div
                  style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.4rem;"
                >
                  <span
                    style="font-size: 0.84rem; color: var(--text-secondary);"
                  >
                    <strong>@{note.author}</strong> · {note.created_at}
                  </span>
                  <button
                    type="button"
                    class="btn-sm btn-danger"
                    onclick={() => handleDeleteNote(note.id)}
                    aria-label={`Delete comment ${note.id} by ${note.author}`}
                  >
                    Delete Note
                  </button>
                </div>
                <div style="white-space: pre-wrap; margin-bottom: 0.5rem;">
                  {note.body}
                </div>
                {#if note.attachments.length > 0}
                  <AttachmentGallery
                    attachments={note.attachments}
                    onChanged={() => {
                      if (detail) void refreshDetail(detail.task.id);
                      onDataChanged();
                    }}
                    {announce}
                  />
                {/if}
              </article>
            {/each}
          </div>
        {/if}

        <form
          onsubmit={handlePostComment}
          style="display: flex; flex-direction: column; gap: 0.65rem;"
        >
          <div class="form-field">
            <label for="new-comment-body">
              Add Comment (as @{activePersona})
            </label>
            <textarea
              id="new-comment-body"
              rows="3"
              bind:value={commentBody}
              onpaste={handleCommentPaste}
              placeholder="Write a comment... Use @persona for blocking subtasks, @takenote:persona for non-blocking heads-up, or #AB1234 to link tasks. Paste images directly with Cmd/Ctrl+V."
            ></textarea>
          </div>

          {#if pendingCommentAttachments.length > 0}
            <ul style="margin: 0; padding-left: 1.2rem; font-size: 0.85rem;">
              {#each pendingCommentAttachments as att, idx}
                <li>
                  <code>{att.path}</code>
                  {#if att.description}
                    — Alt: "{att.description}"
                  {/if}
                  <button
                    type="button"
                    class="btn-sm btn-danger"
                    onclick={() => {
                      pendingCommentAttachments =
                        pendingCommentAttachments.filter((_, i) => i !== idx);
                    }}
                  >
                    Remove
                  </button>
                </li>
              {/each}
            </ul>
          {/if}

          <div class="actions-row" style="justify-content: space-between;">
            <div class="actions-row">
              <input
                id="comment-att-desc"
                type="text"
                bind:value={pendingCommentDesc}
                placeholder="Alt text for next comment attachment"
                style="width: 15rem;"
                aria-label="Alt text description for comment attachment"
              />
              <button
                type="button"
                class="btn-sm"
                onclick={handlePickCommentAttachment}
              >
                Attach File to Comment...
              </button>
              <label
                style="display: inline-flex; align-items: center; gap: 0.35rem; font-size: 0.85rem;"
              >
                <input type="checkbox" bind:checked={commentSpawnSubtasks} />
                Auto-spawn @mention subtasks
              </label>
            </div>

            <button type="submit" class="btn-primary">Post Comment</button>
          </div>
        </form>
      </section>
    {/if}
  </div>
</div>
