<script lang="ts">
  import {
    type TaskStatus,
    type RelationInputKind,
    type AttachmentInput,
    STATUS_LABELS,
    STATUS_ORDER,
    createNewTask,
    pickAttachmentFileDialog,
    saveClipboardBytesForDraft,
    fileToBase64,
    parseDurationInput,
  } from "../lib/ipc";

  interface Props {
    activePersona: string;
    knownPersonas: string[];
    knownTags: string[];
    onCreated: (createdId: string) => void;
    announce: (msg: string) => void;
  }

  let {
    activePersona,
    knownPersonas,
    knownTags,
    onCreated,
    announce,
  }: Props = $props();

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
  let tagsInput = $state<string>("");
  let reporter = $state<string>("");
  let assignee = $state<string>("");
  let evaluator = $state<string>("");

  let blocksIds = $state<string>("");
  let blockedByIds = $state<string>("");
  let subTaskOfIds = $state<string>("");
  let parentTaskOfIds = $state<string>("");
  let parallelIds = $state<string>("");
  let relatedIds = $state<string>("");

  let pendingAttachments = $state<AttachmentInput[]>([]);
  let nextAttachmentDesc = $state<string>("");
  let isDragging = $state<boolean>(false);

  let titleError = $state<string | null>(null);
  let dateError = $state<string | null>(null);
  let durationError = $state<string | null>(null);
  let formError = $state<string | null>(null);

  $effect(() => {
    if (!reporter) reporter = activePersona;
    if (!assignee) assignee = activePersona;
    if (!evaluator) evaluator = activePersona;
  });

  function splitShortIds(raw: string): string[] {
    return raw
      .split(/[,\s]+/)
      .map((s) => s.trim())
      .filter(Boolean);
  }

  async function handlePickAttachment() {
    try {
      const path = await pickAttachmentFileDialog();
      if (!path) return;
      pendingAttachments = [
        ...pendingAttachments,
        {
          path,
          description: nextAttachmentDesc.trim() || null,
        },
      ];
      nextAttachmentDesc = "";
      announce(`Staged attachment ${path}`);
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
        if (!fromClipboard && nativePath) {
          pendingAttachments = [
            ...pendingAttachments,
            {
              path: nativePath,
              description: nextAttachmentDesc.trim() || file.name,
            },
          ];
        } else {
          const b64 = await fileToBase64(file);
          const savedPath = await saveClipboardBytesForDraft(
            b64,
            file.name || "clipboard-attachment.png",
          );
          pendingAttachments = [
            ...pendingAttachments,
            {
              path: savedPath,
              description: nextAttachmentDesc.trim() || file.name || null,
            },
          ];
        }
        announce(`Staged attachment ${file.name || "clipboard file"}`);
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
    tagsInput = "";
    reporter = activePersona;
    assignee = activePersona;
    evaluator = activePersona;
    blocksIds = "";
    blockedByIds = "";
    subTaskOfIds = "";
    parentTaskOfIds = "";
    parallelIds = "";
    relatedIds = "";
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

    if (
      sgdbDate.trim() &&
      dueDate.trim() &&
      sgdbDate.trim() > dueDate.trim()
    ) {
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

    const tags = tagsInput
      .split(/[,\s]+/)
      .map((s) => s.trim())
      .filter(Boolean);

    const initial_relations: {
      relation: RelationInputKind;
      target_id: string;
    }[] = [];

    const pushRels = (kind: RelationInputKind, raw: string) => {
      for (const id of splitShortIds(raw)) {
        initial_relations.push({ relation: kind, target_id: id });
      }
    };

    pushRels("blocks", blocksIds);
    pushRels("blocked_by", blockedByIds);
    pushRels("sub_task_of", subTaskOfIds);
    pushRels("parent_task_of", parentTaskOfIds);
    pushRels("parallel", parallelIds);
    pushRels("related", relatedIds);

    try {
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
        reporter: reporter.trim() || activePersona,
        assignee: assignee.trim() || activePersona,
        evaluator: evaluator.trim() || activePersona,
        attachments: pendingAttachments,
        initial_relations,
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
      Task Data Entry
    </h2>
    <p style="margin: 0.2rem 0 0 0; color: var(--text-secondary); font-size: 0.88rem;">
      Create a task with full scheduling metadata, roles, graph dependencies, and
      file/image attachments.
    </p>
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

      <div class="form-field">
        <label for="entry-sgdb">Should Get Done By (SGDB Date)</label>
        <input
          id="entry-sgdb"
          type="date"
          bind:value={sgdbDate}
          aria-invalid={dateError ? "true" : "false"}
          aria-describedby="entry-date-hint"
        />
        <span id="entry-date-hint" class="field-hint">
          Target internal completion date (must be &le; Due Date if both are set).
        </span>
        {#if dateError}
          <span class="field-error" role="alert">{dateError}</span>
        {/if}
      </div>

      <div class="form-field">
        <label for="entry-due">Hard Due Date</label>
        <input
          id="entry-due"
          type="date"
          bind:value={dueDate}
          aria-invalid={dateError ? "true" : "false"}
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
          aria-describedby={durationError ? "entry-dur-err" : "entry-dur-hint"}
        />
        <span id="entry-dur-hint" class="field-hint">
          Supports <code>15m</code>, <code>2h</code>, <code>1d</code> (8h), or
          <code>1w</code> (40h).
        </span>
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
        <label for="entry-reporter">Reporter Persona</label>
        <input
          id="entry-reporter"
          type="text"
          list="persona-datalist"
          bind:value={reporter}
        />
      </div>

      <div class="form-field">
        <label for="entry-assignee">Assignee Persona</label>
        <input
          id="entry-assignee"
          type="text"
          list="persona-datalist"
          bind:value={assignee}
        />
      </div>

      <div class="form-field">
        <label for="entry-evaluator">Evaluator Persona (Approver)</label>
        <input
          id="entry-evaluator"
          type="text"
          list="persona-datalist"
          bind:value={evaluator}
        />
        <span class="field-hint">
          If Assignee &ne; Evaluator, completing this task moves it to In Review.
        </span>
      </div>
    </div>

    <datalist id="persona-datalist">
      {#each knownPersonas as p}
        <option value={p}></option>
      {/each}
    </datalist>

    <div class="form-field">
      <label for="entry-tags">
        Tags (single-word tokens, comma or space separated)
      </label>
      <input
        id="entry-tags"
        type="text"
        bind:value={tagsInput}
        placeholder="e.g. project-a, backend, a11y"
      />
      {#if knownTags.length > 0}
        <span class="field-hint">
          Existing tags: {knownTags.map((t) => `#${t}`).join(", ")}
        </span>
      {/if}
    </div>

    <!-- Initial Graph Relations -->
    <fieldset
      style="border: 1px solid var(--border-subtle); border-radius: 0.5rem; padding: 1rem;"
    >
      <legend style="font-weight: 700; padding: 0 0.4rem;">
        Initial Task Relations (Optional Short IDs, e.g. AB1234)
      </legend>
      <div class="form-grid">
        <div class="form-field">
          <label for="rel-blocks">Blocks Task ID(s)</label>
          <input
            id="rel-blocks"
            type="text"
            bind:value={blocksIds}
            placeholder="AB1234, CD5678"
          />
        </div>
        <div class="form-field">
          <label for="rel-blocked-by">Blocked By Task ID(s)</label>
          <input
            id="rel-blocked-by"
            type="text"
            bind:value={blockedByIds}
            placeholder="AB1234"
          />
        </div>
        <div class="form-field">
          <label for="rel-subtask-of">Sub-task Of (Parent ID)</label>
          <input
            id="rel-subtask-of"
            type="text"
            bind:value={subTaskOfIds}
            placeholder="AB1234"
          />
        </div>
        <div class="form-field">
          <label for="rel-parent-of">Parent Task Of (Child IDs)</label>
          <input
            id="rel-parent-of"
            type="text"
            bind:value={parentTaskOfIds}
            placeholder="AB1234"
          />
        </div>
        <div class="form-field">
          <label for="rel-parallel">Parallel With ID(s)</label>
          <input
            id="rel-parallel"
            type="text"
            bind:value={parallelIds}
            placeholder="AB1234"
          />
        </div>
        <div class="form-field">
          <label for="rel-related">Related To ID(s)</label>
          <input
            id="rel-related"
            type="text"
            bind:value={relatedIds}
            placeholder="AB1234"
          />
        </div>
      </div>
    </fieldset>

    <!-- File & Image Attachments -->
    <fieldset
      style="border: 1px solid var(--border-subtle); border-radius: 0.5rem; padding: 1rem;"
    >
      <legend style="font-weight: 700; padding: 0 0.4rem;">
        File & Image Attachments ({pendingAttachments.length})
      </legend>

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
              Optional Alt Text / Description for Next Attachment
            </label>
            <input
              id="entry-att-desc"
              type="text"
              bind:value={nextAttachmentDesc}
              placeholder="Defaults to filename for screen readers"
            />
          </div>
          <div class="actions-row">
            <button
              type="button"
              class="btn-sm"
              onclick={handlePickAttachment}
            >
              Choose File from Disk...
            </button>
            <span class="field-hint">
              Or drag-and-drop / paste (Cmd/Ctrl+V) an image
            </span>
          </div>
        </div>
      </div>

      {#if pendingAttachments.length > 0}
        <ul style="margin: 0.75rem 0 0 0; padding-left: 1.2rem; font-size: 0.88rem;">
          {#each pendingAttachments as att, idx}
            <li style="margin-bottom: 0.3rem;">
              <code>{att.path}</code>
              {#if att.description}
                — Alt: "{att.description}"
              {/if}
              <button
                type="button"
                class="btn-sm btn-danger"
                style="margin-left: 0.5rem;"
                onclick={() => {
                  pendingAttachments = pendingAttachments.filter(
                    (_, i) => i !== idx,
                  );
                }}
              >
                Remove
              </button>
            </li>
          {/each}
        </ul>
      {/if}
    </fieldset>

    <div class="actions-row">
      <button type="submit" class="btn-primary">Create Task</button>
      <button type="button" onclick={resetForm}>Reset Form</button>
    </div>
  </form>
</section>
