<script lang="ts">
  import DateFieldPicker from "./DateFieldPicker.svelte";
  import TagChipsInput from "./TagChipsInput.svelte";
  import RelationPicker, {
    type SelectedRelationEntry,
  } from "./RelationPicker.svelte";
  import {
    type TaskStatus,
    type AttachmentInput,
    STATUS_LABELS,
    STATUS_ORDER,
    RELATION_LABELS,
    createNewTask,
    pickAttachmentFileDialog,
    saveClipboardBytesForDraft,
    fileToBase64,
    parseDurationInput,
  } from "../lib/ipc";

  interface DraftAttachmentItem {
    id: string;
    path?: string;
    base64Data?: string;
    suggestedName: string;
    description: string | null;
    previewUrl?: string;
  }

  interface Props {
    activeUser: string;
    knownUsers: string[];
    knownTags: string[];
    onCreated: (createdId: string) => void;
    announce: (msg: string) => void;
  }

  let { activeUser, knownUsers, knownTags, onCreated, announce }: Props =
    $props();

  let title = $state<string>("");
  let description = $state<string>("");
  let status = $state<TaskStatus>("todo");
  let sgdbDate = $state<string>("");
  let dueDate = $state<string>("");
  let estimateInput = $state<string>("");
  let importance = $state<number>(3);
  let difficulty = $state<number>(3);
  let manualPriority = $state<number>(0);
  let urgentOverride = $state<boolean>(false);
  let tags = $state<string[]>([]);
  let reporter = $state<string>("");
  let assignee = $state<string>("");
  let evaluator = $state<string>("");

  let stagedRelations = $state<SelectedRelationEntry[]>([]);
  let pendingAttachments = $state<DraftAttachmentItem[]>([]);
  let nextAttachmentDesc = $state<string>("");
  let isDragging = $state<boolean>(false);

  let titleError = $state<string | null>(null);
  let dateError = $state<string | null>(null);
  let durationError = $state<string | null>(null);
  let formError = $state<string | null>(null);

  $effect(() => {
    if (!reporter) reporter = activeUser;
    if (!assignee) assignee = activeUser;
    if (!evaluator) evaluator = activeUser;
  });

  function basenameOfPath(rawPath: string): string {
    const parts = rawPath.split(/[/\\]/);
    return parts[parts.length - 1] || rawPath;
  }

  function addStagedRelation(entry: SelectedRelationEntry) {
    const exists = stagedRelations.some(
      (r) =>
        r.relation === entry.relation &&
        r.target_id.toUpperCase() === entry.target_id.toUpperCase(),
    );
    if (!exists) {
      stagedRelations = [...stagedRelations, entry];
      announce(
        `Added ${RELATION_LABELS[entry.relation]} relation to ${entry.target_id}`,
      );
    }
  }

  function removeStagedRelation(index: number) {
    stagedRelations = stagedRelations.filter((_, idx) => idx !== index);
  }

  async function handlePickAttachment() {
    try {
      const path = await pickAttachmentFileDialog();
      if (!path) return;
      const fileName = basenameOfPath(path);
      pendingAttachments = [
        ...pendingAttachments,
        {
          id: `${Date.now()}-${Math.random()}`,
          path,
          suggestedName: fileName,
          description: nextAttachmentDesc.trim() || null,
        },
      ];
      nextAttachmentDesc = "";
      announce(`Staged attachment ${fileName}`);
    } catch (err) {
      formError = String(err);
    }
  }

  async function handleFilesDropOrPaste(
    files: FileList | File[],
    fromClipboard: boolean,
  ) {
    for (const file of Array.from(files)) {
      try {
        const nativePath = (file as File & { path?: string }).path;
        const friendlyName =
          file.name ||
          `pasted-image-${new Date().toISOString().slice(11, 19).replace(/:/g, "")}.png`;
        if (!fromClipboard && nativePath) {
          pendingAttachments = [
            ...pendingAttachments,
            {
              id: `${Date.now()}-${Math.random()}`,
              path: nativePath,
              suggestedName: basenameOfPath(nativePath),
              description: nextAttachmentDesc.trim() || friendlyName,
            },
          ];
        } else {
          // Hold pasted or in-memory file in memory until task creation
          const b64 = await fileToBase64(file);
          pendingAttachments = [
            ...pendingAttachments,
            {
              id: `${Date.now()}-${Math.random()}`,
              base64Data: b64,
              suggestedName: friendlyName,
              description: nextAttachmentDesc.trim() || friendlyName,
              previewUrl: file.type.startsWith("image/") ? b64 : undefined,
            },
          ];
        }
        announce(`Staged attachment ${friendlyName}`);
      } catch (err) {
        formError = String(err);
      }
    }
    nextAttachmentDesc = "";
  }

  function resetForm() {
    title = "";
    description = "";
    status = "todo";
    sgdbDate = "";
    dueDate = "";
    estimateInput = "";
    importance = 3;
    difficulty = 3;
    manualPriority = 0;
    urgentOverride = false;
    tags = [];
    reporter = activeUser;
    assignee = activeUser;
    evaluator = activeUser;
    stagedRelations = [];
    pendingAttachments = [];
    nextAttachmentDesc = "";
    titleError = null;
    dateError = null;
    durationError = null;
    formError = null;
  }

  async function handleSubmit(e: SubmitEvent) {
    e.preventDefault();
    titleError = null;
    dateError = null;
    durationError = null;
    formError = null;

    if (!title.trim()) {
      titleError = "Task title is required.";
      announce(titleError);
      return;
    }

    if (sgdbDate.trim() && dueDate.trim() && sgdbDate.trim() > dueDate.trim()) {
      dateError =
        "Should Get Done By (SGDB) date must be on or before the Hard Due Date.";
      announce(dateError);
      return;
    }

    const parsedDur = parseDurationInput(estimateInput);
    if (parsedDur.error) {
      durationError = parsedDur.error;
      announce(durationError);
      return;
    }

    try {
      const attachmentsToSubmit: AttachmentInput[] = [];
      for (const item of pendingAttachments) {
        if (item.path) {
          attachmentsToSubmit.push({
            path: item.path,
            description: item.description,
          });
        } else if (item.base64Data) {
          const savedPath = await saveClipboardBytesForDraft(
            item.base64Data,
            item.suggestedName,
          );
          attachmentsToSubmit.push({
            path: savedPath,
            description: item.description,
          });
        }
      }

      const created = await createNewTask({
        title: title.trim(),
        description: description.trim() || null,
        status,
        due_date: dueDate.trim() || null,
        sgdb_date: sgdbDate.trim() || null,
        time_estimate_minutes: parsedDur.minutes,
        difficulty: Number(difficulty),
        importance: Number(importance),
        urgent_override: urgentOverride,
        manual_priority: Number(manualPriority),
        tags,
        reporter: reporter.trim() || activeUser,
        assignee: assignee.trim() || activeUser,
        evaluator: evaluator.trim() || activeUser,
        attachments: attachmentsToSubmit,
        initial_relations: stagedRelations.map((r) => ({
          relation: r.relation,
          target_id: r.target_id,
        })),
      });

      announce(`Created task ${created.id}: ${created.title}`);
      resetForm();
      onCreated(created.id);
    } catch (err) {
      formError = String(err);
      announce(`Error creating task: ${String(err)}`);
    }
  }
</script>

<section aria-labelledby="data-entry-heading">
  <div style="margin-bottom: 1rem;">
    <h2 id="data-entry-heading" style="margin: 0; font-size: 1.35rem;">
      New Task
    </h2>
  </div>

  {#if formError}
    <div class="banner-alert" role="alert">
      <span>{formError}</span>
      <button type="button" class="btn-sm" onclick={() => (formError = null)}>
        Dismiss
      </button>
    </div>
  {/if}

  <form
    onsubmit={handleSubmit}
    class="card"
    style="display: flex; flex-direction: column; gap: 1.25rem;"
    novalidate
  >
    <div class="form-field">
      <label for="entry-title">Task Title *</label>
      <input
        id="entry-title"
        type="text"
        bind:value={title}
        required
        aria-invalid={titleError ? "true" : "false"}
        aria-describedby={titleError ? "entry-title-err" : undefined}
        placeholder="Concise summary of what needs to be done"
      />
      {#if titleError}
        <span id="entry-title-err" class="field-error" role="alert">
          {titleError}
        </span>
      {/if}
    </div>

    <div class="form-field">
      <label for="entry-desc">Description</label>
      <textarea
        id="entry-desc"
        rows="4"
        bind:value={description}
        onpaste={(e) => {
          if (e.clipboardData?.files?.length) {
            void handleFilesDropOrPaste(e.clipboardData.files, true);
          }
        }}
        placeholder="Detailed acceptance criteria, notes, or context. Paste images directly with Cmd/Ctrl+V."
      ></textarea>
    </div>

    <div class="form-grid">
      <div class="form-field">
        <label for="entry-status">Initial Status</label>
        <select id="entry-status" bind:value={status}>
          {#each STATUS_ORDER as st}
            <option value={st}>{STATUS_LABELS[st]}</option>
          {/each}
        </select>
      </div>

      <div>
        <DateFieldPicker
          id="entry-sgdb"
          label="Should Get Done By (SGDB Date)"
          bind:value={sgdbDate}
          invalid={Boolean(dateError)}
          describedBy={dateError ? "entry-date-err" : undefined}
        />
        {#if dateError}
          <span id="entry-date-err" class="field-error" role="alert">
            {dateError}
          </span>
        {/if}
      </div>

      <div>
        <DateFieldPicker
          id="entry-due"
          label="Hard Due Date"
          bind:value={dueDate}
          invalid={Boolean(dateError)}
        />
      </div>

      <div class="form-field">
        <label for="entry-estimate">Time Estimate</label>
        <input
          id="entry-estimate"
          type="text"
          bind:value={estimateInput}
          placeholder="e.g. 30m, 2h, 1h30m, 3d"
          aria-invalid={durationError ? "true" : "false"}
          aria-describedby={durationError ? "entry-dur-err" : undefined}
        />
        {#if durationError}
          <span id="entry-dur-err" class="field-error" role="alert">
            {durationError}
          </span>
        {/if}
      </div>

      <div class="form-field">
        <label for="entry-importance">Importance (1–5)</label>
        <input
          id="entry-importance"
          type="number"
          min="1"
          max="5"
          bind:value={importance}
        />
      </div>

      <div class="form-field">
        <label for="entry-difficulty">Difficulty (1–5)</label>
        <input
          id="entry-difficulty"
          type="number"
          min="1"
          max="5"
          bind:value={difficulty}
        />
      </div>

      <div class="form-field">
        <label for="entry-manual-prio">Manual Priority (-5 to +5)</label>
        <input
          id="entry-manual-prio"
          type="number"
          min="-5"
          max="5"
          bind:value={manualPriority}
        />
      </div>

      <div class="form-field" style="justify-content: center;">
        <div class="control-inline" style="margin-top: 1.4rem;">
          <input
            id="entry-urgent"
            type="checkbox"
            bind:checked={urgentOverride}
          />
          <label for="entry-urgent">Urgent Override</label>
        </div>
      </div>
    </div>

    <div class="form-grid">
      <div class="form-field">
        <label for="entry-reporter">Reporter User</label>
        <input
          id="entry-reporter"
          type="text"
          list="user-datalist"
          bind:value={reporter}
        />
      </div>

      <div class="form-field">
        <label for="entry-assignee">Assignee User</label>
        <input
          id="entry-assignee"
          type="text"
          list="user-datalist"
          bind:value={assignee}
        />
      </div>

      <div class="form-field">
        <label for="entry-evaluator">Evaluator User (Approver)</label>
        <input
          id="entry-evaluator"
          type="text"
          list="user-datalist"
          bind:value={evaluator}
        />
      </div>
    </div>

    <datalist id="user-datalist">
      {#each knownUsers as u}
        <option value={u}></option>
      {/each}
    </datalist>

    <TagChipsInput id="entry-tags" bind:tags {knownTags} />

    <!-- Unified Fuzzy Relation Picker -->
    <div class="form-field">
      <span class="field-label">Task Relationships</span>
      <RelationPicker buttonLabel="Add Relation" onAdd={addStagedRelation} />

      {#if stagedRelations.length > 0}
        <div class="staged-relations-list">
          {#each stagedRelations as rel, idx (`${rel.relation}-${rel.target_id}`)}
            <span class="tag-chip">
              <span>
                <strong>{RELATION_LABELS[rel.relation]}</strong>: #{rel.target_id}
                {#if rel.target_title}
                  ({rel.target_title})
                {/if}
              </span>
              <button
                type="button"
                class="tag-chip-remove"
                onclick={() => removeStagedRelation(idx)}
                aria-label={`Remove ${RELATION_LABELS[rel.relation]} ${rel.target_id}`}
              >
                ×
              </button>
            </span>
          {/each}
        </div>
      {/if}
    </div>

    <!-- File & Image Attachments -->
    <div class="form-field">
      <span class="field-label">
        File & Image Attachments ({pendingAttachments.length})
      </span>

      <div
        class={`dropzone ${isDragging ? "dragging" : ""}`}
        role="region"
        aria-label="Drop or paste files to attach to this new task"
        ondragover={(e) => {
          e.preventDefault();
          isDragging = true;
        }}
        ondragleave={() => (isDragging = false)}
        ondrop={(e) => {
          e.preventDefault();
          isDragging = false;
          if (e.dataTransfer?.files?.length) {
            void handleFilesDropOrPaste(e.dataTransfer.files, false);
          }
        }}
        onpaste={(e) => {
          if (e.clipboardData?.files?.length) {
            void handleFilesDropOrPaste(e.clipboardData.files, true);
          }
        }}
      >
        <div class="form-grid" style="align-items: end; text-align: left;">
          <div class="form-field">
            <label for="entry-att-desc">
              Optional Alt Text / Description
            </label>
            <input
              id="entry-att-desc"
              type="text"
              bind:value={nextAttachmentDesc}
              placeholder="Defaults to filename"
            />
          </div>
          <div class="actions-row">
            <button type="button" class="btn-sm" onclick={handlePickAttachment}>
              Choose File from Disk...
            </button>
            <span class="field-hint">
              Or drag-and-drop / paste (Cmd/Ctrl+V) an image
            </span>
          </div>
        </div>
      </div>

      {#if pendingAttachments.length > 0}
        <ul class="draft-attachments-list">
          {#each pendingAttachments as att (att.id)}
            <li class="draft-attachment-item">
              <div class="draft-attachment-info">
                {#if att.previewUrl}
                  <img
                    src={att.previewUrl}
                    alt={att.description ?? att.suggestedName}
                    class="draft-attachment-thumb"
                  />
                {/if}
                <div>
                  <strong>{att.suggestedName}</strong>
                  {#if att.description && att.description !== att.suggestedName}
                    <span class="field-hint"> — "{att.description}"</span>
                  {/if}
                </div>
              </div>
              <button
                type="button"
                class="btn-sm btn-danger"
                onclick={() => {
                  pendingAttachments = pendingAttachments.filter(
                    (a) => a.id !== att.id,
                  );
                }}
              >
                Remove
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </div>

    <div class="actions-row">
      <button type="submit" class="btn-primary">Create Task</button>
      <button type="button" onclick={resetForm}>Reset Form</button>
    </div>
  </form>
</section>
