<script lang="ts">
  import ConfirmModal from "./ConfirmModal.svelte";
  import DateFieldPicker from "./DateFieldPicker.svelte";
  import RatingSlider from "./RatingSlider.svelte";
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
    validateDeadlines,
    validateUsername,
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
    isDirty?: boolean;
    onCreated: (createdId: string) => void;
    announce: (msg: string) => void;
  }

  let {
    activeUser,
    knownUsers,
    knownTags,
    isDirty = $bindable(false),
    onCreated,
    announce,
  }: Props = $props();

  let formElement = $state<HTMLFormElement | null>(null);

  let title = $state<string>("");
  let description = $state<string>("");
  let status = $state<TaskStatus>("todo");
  let sgdbDate = $state<string>("");
  let dueDate = $state<string>("");
  let estimateInput = $state<string>("");
  let importance = $state<number>(3);
  let difficulty = $state<number>(3);
  let manualPriority = $state<number>(3);
  let urgentOverride = $state<boolean>(false);
  let tags = $state<string[]>([]);
  let reporter = $state<string>("");
  let assignee = $state<string>("");
  let evaluator = $state<string>("");

  let stagedRelations = $state<SelectedRelationEntry[]>([]);
  let pendingAttachments = $state<DraftAttachmentItem[]>([]);
  let nextAttachmentDesc = $state<string>("");
  let isDragging = $state<boolean>(false);

  let sgdbPartial = $state<boolean>(false);
  let duePartial = $state<boolean>(false);
  let tagInputText = $state<string>("");
  let relationSearchQuery = $state<string>("");
  let formResetKey = $state<number>(0);
  let showResetConfirmModal = $state<boolean>(false);

  let titleTouched = $state<boolean>(false);
  let titleError = $state<string | null>(null);
  let sgdbError = $state<string | null>(null);
  let dueError = $state<string | null>(null);
  let durationError = $state<string | null>(null);
  let formError = $state<string | null>(null);

  let durationTimer: ReturnType<typeof setTimeout> | null = null;
  let dateTimer: ReturnType<typeof setTimeout> | null = null;

  let durationPreview = $derived.by((): string | null => {
    const trimmed = estimateInput.trim();
    if (!trimmed) return null;
    const parsed = parseDurationInput(trimmed);
    if (parsed.error || parsed.minutes === null || !parsed.normalized) {
      return null;
    }
    return `= ${parsed.normalized} (${parsed.minutes.toLocaleString()} min${parsed.minutes === 1 ? "" : "s"})`;
  });

  $effect(() => {
    if (!reporter) reporter = activeUser;
    if (!assignee) assignee = activeUser;
    if (!evaluator) evaluator = activeUser;
  });

  // Clear title error immediately when user types a valid title
  $effect(() => {
    if (title.trim() && titleError) {
      titleError = null;
    }
  });

  // Debounced validation on Time Estimate (400ms idle) + immediate clear when valid
  $effect(() => {
    const current = estimateInput;
    if (durationTimer) clearTimeout(durationTimer);
    if (!current.trim()) {
      durationError = null;
      return;
    }
    const check = parseDurationInput(current);
    if (!check.error && durationError) {
      durationError = null;
      return;
    }
    durationTimer = setTimeout(() => {
      const res = parseDurationInput(current);
      durationError = res.error;
    }, 400);
    return () => {
      if (durationTimer) clearTimeout(durationTimer);
    };
  });

  // Debounced validation on SGDB vs Hard Due Date + past date check (400ms idle)
  $effect(() => {
    const s = sgdbDate;
    const d = dueDate;
    if (dateTimer) clearTimeout(dateTimer);
    const check = validateDeadlines({
      sgdbDate: s,
      dueDate: d,
      checkPastSgdb: true,
      checkPastDue: true,
    });
    if (!check.sgdbError && sgdbError) {
      sgdbError = null;
    }
    if (!check.dueError && dueError) {
      dueError = null;
    }
    if (!check.sgdbError && !check.dueError) {
      return;
    }
    dateTimer = setTimeout(() => {
      const res = validateDeadlines({
        sgdbDate: s,
        dueDate: d,
        checkPastSgdb: true,
        checkPastDue: true,
      });
      sgdbError = res.sgdbError;
      dueError = res.dueError;
    }, 400);
    return () => {
      if (dateTimer) clearTimeout(dateTimer);
    };
  });

  function validateTitleNow(): boolean {
    if (!title.trim()) {
      titleError = "Task title is required.";
      return false;
    }
    titleError = null;
    return true;
  }

  function validateDurationNow(normalizeOnValid = false): boolean {
    if (durationTimer) clearTimeout(durationTimer);
    const parsed = parseDurationInput(estimateInput);
    durationError = parsed.error;
    if (normalizeOnValid && !parsed.error && parsed.normalized) {
      estimateInput = parsed.normalized;
    }
    return !parsed.error;
  }

  function validateDatesNow(): boolean {
    if (dateTimer) clearTimeout(dateTimer);
    const res = validateDeadlines({
      sgdbDate,
      dueDate,
      checkPastSgdb: true,
      checkPastDue: true,
    });
    sgdbError = res.sgdbError;
    dueError = res.dueError;
    return !res.sgdbError && !res.dueError;
  }

  function isFormDirty(): boolean {
    return (
      title.trim().length > 0 ||
      description.trim().length > 0 ||
      status !== "todo" ||
      sgdbDate.trim().length > 0 ||
      dueDate.trim().length > 0 ||
      sgdbPartial ||
      duePartial ||
      estimateInput.trim().length > 0 ||
      importance !== 3 ||
      difficulty !== 3 ||
      manualPriority !== 3 ||
      urgentOverride ||
      tags.length > 0 ||
      tagInputText.trim().length > 0 ||
      stagedRelations.length > 0 ||
      relationSearchQuery.trim().length > 0 ||
      pendingAttachments.length > 0 ||
      nextAttachmentDesc.trim().length > 0 ||
      (reporter.trim().length > 0 && reporter.trim() !== activeUser) ||
      (assignee.trim().length > 0 && assignee.trim() !== activeUser) ||
      (evaluator.trim().length > 0 && evaluator.trim() !== activeUser)
    );
  }

  $effect(() => {
    isDirty = isFormDirty();
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
    if (durationTimer) clearTimeout(durationTimer);
    if (dateTimer) clearTimeout(dateTimer);
    title = "";
    description = "";
    status = "todo";
    sgdbDate = "";
    dueDate = "";
    sgdbPartial = false;
    duePartial = false;
    estimateInput = "";
    importance = 3;
    difficulty = 3;
    manualPriority = 3;
    urgentOverride = false;
    tags = [];
    tagInputText = "";
    reporter = activeUser;
    assignee = activeUser;
    evaluator = activeUser;
    stagedRelations = [];
    relationSearchQuery = "";
    pendingAttachments = [];
    nextAttachmentDesc = "";
    titleTouched = false;
    titleError = null;
    sgdbError = null;
    dueError = null;
    durationError = null;
    formError = null;
    formResetKey += 1;
  }

  function handleCancelOrReset() {
    if (isFormDirty()) {
      showResetConfirmModal = true;
      return;
    }
    resetForm();
    announce("Form reset");
  }

  function focusNextField(current: HTMLElement) {
    if (!formElement) return;
    const focusables = Array.from(
      formElement.querySelectorAll<HTMLElement>(
        'input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [role="slider"]:not([disabled]), button:not([disabled])',
      ),
    ).filter((el) => el.tabIndex >= 0 && el.offsetParent !== null);
    const idx = focusables.indexOf(current);
    if (idx >= 0 && idx + 1 < focusables.length) {
      focusables[idx + 1].focus();
    }
  }

  function handleFormKeydown(e: KeyboardEvent) {
    // Explicit global submit via Cmd+Enter (macOS) or Ctrl+Enter (Win/Linux)
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      void submitTask();
      return;
    }

    // Esc resets editable state (with confirmation if dirty)
    if (e.key === "Escape") {
      const target = e.target as HTMLElement | null;
      if (
        target?.closest(
          "[data-combobox-input], [data-date-field-input], [data-time-field-input]",
        )
      ) {
        return;
      }
      e.preventDefault();
      handleCancelOrReset();
      return;
    }

    // Prevent bare Enter inside single-line <input> from submitting the form;
    // advance focus to the next field instead.
    if (e.key === "Enter" && !e.shiftKey && !e.altKey) {
      const target = e.target as HTMLElement | null;
      if (
        target &&
        target.tagName === "INPUT" &&
        (target as HTMLInputElement).type !== "submit" &&
        (target as HTMLInputElement).type !== "button" &&
        (target as HTMLInputElement).type !== "checkbox"
      ) {
        e.preventDefault();
        focusNextField(target);
      }
    }
  }

  async function submitTask() {
    titleTouched = true;
    formError = null;

    const titleOk = validateTitleNow();
    const datesOk = validateDatesNow();
    const durOk = validateDurationNow(true);

    if (!titleOk) {
      announce(titleError!);
      return;
    }
    if (!datesOk) {
      announce(sgdbError ?? dueError ?? "Invalid date selection");
      return;
    }
    if (!durOk) {
      announce(durationError!);
      return;
    }

    for (const [label, raw] of [
      ["Assignee", assignee.trim() || activeUser],
      ["Evaluator", evaluator.trim() || activeUser],
      ["Reporter", reporter.trim() || activeUser],
    ] as const) {
      const check = validateUsername(raw);
      if (check.error) {
        formError = `${label}: ${check.error}`;
        announce(formError);
        return;
      }
    }

    const parsedDur = parseDurationInput(estimateInput);

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

  async function handleSubmit(e: SubmitEvent) {
    e.preventDefault();
    await submitTask();
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

  <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
  <form
    bind:this={formElement}
    onsubmit={handleSubmit}
    onkeydown={handleFormKeydown}
    class="card"
    style="display: flex; flex-direction: column; gap: 1.25rem;"
    novalidate
  >
    {#key formResetKey}
      <div class="form-field">
        <label for="entry-title">Task Title *</label>
        <input
          id="entry-title"
          type="text"
          bind:value={title}
          required
          onblur={() => {
            titleTouched = true;
            validateTitleNow();
          }}
          aria-invalid={titleError ? "true" : "false"}
          aria-describedby={titleError ? "entry-title-err" : undefined}
          placeholder="Concise summary of what needs to be done"
        />
        {#if titleError && titleTouched}
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

      <div class="form-grid-dates">
        <DateFieldPicker
          id="entry-sgdb"
          label="SGDB Date (Soft Deadline)"
          bind:value={sgdbDate}
          bind:hasPartialInput={sgdbPartial}
          invalid={Boolean(sgdbError)}
          errorMessage={sgdbError}
          onBlur={() => {
            validateDatesNow();
          }}
        />

        <DateFieldPicker
          id="entry-due"
          label="Hard Due Date"
          bind:value={dueDate}
          bind:hasPartialInput={duePartial}
          minDate={sgdbDate}
          invalid={Boolean(dueError)}
          errorMessage={dueError}
          onBlur={() => {
            validateDatesNow();
          }}
        />
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
          <div class="date-picker-label-row">
            <label for="entry-estimate">Time Estimate</label>
            {#if durationPreview && !durationError}
              <span class="duration-preview-badge">{durationPreview}</span>
            {/if}
          </div>
          <input
            id="entry-estimate"
            type="text"
            bind:value={estimateInput}
            onblur={() => {
              validateDurationNow(true);
            }}
            placeholder="e.g. 1w 2d 3h 20m, 2h 30m, 45m"
            aria-invalid={durationError ? "true" : "false"}
            aria-describedby={durationError ? "entry-dur-err" : undefined}
          />
          {#if durationError}
            <span id="entry-dur-err" class="field-error" role="alert">
              {durationError}
            </span>
          {/if}
        </div>
      </div>

      <div class="form-grid">
        <RatingSlider
          id="entry-importance"
          label="Importance (1–5)"
          bind:value={importance}
          min={1}
          max={5}
          step={1}
        />

        <RatingSlider
          id="entry-difficulty"
          label="Difficulty (1–5)"
          bind:value={difficulty}
          min={1}
          max={5}
          step={1}
        />

        <RatingSlider
          id="entry-manual-prio"
          label="Manual Priority (1–5)"
          bind:value={manualPriority}
          min={1}
          max={5}
          step={1}
        />

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

      <TagChipsInput
        id="entry-tags"
        bind:tags
        bind:inputText={tagInputText}
        {knownTags}
      />

      <!-- Unified Fuzzy Relation Picker -->
      <div class="form-field">
        <span class="field-label">Task Relationships</span>
        <RelationPicker
          buttonLabel="Add Relation"
          bind:searchQuery={relationSearchQuery}
          onAdd={addStagedRelation}
        />

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
    {/key}

    <div class="actions-row">
      <button
        type="submit"
        class="btn-primary"
        title="Create Task (Cmd/Ctrl+Enter)"
      >
        Create Task
      </button>
      <button type="button" onclick={handleCancelOrReset}>Reset Form</button>
      <span class="field-hint" style="margin-left: auto;">
        Tip: Press Cmd/Ctrl+Enter to submit from any field
      </span>
    </div>
  </form>

  <ConfirmModal
    bind:open={showResetConfirmModal}
    title="Reset New Task Form?"
    description="This will clear all fields, tags, relationships, and staged attachments you have entered."
    confirmLabel="Clear Form"
    cancelLabel="Cancel"
    onConfirm={() => {
      resetForm();
      announce("Form reset");
    }}
    onCancel={() => {
      showResetConfirmModal = false;
    }}
  />
</section>
