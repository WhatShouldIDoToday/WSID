<script lang="ts">
  import { Dialog } from "bits-ui";
  import Select from "./ui/Select.svelte";
  import UserPill from "./UserPill.svelte";
  import { onMount } from "svelte";
  import AttachmentGallery from "./AttachmentGallery.svelte";
  import DateFieldPicker from "./DateFieldPicker.svelte";
  import RatingSlider from "./RatingSlider.svelte";
  import TagChipsInput from "./TagChipsInput.svelte";
  import RichTextField from "./RichTextField.svelte";
  import RichTextDisplay from "./RichTextDisplay.svelte";
  import RelationPicker, {
    type SelectedRelationEntry,
  } from "./RelationPicker.svelte";
  import RelationTypeSelect from "./RelationTypeSelect.svelte";
  import ConfirmModal from "./ConfirmModal.svelte";
  import {
    type TaskFullDetail,
    type TaskStatus,
    type RelationInputKind,
    type AttachmentInput,
    STATUS_LABELS,
    STATUS_ORDER,
    RELATION_LABELS,
    getTaskDetailData,
    transitionStatus,
    updateExistingTask,
    describeUserNotice,
    withUserNotices,
    type UserNotice,
    userKindOf,
    removeTask,
    addCommentNote,
    removeCommentNote,
    addRelation,
    removeRelation,
    updateRelation,
    dismissSubtaskBlocker,
    pickAttachmentFileDialog,
    attachFileFromDisk,
    attachClipboardBytes,
    saveClipboardBytesForDraft,
    fileToBase64,
    parseDurationInput,
    formatDurationMinutes,
    validateDeadlines,
    formatDeadlineDisplay,
    validateUserRef,
    parseCommentMentions,
    tokenizeCommentBody,
  } from "../lib/ipc";
  import { subscribeDropzone } from "../lib/filedrop";

  interface DraftCommentAttachment {
    id: string;
    path?: string;
    base64Data?: string;
    suggestedName: string;
    description: string | null;
    previewUrl?: string;
  }

  interface Props {
    taskId: string;
    activeUser: string;
    knownUsers?: string[];
    /** Account kind per user (`human` unless set); drives typed-mention hints. */
    userKinds?: Record<string, string>;
    knownTags?: string[];
    knownProjects?: string[];
    onClose: () => void;
    onSelectTask: (id: string) => void;
    onDataChanged: () => void;
    announce: (msg: string) => void;
  }

  let {
    taskId,
    activeUser,
    knownUsers = [],
    userKinds = {},
    knownTags = [],
    knownProjects = [],
    onClose,
    onSelectTask,
    onDataChanged,
    announce,
  }: Props = $props();

  let detail = $state<TaskFullDetail | null>(null);
  let loading = $state<boolean>(true);
  let errorMsg = $state<string | null>(null);
  // Kind conflicts from typed user references (`@kind:user`) that need a visible, dismissible alert.
  let userNoticeAlerts = $state<string[]>([]);

  function setUserNoticeAlerts(notices: UserNotice[]) {
    userNoticeAlerts = notices
      .filter((n) => n.type === "kind_mismatch")
      .map(describeUserNotice);
  }

  // Inline edit state
  let editFormElement = $state<HTMLElement | null>(null);
  let editingTask = $state<boolean>(false);
  let editTitle = $state<string>("");
  let editDescription = $state<string>("");
  let editDueDate = $state<string>("");
  let editSgdbDate = $state<string>("");
  let editEstimateInput = $state<string>("");
  let editDifficulty = $state<number>(3);
  let editImportance = $state<number>(3);
  let editUrgentOverride = $state<boolean>(false);
  let editManualPriority = $state<number>(3);
  let editTags = $state<string[]>([]);
  let editProject = $state<string>("default");
  let editAssignee = $state<string>("");
  let editEvaluator = $state<string>("");
  let editReporter = $state<string>("");

  let editSgdbError = $state<string | null>(null);
  let editDueError = $state<string | null>(null);
  let editDurationError = $state<string | null>(null);
  let editDurationTimer: ReturnType<typeof setTimeout> | null = null;
  let editDateTimer: ReturnType<typeof setTimeout> | null = null;

  let editDurationPreview = $derived.by((): string | null => {
    const trimmed = editEstimateInput.trim();
    if (!trimmed) return null;
    const parsed = parseDurationInput(trimmed);
    if (parsed.error || parsed.minutes === null || !parsed.normalized) {
      return null;
    }
    return `= ${parsed.normalized} (${parsed.minutes.toLocaleString()} min${parsed.minutes === 1 ? "" : "s"})`;
  });

  // Task-level attachment state
  let newTaskAttachmentDesc = $state<string>("");
  let isDraggingTaskFile = $state<boolean>(false);
  let taskDropzoneEl = $state<HTMLDivElement | null>(null);

  // Comment composer state
  let commentBody = $state<string>("");
  let commentSpawnSubtasks = $state<boolean>(true);
  let pendingCommentAttachments = $state<DraftCommentAttachment[]>([]);
  let pendingCommentDesc = $state<string>("");

  let commentMentionTokens = $derived.by(() =>
    parseCommentMentions(commentBody, knownUsers, userKinds),
  );

  let drawerElement = $state<HTMLElement | null>(null);

  const SIDE_WIDTH_KEY = "wsid.task-modal.sidebar-width.v1";
  const SIDE_WIDTH_DEFAULT_REM = 22;
  const SIDE_WIDTH_MIN_REM = 16;
  const SIDE_WIDTH_MAX_REM = 36;

  function clampSideWidth(rem: number): number {
    return Math.min(SIDE_WIDTH_MAX_REM, Math.max(SIDE_WIDTH_MIN_REM, rem));
  }

  function loadSideWidth(): number {
    try {
      const stored = Number(window.localStorage.getItem(SIDE_WIDTH_KEY));
      if (Number.isFinite(stored) && stored > 0) return clampSideWidth(stored);
    } catch {
      // Storage may be unavailable; fall back to the default width.
    }
    return SIDE_WIDTH_DEFAULT_REM;
  }

  let sideWidthRem = $state<number>(loadSideWidth());

  function setSideWidth(rem: number) {
    sideWidthRem = clampSideWidth(rem);
    try {
      window.localStorage.setItem(SIDE_WIDTH_KEY, String(sideWidthRem));
    } catch {
      // Ignore storage write errors; the width still applies for this session.
    }
  }

  function remToPx(): number {
    return parseFloat(getComputedStyle(document.documentElement).fontSize);
  }

  let sideHandleDragging = $state<boolean>(false);

  function handleSideDragStart(e: PointerEvent) {
    if (e.button !== 0) return;
    sideHandleDragging = true;
    (e.currentTarget as HTMLElement).setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function handleSideDragMove(e: PointerEvent) {
    if (!sideHandleDragging || !editFormElement) return;
    const bodyRight = editFormElement.getBoundingClientRect().right;
    setSideWidth((bodyRight - e.clientX) / remToPx());
  }

  function handleSideDragEnd(e: PointerEvent) {
    if (!sideHandleDragging) return;
    sideHandleDragging = false;
    (e.currentTarget as HTMLElement).releasePointerCapture(e.pointerId);
  }

  function handleSideHandleKeydown(e: KeyboardEvent) {
    // The handle sits on the sidebar's left edge: ArrowLeft widens the sidebar.
    const step = e.shiftKey ? 4 : 1;
    if (e.key === "ArrowLeft") {
      e.preventDefault();
      setSideWidth(sideWidthRem + step);
    } else if (e.key === "ArrowRight") {
      e.preventDefault();
      setSideWidth(sideWidthRem - step);
    } else if (e.key === "Home") {
      e.preventDefault();
      setSideWidth(SIDE_WIDTH_MAX_REM);
    } else if (e.key === "End") {
      e.preventDefault();
      setSideWidth(SIDE_WIDTH_MIN_REM);
    }
  }

  function resetSideWidth() {
    setSideWidth(SIDE_WIDTH_DEFAULT_REM);
  }

  function basenameOfPath(rawPath: string): string {
    const parts = rawPath.split(/[/\\]/);
    return parts[parts.length - 1] || rawPath;
  }

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
    editTags = [...d.task.tags];
    editProject = d.task.project;
    editAssignee = d.task.assignee;
    editEvaluator = d.task.evaluator;
    editReporter = d.task.reporter;
    editSgdbError = null;
    editDueError = null;
    editDurationError = null;
  }

  $effect(() => {
    void refreshDetail(taskId);
  });

  // Debounced validation on editEstimateInput (400ms)
  $effect(() => {
    if (!editingTask) return;
    const current = editEstimateInput;
    if (editDurationTimer) clearTimeout(editDurationTimer);
    if (!current.trim()) {
      editDurationError = null;
      return;
    }
    const check = parseDurationInput(current);
    if (!check.error && editDurationError) {
      editDurationError = null;
      return;
    }
    editDurationTimer = setTimeout(() => {
      const res = parseDurationInput(current);
      editDurationError = res.error;
    }, 400);
    return () => {
      if (editDurationTimer) clearTimeout(editDurationTimer);
    };
  });

  // Debounced validation on editSgdbDate vs editDueDate (400ms)
  $effect(() => {
    if (!editingTask) return;
    const s = editSgdbDate;
    const d = editDueDate;
    const checkPastSgdb = s !== (detail?.task.sgdb_date ?? "");
    const checkPastDue = d !== (detail?.task.due_date ?? "");
    if (editDateTimer) clearTimeout(editDateTimer);
    const check = validateDeadlines({
      sgdbDate: s,
      dueDate: d,
      checkPastSgdb,
      checkPastDue,
    });
    if (!check.sgdbError && editSgdbError) {
      editSgdbError = null;
    }
    if (!check.dueError && editDueError) {
      editDueError = null;
    }
    if (!check.sgdbError && !check.dueError) {
      return;
    }
    editDateTimer = setTimeout(() => {
      const res = validateDeadlines({
        sgdbDate: s,
        dueDate: d,
        checkPastSgdb,
        checkPastDue,
      });
      editSgdbError = res.sgdbError;
      editDueError = res.dueError;
    }, 400);
    return () => {
      if (editDateTimer) clearTimeout(editDateTimer);
    };
  });

  function validateEditDurationNow(normalizeOnValid = false): boolean {
    if (editDurationTimer) clearTimeout(editDurationTimer);
    const parsed = parseDurationInput(editEstimateInput);
    editDurationError = parsed.error;
    if (normalizeOnValid && !parsed.error && parsed.normalized) {
      editEstimateInput = parsed.normalized;
    }
    return !parsed.error;
  }

  function validateEditDatesNow(): boolean {
    if (editDateTimer) clearTimeout(editDateTimer);
    const checkPastSgdb = editSgdbDate !== (detail?.task.sgdb_date ?? "");
    const checkPastDue = editDueDate !== (detail?.task.due_date ?? "");
    const res = validateDeadlines({
      sgdbDate: editSgdbDate,
      dueDate: editDueDate,
      checkPastSgdb,
      checkPastDue,
    });
    editSgdbError = res.sgdbError;
    editDueError = res.dueError;
    return !res.sgdbError && !res.dueError;
  }

  function isEditDirty(): boolean {
    if (!editingTask || !detail) return false;
    const origEst =
      detail.task.time_estimate_minutes !== null
        ? formatDurationMinutes(detail.task.time_estimate_minutes)
        : "";
    return (
      editTitle !== detail.task.title ||
      editDescription !== detail.task.description ||
      editDueDate !== (detail.task.due_date ?? "") ||
      editSgdbDate !== (detail.task.sgdb_date ?? "") ||
      editEstimateInput !== origEst ||
      editDifficulty !== detail.task.difficulty ||
      editImportance !== detail.task.importance ||
      editManualPriority !== detail.task.manual_priority ||
      editUrgentOverride !== detail.task.urgent_override ||
      editAssignee !== detail.task.assignee ||
      editEvaluator !== detail.task.evaluator ||
      editReporter !== detail.task.reporter ||
      editProject !== detail.task.project ||
      editTags.join(",") !== detail.task.tags.join(",")
    );
  }

  let confirmDiscardAction = $state<"cancel_edit" | "close_drawer" | null>(
    null,
  );
  let confirmDiscardOpen = $state<boolean>(false);

  function cancelEditingWithConfirm() {
    if (isEditDirty()) {
      confirmDiscardAction = "cancel_edit";
      confirmDiscardOpen = true;
      return;
    }
    if (detail) syncEditFields(detail);
    editingTask = false;
  }

  function hasUnsavedChanges(): boolean {
    return (
      isEditDirty() ||
      commentBody.trim() !== "" ||
      pendingCommentAttachments.length > 0 ||
      newTaskAttachmentDesc.trim() !== ""
    );
  }

  function requestCloseDrawer() {
    if (hasUnsavedChanges()) {
      confirmDiscardAction = "close_drawer";
      confirmDiscardOpen = true;
      return;
    }
    onClose();
  }

  function handleConfirmDiscard() {
    const action = confirmDiscardAction;
    confirmDiscardAction = null;
    confirmDiscardOpen = false;
    if (action === "close_drawer") {
      onClose();
    } else {
      if (detail) syncEditFields(detail);
      editingTask = false;
    }
  }

  function focusNextEditField(current: HTMLElement) {
    if (!editFormElement) return;
    const focusables = Array.from(
      editFormElement.querySelectorAll<HTMLElement>(
        'input:not([disabled]):not([type="hidden"]), select:not([disabled]), textarea:not([disabled]), [role="slider"]:not([disabled]), button:not([disabled])',
      ),
    ).filter((el) => el.tabIndex >= 0 && el.offsetParent !== null);
    const idx = focusables.indexOf(current);
    if (idx >= 0 && idx + 1 < focusables.length) {
      focusables[idx + 1].focus();
    }
  }

  function handleEditFormKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      e.stopPropagation();
      void saveEdits();
      return;
    }
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
        focusNextEditField(target);
      }
    }
  }

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

  async function saveEdits() {
    if (!detail) return;
    errorMsg = null;

    if (!editTitle.trim()) {
      errorMsg = "Task title cannot be empty.";
      announce(errorMsg);
      return;
    }

    if (!validateEditDatesNow()) {
      errorMsg = editSgdbError ?? editDueError;
      if (errorMsg) announce(errorMsg);
      return;
    }

    if (!validateEditDurationNow(true)) {
      errorMsg = editDurationError;
      if (errorMsg) announce(errorMsg);
      return;
    }

    for (const [label, val] of [
      ["Assignee", editAssignee],
      ["Evaluator", editEvaluator],
      ["Reporter", editReporter],
    ] as const) {
      const check = validateUserRef(val);
      if (check.error) {
        errorMsg = `${label}: ${check.error}`;
        announce(errorMsg);
        return;
      }
    }

    const parsedDur = parseDurationInput(editEstimateInput);

    try {
      const saved = await updateExistingTask({
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
        tags: editTags,
        project: editProject,
        assignee: editAssignee,
        evaluator: editEvaluator,
        reporter: editReporter,
      });
      editingTask = false;
      setUserNoticeAlerts(saved.user_notices);
      announce(
        withUserNotices(
          `Saved changes to task ${detail.task.id}`,
          saved.user_notices,
        ),
      );
      await refreshDetail(detail.task.id);
      onDataChanged();
    } catch (err) {
      errorMsg = String(err);
      announce(`Error saving task: ${String(err)}`);
    }
  }

  async function handleSaveEdits(e: SubmitEvent) {
    e.preventDefault();
    await saveEdits();
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

  async function handleTaskDropPaths(paths: string[]) {
    if (!detail) return;
    for (const path of paths) {
      try {
        const fileName = basenameOfPath(path);
        await attachFileFromDisk({
          taskId: detail.task.id,
          filePath: path,
          description: newTaskAttachmentDesc.trim() || null,
        });
        announce(`Attached ${fileName} to ${detail.task.id}`);
      } catch (err) {
        errorMsg = String(err);
        announce(`Failed to attach file: ${String(err)}`);
      }
    }
    newTaskAttachmentDesc = "";
    await refreshDetail(detail.task.id);
    onDataChanged();
  }

  async function handleTaskPasteFiles(files: FileList | File[]) {
    if (!detail) return;
    for (const file of Array.from(files)) {
      try {
        const b64 = await fileToBase64(file);
        await attachClipboardBytes({
          taskId: detail.task.id,
          base64Data: b64,
          suggestedName: file.name || "clipboard.png",
          description: newTaskAttachmentDesc.trim() || null,
        });
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

  $effect(() =>
    subscribeDropzone(() => taskDropzoneEl, {
      onDragChange: (dragging) => {
        isDraggingTaskFile = dragging;
      },
      onDrop: (paths) => {
        void handleTaskDropPaths(paths);
      },
    }),
  );

  async function handleAddRelationFromPicker(entry: SelectedRelationEntry) {
    if (!detail) return;
    try {
      await addRelation(detail.task.id, entry.relation, entry.target_id);
      announce(
        `Linked ${detail.task.id} (${RELATION_LABELS[entry.relation]}) to ${entry.target_id}`,
      );
      await refreshDetail(detail.task.id);
      onDataChanged();
    } catch (err) {
      errorMsg = String(err);
      announce(`Relation error: ${String(err)}`);
    }
  }

  async function handleChangeRelation(
    oldKind: RelationInputKind,
    newKind: RelationInputKind,
    targetId: string,
  ) {
    if (!detail || oldKind === newKind) return;
    try {
      await updateRelation(detail.task.id, oldKind, newKind, targetId);
      announce(
        `Changed ${targetId} relation from ${RELATION_LABELS[oldKind]} to ${RELATION_LABELS[newKind]}`,
      );
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
      const fileName = basenameOfPath(path);
      pendingCommentAttachments = [
        ...pendingCommentAttachments,
        {
          id: `${Date.now()}-${Math.random()}`,
          path,
          suggestedName: fileName,
          description: pendingCommentDesc.trim() || null,
        },
      ];
      pendingCommentDesc = "";
      announce(`Added file ${fileName} to pending comment`);
    } catch (err) {
      errorMsg = String(err);
    }
  }

  async function handleCommentPasteFiles(files: FileList) {
    const items = files;
    if (items.length === 0) return;
    for (const file of Array.from(items)) {
      try {
        const b64 = await fileToBase64(file);
        const friendlyName =
          file.name ||
          `comment-paste-${new Date().toISOString().slice(11, 19).replace(/:/g, "")}.png`;
        pendingCommentAttachments = [
          ...pendingCommentAttachments,
          {
            id: `${Date.now()}-${Math.random()}`,
            base64Data: b64,
            suggestedName: friendlyName,
            description: pendingCommentDesc.trim() || friendlyName,
            previewUrl: file.type.startsWith("image/") ? b64 : undefined,
          },
        ];
        announce(`Pasted image staged for comment`);
      } catch (err) {
        errorMsg = String(err);
      }
    }
  }

  async function postComment() {
    if (!detail || !commentBody.trim()) return;
    errorMsg = null;
    try {
      const attachmentsToSubmit: AttachmentInput[] = [];
      for (const item of pendingCommentAttachments) {
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

      const outcome = await addCommentNote({
        taskId: detail.task.id,
        author: activeUser,
        body: commentBody.trim(),
        spawnSubtasks: commentSpawnSubtasks,
        attachments: attachmentsToSubmit,
      });
      commentBody = "";
      pendingCommentAttachments = [];
      pendingCommentDesc = "";

      const spawnedCount = outcome.spawned_tasks.length;
      const reusedCount = outcome.reused_tasks.length;
      setUserNoticeAlerts(outcome.user_notices);
      announce(
        withUserNotices(
          `Posted note on ${detail.task.id}${
            spawnedCount > 0 ? ` (${spawnedCount} subtask(s) spawned)` : ""
          }${reusedCount > 0 ? ` (${reusedCount} open subtask(s) updated)` : ""}`,
          outcome.user_notices,
        ),
      );
      await refreshDetail(detail.task.id);
      onDataChanged();
    } catch (err) {
      errorMsg = String(err);
      announce(`Failed to post comment: ${String(err)}`);
    }
  }

  async function handlePostComment(e: SubmitEvent) {
    e.preventDefault();
    await postComment();
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

<Dialog.Root
  open={true}
  onOpenChange={(nextOpen) => {
    if (!nextOpen) requestCloseDrawer();
  }}
>
  <Dialog.Portal>
    <Dialog.Overlay class="detail-modal-overlay" />
    <Dialog.Content
      bind:ref={drawerElement}
      class="detail-modal"
      aria-labelledby="drawer-task-title"
      onInteractOutside={(e) => {
        // Clicking the backdrop must not discard work; Esc and Close are the exits.
        e.preventDefault();
      }}
      onEscapeKeydown={(e) => {
        const target = e.target as HTMLElement | null;
        if (
          target?.closest(
            "[data-combobox-input], [data-date-field-input], [data-time-field-input], [data-select-trigger], [data-select-content]",
          )
        ) {
          e.preventDefault();
        }
      }}
    >
      {#if loading && !detail}
        <p>Loading task {taskId}...</p>
      {:else if detail}
        <header class="detail-modal-header">
          <div class="detail-modal-heading">
            <div class="meta-row">
              <span class="badge">{STATUS_LABELS[detail.task.status]}</span>
              {#if detail.task.urgent_override}
                <span class="badge badge-urgent">Urgent Override</span>
              {/if}
            </div>
            <h2 id="drawer-task-title" class="detail-modal-title">
              <span class="task-id">{detail.task.id}</span>{editingTask
                ? " · Editing"
                : `: ${detail.task.title}`}
            </h2>
          </div>

          <div class="actions-row">
            {#if editingTask}
              <button
                type="button"
                class="btn-primary btn-sm"
                title="Save Changes (Cmd/Ctrl+Enter)"
                onclick={() => void saveEdits()}
              >
                Save Changes
              </button>
              <button
                type="button"
                class="btn-sm"
                onclick={cancelEditingWithConfirm}
              >
                Cancel
              </button>
            {:else}
              <button
                type="button"
                class="btn-sm"
                onclick={() => {
                  editingTask = true;
                }}
              >
                Edit Fields
              </button>
            {/if}
            <button
              type="button"
              class="btn-sm btn-danger"
              onclick={handleDeleteTask}
            >
              Delete Task
            </button>
            <button
              type="button"
              onclick={requestCloseDrawer}
              aria-label="Close task detail"
            >
              Close (Esc)
            </button>
          </div>
        </header>

        {#each userNoticeAlerts as alert (alert)}
          <div class="banner-alert" role="alert">
            <span>{alert}</span>
            <button
              type="button"
              class="btn-sm"
              onclick={() =>
                (userNoticeAlerts = userNoticeAlerts.filter(
                  (a) => a !== alert,
                ))}
            >
              Dismiss
            </button>
          </div>
        {/each}

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

        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div
          class="detail-modal-body"
          style="--detail-side-width: {sideWidthRem}rem"
          bind:this={editFormElement}
          onkeydown={handleEditFormKeydown}
        >
          <aside class="detail-modal-side" aria-label="Task details">
            <!-- Accessible Status Action Bar -->
            <section class="side-block" aria-label="Task Status Transition">
              <h3 class="side-heading">Status</h3>
              <div class="side-stack">
                <Select
                  id="drawer-status-select"
                  size="sm"
                  ariaLabel="Transition status"
                  value={detail.task.status}
                  options={STATUS_ORDER.map((st) => ({
                    value: st,
                    label: STATUS_LABELS[st],
                  }))}
                  onValueChange={(v) => handleStatusChange(v as TaskStatus)}
                />
              </div>
            </section>

            <section class="side-block" aria-labelledby="drawer-people-heading">
              <h3 id="drawer-people-heading" class="side-heading">People</h3>
              {#if editingTask}
                <div class="side-field">
                  <label for="edit-reporter">Reporter</label>
                  <input
                    id="edit-reporter"
                    type="text"
                    bind:value={editReporter}
                  />
                </div>
                <div class="side-field">
                  <label for="edit-assignee">Assignee</label>
                  <input
                    id="edit-assignee"
                    type="text"
                    bind:value={editAssignee}
                  />
                </div>
                <div class="side-field">
                  <label for="edit-evaluator">Evaluator</label>
                  <input
                    id="edit-evaluator"
                    type="text"
                    bind:value={editEvaluator}
                  />
                </div>
              {:else}
                <dl class="side-list">
                  <dt>Reporter</dt>
                  <dd>
                    <UserPill
                      user={detail.task.reporter}
                      kind={userKindOf(userKinds, detail.task.reporter)}
                      role="Reporter"
                    />
                  </dd>
                  <dt>Assignee</dt>
                  <dd>
                    <UserPill
                      user={detail.task.assignee}
                      kind={userKindOf(userKinds, detail.task.assignee)}
                      role="Assignee"
                    />
                  </dd>
                  <dt>Evaluator</dt>
                  <dd>
                    <UserPill
                      user={detail.task.evaluator}
                      kind={userKindOf(userKinds, detail.task.evaluator)}
                      role="Evaluator"
                    />
                  </dd>
                </dl>
              {/if}
            </section>

            <section class="side-block" aria-labelledby="drawer-timing-heading">
              <h3 id="drawer-timing-heading" class="side-heading">Timing</h3>
              {#if editingTask}
                <DateFieldPicker
                  id="edit-sgdb"
                  label="SGDB (soft)"
                  bind:value={editSgdbDate}
                  invalid={Boolean(editSgdbError)}
                  errorMessage={editSgdbError}
                  onBlur={() => {
                    validateEditDatesNow();
                  }}
                />
                <DateFieldPicker
                  id="edit-due"
                  label="Hard due"
                  bind:value={editDueDate}
                  minDate={editSgdbDate}
                  invalid={Boolean(editDueError)}
                  errorMessage={editDueError}
                  onBlur={() => {
                    validateEditDatesNow();
                  }}
                />
                <div class="side-field">
                  <div class="date-picker-label-row">
                    <label for="edit-est">Estimate</label>
                    {#if editDurationPreview && !editDurationError}
                      <span class="duration-preview-badge"
                        >{editDurationPreview}</span
                      >
                    {/if}
                  </div>
                  <input
                    id="edit-est"
                    type="text"
                    bind:value={editEstimateInput}
                    onblur={() => {
                      validateEditDurationNow(true);
                    }}
                    placeholder="e.g. 2h 30m"
                    aria-invalid={editDurationError ? "true" : "false"}
                    aria-describedby={editDurationError
                      ? "edit-dur-err"
                      : undefined}
                  />
                  {#if editDurationError}
                    <span id="edit-dur-err" class="field-error" role="alert">
                      {editDurationError}
                    </span>
                  {/if}
                </div>
              {:else}
                <dl class="side-list">
                  <dt>SGDB</dt>
                  <dd>
                    {detail.task.sgdb_date
                      ? formatDeadlineDisplay(detail.task.sgdb_date)
                      : "—"}
                  </dd>
                  <dt>Hard due</dt>
                  <dd>
                    {detail.task.due_date
                      ? formatDeadlineDisplay(detail.task.due_date)
                      : "—"}
                  </dd>
                  <dt>Estimate</dt>
                  <dd>
                    {formatDurationMinutes(detail.task.time_estimate_minutes)}
                  </dd>
                </dl>
              {/if}
            </section>

            <section
              class="side-block"
              aria-labelledby="drawer-metrics-heading"
            >
              <h3 id="drawer-metrics-heading" class="side-heading">Metrics</h3>
              {#if editingTask}
                <RatingSlider
                  id="edit-imp"
                  label="Importance (1–5)"
                  bind:value={editImportance}
                  min={1}
                  max={5}
                  step={1}
                />
                <RatingSlider
                  id="edit-diff"
                  label="Difficulty (1–5)"
                  bind:value={editDifficulty}
                  min={1}
                  max={5}
                  step={1}
                />
                <RatingSlider
                  id="edit-prio"
                  label="Priority (1–5)"
                  bind:value={editManualPriority}
                  min={1}
                  max={5}
                  step={1}
                />
              {:else}
                <dl class="side-list">
                  <dt>Importance</dt>
                  <dd>{detail.task.importance}/5</dd>
                  <dt>Difficulty</dt>
                  <dd>{detail.task.difficulty}/5</dd>
                  <dt>Priority</dt>
                  <dd>{detail.task.manual_priority}/5</dd>
                </dl>
              {/if}
            </section>

            <section class="side-block" aria-labelledby="drawer-tags-heading">
              <h3 id="drawer-tags-heading" class="side-heading">
                Tags & Project
              </h3>
              {#if editingTask}
                <div class="side-field">
                  <label for="edit-project">Project</label>
                  <input
                    id="edit-project"
                    type="text"
                    bind:value={editProject}
                    list="edit-project-datalist"
                    spellcheck="false"
                    pattern="[A-Za-z0-9_-]+"
                  />
                  <datalist id="edit-project-datalist">
                    {#each knownProjects as p}
                      <option value={p}></option>
                    {/each}
                  </datalist>
                </div>
                <TagChipsInput
                  id="edit-tags"
                  bind:tags={editTags}
                  {knownTags}
                />
                <div class="control-inline">
                  <input
                    id="edit-urgent"
                    type="checkbox"
                    bind:checked={editUrgentOverride}
                  />
                  <label for="edit-urgent">Urgent override</label>
                </div>
              {:else}
                <div class="side-stack">
                  <span class="project-badge" title="Project">
                    {detail.task.project}
                  </span>
                  {#if detail.task.tags.length > 0}
                    <div class="meta-row">
                      {#each detail.task.tags as tag}
                        <span class="badge badge-tag">#{tag}</span>
                      {/each}
                    </div>
                  {/if}
                  <p class="side-muted">Origin: {detail.task.origin_kind}</p>
                </div>
              {/if}
            </section>
          </aside>

          <!-- A focusable separator is the ARIA window-splitter pattern. -->
          <!-- svelte-ignore a11y_no_noninteractive_tabindex -->
          <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
          <div
            class="detail-modal-handle"
            class:is-dragging={sideHandleDragging}
            role="separator"
            aria-orientation="vertical"
            aria-label="Resize details sidebar"
            aria-valuemin={SIDE_WIDTH_MIN_REM}
            aria-valuemax={SIDE_WIDTH_MAX_REM}
            aria-valuenow={Math.round(sideWidthRem)}
            aria-keyshortcuts="ArrowLeft ArrowRight Home End"
            title="Drag or use arrow keys to resize. Double-click to reset."
            tabindex="0"
            onpointerdown={handleSideDragStart}
            onpointermove={handleSideDragMove}
            onpointerup={handleSideDragEnd}
            onpointercancel={handleSideDragEnd}
            ondblclick={resetSideWidth}
            onkeydown={handleSideHandleKeydown}
          ></div>

          <div class="detail-modal-main">
            {#if editingTask}
              <section class="detail-section" aria-label="Edit task">
                <div class="side-field">
                  <label for="edit-title">Title *</label>
                  <input
                    id="edit-title"
                    type="text"
                    bind:value={editTitle}
                    required
                  />
                </div>
                <h3 class="section-heading">Description</h3>
                <RichTextField
                  id="edit-desc"
                  rows={6}
                  bind:value={editDescription}
                  {knownUsers}
                  {userKinds}
                  mode="description"
                />
              </section>
            {:else}
              <section
                class="detail-section"
                aria-labelledby="drawer-description-heading"
              >
                <h3 id="drawer-description-heading" class="section-heading">
                  Description
                </h3>
                <RichTextDisplay
                  body={detail.task.description}
                  {knownUsers}
                  {userKinds}
                  mode="description"
                  {onSelectTask}
                />
              </section>
            {/if}

            <!-- Task-Level Attachments -->
            {#if editingTask || detail.task.attachments.length > 0}
              <section
                class="card"
                aria-labelledby="drawer-attachments-heading"
              >
                <h3
                  id="drawer-attachments-heading"
                  style="margin: 0 0 0.75rem 0; font-size: 1.02rem;"
                >
                  Task Attachments ({detail.task.attachments.length})
                </h3>

                <AttachmentGallery
                  attachments={detail.task.attachments}
                  editable={editingTask}
                  onChanged={() => {
                    if (detail) void refreshDetail(detail.task.id);
                    onDataChanged();
                  }}
                  {announce}
                />

                {#if editingTask}
                  <div
                    bind:this={taskDropzoneEl}
                    class={`dropzone ${isDraggingTaskFile ? "dragging" : ""}`}
                    style="margin-top: 0.75rem;"
                    role="region"
                    aria-label="Attach files to task by drag-and-drop, clipboard paste, or file picker"
                    onpaste={(e) => {
                      if (e.clipboardData?.files?.length) {
                        void handleTaskPasteFiles(e.clipboardData.files);
                      }
                    }}
                  >
                    <div
                      class="form-grid"
                      style="align-items: end; text-align: left;"
                    >
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
                          Or drag-and-drop / paste (Cmd/Ctrl+V) a file here
                        </span>
                      </div>
                    </div>
                  </div>
                {/if}
              </section>
            {/if}

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
                    <ul class="relation-group-list">
                      {#each detail.blockers as b}
                        <li class="relation-row">
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
                            <RelationTypeSelect
                              value="blocked_by"
                              label={`Relation type for ${b.id}`}
                              onChange={(kind) =>
                                handleChangeRelation("blocked_by", kind, b.id)}
                            />
                            <button
                              type="button"
                              class="btn-sm btn-danger"
                              aria-label={`Unlink blocked by relation to ${b.id}`}
                              onclick={() => handleUnlink("blocked_by", b.id)}
                            >
                              ×
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
                    <ul class="relation-group-list">
                      {#each detail.blocking as t}
                        <li class="relation-row">
                          <button
                            type="button"
                            class="btn-sm"
                            onclick={() => onSelectTask(t.id)}
                          >
                            {t.id}: {t.title}
                          </button>
                          <RelationTypeSelect
                            value="blocks"
                            label={`Relation type for ${t.id}`}
                            onChange={(kind) =>
                              handleChangeRelation("blocks", kind, t.id)}
                          />
                          <button
                            type="button"
                            class="btn-sm btn-danger"
                            aria-label={`Unlink blocks relation to ${t.id}`}
                            onclick={() => handleUnlink("blocks", t.id)}
                          >
                            ×
                          </button>
                        </li>
                      {/each}
                    </ul>
                  </div>
                {/if}

                {#if detail.parents.length > 0}
                  <div>
                    <strong>Sub-task Of ({detail.parents.length}):</strong>
                    <ul class="relation-group-list">
                      {#each detail.parents as t}
                        <li class="relation-row">
                          <button
                            type="button"
                            class="btn-sm"
                            onclick={() => onSelectTask(t.id)}
                          >
                            {t.id}: {t.title}
                          </button>
                          <RelationTypeSelect
                            value="sub_task_of"
                            label={`Relation type for ${t.id}`}
                            onChange={(kind) =>
                              handleChangeRelation("sub_task_of", kind, t.id)}
                          />
                          <button
                            type="button"
                            class="btn-sm btn-danger"
                            aria-label={`Unlink sub-task relation to ${t.id}`}
                            onclick={() => handleUnlink("sub_task_of", t.id)}
                          >
                            ×
                          </button>
                        </li>
                      {/each}
                    </ul>
                  </div>
                {/if}

                {#if detail.subtasks.length > 0}
                  <div>
                    <strong>Subtasks ({detail.subtasks.length}):</strong>
                    <ul class="relation-group-list">
                      {#each detail.subtasks as t}
                        <li class="relation-row">
                          <button
                            type="button"
                            class="btn-sm"
                            onclick={() => onSelectTask(t.id)}
                          >
                            {t.id}: {t.title} ({STATUS_LABELS[t.status]})
                          </button>
                          <RelationTypeSelect
                            value="parent_task_of"
                            label={`Relation type for subtask ${t.id}`}
                            onChange={(kind) =>
                              handleChangeRelation(
                                "parent_task_of",
                                kind,
                                t.id,
                              )}
                          />
                          <button
                            type="button"
                            class="btn-sm btn-danger"
                            aria-label={`Unlink subtask relation to ${t.id}`}
                            onclick={() => handleUnlink("parent_task_of", t.id)}
                          >
                            ×
                          </button>
                        </li>
                      {/each}
                    </ul>
                  </div>
                {/if}

                {#if detail.parallel.length > 0}
                  <div>
                    <strong>Parallel Tasks ({detail.parallel.length}):</strong>
                    <ul class="relation-group-list">
                      {#each detail.parallel as t}
                        <li class="relation-row">
                          <button
                            type="button"
                            class="btn-sm"
                            onclick={() => onSelectTask(t.id)}
                          >
                            {t.id}: {t.title}
                          </button>
                          <RelationTypeSelect
                            value="parallel"
                            label={`Relation type for ${t.id}`}
                            onChange={(kind) =>
                              handleChangeRelation("parallel", kind, t.id)}
                          />
                          <button
                            type="button"
                            class="btn-sm btn-danger"
                            aria-label={`Unlink parallel relation to ${t.id}`}
                            onclick={() => handleUnlink("parallel", t.id)}
                          >
                            ×
                          </button>
                        </li>
                      {/each}
                    </ul>
                  </div>
                {/if}

                {#if detail.related.length > 0}
                  <div>
                    <strong>Related Tasks ({detail.related.length}):</strong>
                    <ul class="relation-group-list">
                      {#each detail.related as t}
                        <li class="relation-row">
                          <button
                            type="button"
                            class="btn-sm"
                            onclick={() => onSelectTask(t.id)}
                          >
                            {t.id}: {t.title}
                          </button>
                          <RelationTypeSelect
                            value="related"
                            label={`Relation type for ${t.id}`}
                            onChange={(kind) =>
                              handleChangeRelation("related", kind, t.id)}
                          />
                          <button
                            type="button"
                            class="btn-sm btn-danger"
                            aria-label={`Unlink related relation to ${t.id}`}
                            onclick={() => handleUnlink("related", t.id)}
                          >
                            ×
                          </button>
                        </li>
                      {/each}
                    </ul>
                  </div>
                {/if}
              </div>

              <RelationPicker
                excludeTaskId={detail.task.id}
                buttonLabel="Link Task"
                onAdd={(entry) => void handleAddRelationFromPicker(entry)}
              />
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
                  No comments yet. Mention <code>@user</code> to spawn an urgent
                  blocking subtask, <code>@takenote:user</code> for a
                  non-blocking related note, or <code>#AB1234</code> to cross-link
                  another task.
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
                      <RichTextDisplay
                        body={note.body}
                        {knownUsers}
                        {userKinds}
                        mode="comment"
                        {onSelectTask}
                        class="comment-note-body"
                      />
                      {#if note.attachments.length > 0}
                        <AttachmentGallery
                          attachments={note.attachments}
                          editable={editingTask}
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

              <!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
              <form
                onsubmit={handlePostComment}
                onkeydown={(e) => {
                  if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                    e.preventDefault();
                    e.stopPropagation();
                    void postComment();
                  } else if (
                    e.key === "Enter" &&
                    (e.target as HTMLElement | null)?.tagName === "INPUT" &&
                    (e.target as HTMLInputElement).type === "text"
                  ) {
                    e.preventDefault();
                  }
                }}
                style="display: flex; flex-direction: column; gap: 0.65rem;"
              >
                <div class="form-field">
                  <label for="new-comment-body">
                    Add Comment (as @{activeUser})
                  </label>
                  <RichTextField
                    id="new-comment-body"
                    rows={3}
                    bind:value={commentBody}
                    {knownUsers}
                    {userKinds}
                    mode="comment"
                    onPasteFiles={handleCommentPasteFiles}
                    placeholder="Write a comment... Use @user for blocking subtasks, @takenote:user for non-blocking heads-up, or #AB1234 to link tasks. Paste images directly with Cmd/Ctrl+V."
                  />

                  {#if commentMentionTokens.length > 0}
                    <div
                      class="comment-mention-preview"
                      role="status"
                      aria-live="polite"
                      aria-label="Detected user mentions in comment"
                    >
                      <span class="comment-mention-preview-label"
                        >Mentions:</span
                      >
                      {#each commentMentionTokens as token (token.rawToken)}
                        <span
                          class={`comment-mention-pill mention-${token.kind} ${token.exists ? "mention-exists" : "mention-missing"}`}
                          data-tooltip={token.tooltip}
                          title={token.tooltip}
                        >
                          <strong>{token.rawToken}</strong>
                          {#if token.kind === "invalid"}
                            <span class="comment-mention-pill-meta">
                              Invalid username (ignored)
                            </span>
                          {:else if token.exists}
                            <span class="comment-mention-pill-meta">
                              ✓ Existing user · {token.kind === "takenote"
                                ? "Take-note"
                                : "Blocking subtask"}
                            </span>
                          {:else}
                            <span class="comment-mention-pill-meta">
                              + Does not exist yet (will create @{token.username})
                            </span>
                          {/if}
                        </span>
                      {/each}
                    </div>
                  {/if}
                </div>

                {#if pendingCommentAttachments.length > 0}
                  <ul class="draft-attachments-list">
                    {#each pendingCommentAttachments as att (att.id)}
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
                              <span class="field-hint">
                                — "{att.description}"</span
                              >
                            {/if}
                          </div>
                        </div>
                        <button
                          type="button"
                          class="btn-sm btn-danger"
                          onclick={() => {
                            pendingCommentAttachments =
                              pendingCommentAttachments.filter(
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

                <div class="comment-actions">
                  <div class="comment-attach-row">
                    <input
                      id="comment-att-desc"
                      type="text"
                      bind:value={pendingCommentDesc}
                      placeholder="Alt text for next comment attachment"
                      aria-label="Alt text description for comment attachment"
                    />
                    <button
                      type="button"
                      class="btn-sm"
                      onclick={handlePickCommentAttachment}
                    >
                      Attach File to Comment...
                    </button>
                    <label class="comment-spawn-toggle">
                      <input
                        type="checkbox"
                        bind:checked={commentSpawnSubtasks}
                      />
                      Auto-spawn @mention subtasks
                    </label>
                  </div>

                  <button
                    type="submit"
                    class="btn-primary"
                    title="Post Comment (Cmd/Ctrl+Enter)"
                  >
                    Post Comment
                  </button>
                </div>
              </form>
            </section>
          </div>
        </div>
      {/if}
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>

<ConfirmModal
  bind:open={confirmDiscardOpen}
  title="Discard Unsaved Changes?"
  description={confirmDiscardAction === "close_drawer"
    ? "You have unsaved changes to this task. Discard them and close?"
    : "You have unsaved changes to this task. Discard your edits?"}
  confirmLabel="Discard Changes"
  cancelLabel="Keep Editing"
  danger={true}
  onConfirm={handleConfirmDiscard}
  onCancel={() => {
    confirmDiscardAction = null;
    confirmDiscardOpen = false;
  }}
/>
