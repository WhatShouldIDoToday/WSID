import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";

export type TaskStatus =
  "backlog" | "todo" | "in_progress" | "in_review" | "stuck" | "completed";

export type OriginKind = "manual" | "mention" | "takenote" | "review";

export type FocusPreset =
  "balanced" | "quick_wins" | "eat_the_frog" | "unblock_others";

export type EisenhowerQuadrant =
  "q1_do_first" | "q2_schedule" | "q3_delegate_quick" | "q4_later";

export type RelationInputKind =
  | "blocks"
  | "blocked_by"
  | "parent_task_of"
  | "sub_task_of"
  | "parallel"
  | "related";

export interface TaskAttachment {
  id: number;
  task_id: string;
  note_id: number | null;
  file_path: string;
  file_name: string;
  media_type: string;
  description: string;
  is_managed: boolean;
  created_at: string;
}

export interface Task {
  id: string;
  title: string;
  description: string;
  status: TaskStatus;
  origin_kind: OriginKind;
  origin_note_id: number | null;
  due_date: string | null;
  sgdb_date: string | null;
  time_estimate_minutes: number | null;
  difficulty: number;
  importance: number;
  urgent_override: boolean;
  manual_priority: number;
  tags: string[];
  reporter: string;
  assignee: string;
  evaluator: string;
  created_at: string;
  updated_at: string;
  completed_at: string | null;
  attachments: TaskAttachment[];
}

export interface TaskNote {
  id: number;
  task_id: string;
  author: string;
  body: string;
  created_at: string;
  attachments: TaskAttachment[];
}

export interface TaskRelation {
  source_id: string;
  target_id: string;
  relation_type: string;
  created_at: string;
}

export interface ScoreBreakdown {
  total_score: number;
  urgency_component: number;
  importance_component: number;
  unblocks_component: number;
  effort_component: number;
  manual_priority_component: number;
  parallel_component: number;
  in_progress_component: number;
  effective_deadline: string | null;
  slack_days: number | null;
  unblocks_count: number;
  has_in_progress_parallel_peer: boolean;
}

export interface ScoredTask {
  task: Task;
  quadrant: EisenhowerQuadrant;
  breakdown: ScoreBreakdown;
}

export interface BlockedSummaryGroup {
  project_or_tag: string;
  blocker_kind: OriginKind;
  waiting_on_assignee: string;
  count: number;
  task_ids: string[];
}

export interface FocusQueueResponse {
  user: string;
  preset: FocusPreset;
  actionable_tasks: ScoredTask[];
  blocked_summary: BlockedSummaryGroup[];
}

export interface BlockerInfo {
  blocker_id: string;
  title: string;
  origin_kind: OriginKind;
  assignee: string;
  status: TaskStatus;
}

export interface BlockedTaskEntry {
  task_id: string;
  title: string;
  status: TaskStatus;
  tags: string[];
  blocked_by: BlockerInfo[];
}

export interface EisenhowerClassification {
  task: Task;
  effective_deadline: string | null;
  slack_days: number | null;
  is_important: boolean;
  is_urgent: boolean;
  quadrant: EisenhowerQuadrant;
}

export interface SearchHit {
  task: Task;
  score: number;
}

export interface AppSessionSnapshot {
  db_path: string;
  global_db_path: string;
  is_global_db: boolean;
  active_user: string;
  active_preset: FocusPreset;
  focus_limit: number;
  recent_workspaces: string[];
  known_users: string[];
  known_tags: string[];
}

export interface TaskFullDetail {
  task: Task;
  blockers: Task[];
  blocking: Task[];
  parents: Task[];
  subtasks: Task[];
  parallel: Task[];
  related: Task[];
  notes: TaskNote[];
}

export interface AttachmentInput {
  path: string;
  description?: string | null;
}

export interface GuiInitialRelation {
  relation: RelationInputKind;
  target_id: string;
}

export interface GuiCreateTaskPayload {
  title: string;
  description?: string | null;
  status?: TaskStatus | null;
  due_date?: string | null;
  sgdb_date?: string | null;
  time_estimate_minutes?: number | null;
  difficulty?: number | null;
  importance?: number | null;
  urgent_override?: boolean | null;
  manual_priority?: number | null;
  tags: string[];
  reporter?: string | null;
  assignee?: string | null;
  evaluator?: string | null;
  attachments: AttachmentInput[];
  initial_relations: GuiInitialRelation[];
}

export interface GuiUpdateTaskPayload {
  id: string;
  title?: string;
  description?: string;
  status?: TaskStatus;
  due_date?: string | null;
  sgdb_date?: string | null;
  time_estimate_minutes?: number | null;
  difficulty?: number;
  importance?: number;
  urgent_override?: boolean;
  manual_priority?: number;
  tags?: string[];
  reporter?: string;
  assignee?: string;
  evaluator?: string;
}

export interface StatusTransitionOutcome {
  task: Task;
  spawned_review_task: Task | null;
  auto_completed_parent_tasks: Task[];
}

export interface AddTaskNoteOutcome {
  note: TaskNote;
  spawned_tasks: Task[];
  reused_tasks: Task[];
  completed_review_tasks: Task[];
}

export const STATUS_LABELS: Record<TaskStatus, string> = {
  backlog: "Backlog",
  todo: "To Do",
  in_progress: "In Progress",
  in_review: "In Review",
  stuck: "Stuck",
  completed: "Completed",
};

export const STATUS_ORDER: TaskStatus[] = [
  "backlog",
  "todo",
  "in_progress",
  "in_review",
  "stuck",
  "completed",
];

export const PRESET_LABELS: Record<FocusPreset, string> = {
  balanced: "Balanced",
  quick_wins: "Quick Wins",
  eat_the_frog: "Eat the Frog",
  unblock_others: "Unblock Others",
};

export const RELATION_LABELS: Record<RelationInputKind, string> = {
  related: "Related to",
  blocks: "Blocks",
  blocked_by: "Is blocked by",
  sub_task_of: "Sub-task of",
  parent_task_of: "Parent task of",
  parallel: "Parallel with",
};

export const QUADRANT_META: Record<
  EisenhowerQuadrant,
  { title: string; subtitle: string; badge: string }
> = {
  q1_do_first: {
    title: "Quadrant 1: Urgent & Important (Do First)",
    subtitle: "Critical deadlines, high-impact blockers, and urgent overrides",
    badge: "Q1 · Do First",
  },
  q2_schedule: {
    title: "Quadrant 2: Not Urgent & Important (Schedule)",
    subtitle: "Strategic deep work with comfortable slack or no deadline yet",
    badge: "Q2 · Schedule",
  },
  q3_delegate_quick: {
    title: "Quadrant 3: Urgent & Not Important (Quick / Delegate)",
    subtitle: "Time-sensitive pings, quick follow-ups, and lower-impact items",
    badge: "Q3 · Quick / Delegate",
  },
  q4_later: {
    title: "Quadrant 4: Not Urgent & Not Important (Later)",
    subtitle: "Low-priority backlog ideas and nice-to-haves",
    badge: "Q4 · Later",
  },
};

export function isImageMediaType(mediaType: string): boolean {
  return mediaType.toLowerCase().startsWith("image/");
}

/**
 * Parses human-friendly duration strings such as `30m`, `2h`, `1h30m`, `3d`, `1w`
 * into minutes (assuming 8h = 480m workdays and 5d = 2400m workweeks).
 */
export function parseDurationInput(raw: string): {
  minutes: number | null;
  error: string | null;
} {
  const trimmed = raw.trim().toLowerCase();
  if (!trimmed) {
    return { minutes: null, error: null };
  }
  if (/^\d+$/.test(trimmed)) {
    return { minutes: parseInt(trimmed, 10), error: null };
  }

  const tokenRegex = /(\d+)\s*([wdhm])/g;
  let match: RegExpExecArray | null;
  let total = 0;
  let consumed = "";

  while ((match = tokenRegex.exec(trimmed)) !== null) {
    const val = parseInt(match[1], 10);
    const unit = match[2];
    consumed += match[0];
    if (unit === "w") total += val * 5 * 480;
    else if (unit === "d") total += val * 480;
    else if (unit === "h") total += val * 60;
    else if (unit === "m") total += val;
  }

  const normalizedInput = trimmed.replace(/\s+/g, "");
  const normalizedConsumed = consumed.replace(/\s+/g, "");
  if (!normalizedConsumed || normalizedInput !== normalizedConsumed) {
    return {
      minutes: null,
      error:
        "Use formats like 30m, 2h, 1h30m, 3d, or 1w (or plain minutes like 45).",
    };
  }

  return { minutes: total, error: null };
}

export function formatDurationMinutes(minutes: number | null): string {
  if (minutes === null || minutes === undefined) return "—";
  if (minutes <= 0) return "0m";
  const hours = Math.floor(minutes / 60);
  const mins = minutes % 60;
  if (hours > 0 && mins > 0) return `${hours}h ${mins}m`;
  if (hours > 0) return `${hours}h`;
  return `${mins}m`;
}

export function parseDeadlineToEpochMs(
  raw: string | null | undefined,
): number | null {
  if (!raw) return null;
  const s = raw.trim();
  if (!s) return null;
  if (/^\d{4}-\d{2}-\d{2}$/.test(s)) {
    const dt = new Date(`${s}T20:00:00`);
    return Number.isNaN(dt.getTime()) ? null : dt.getTime();
  }
  const dt = new Date(s);
  return Number.isNaN(dt.getTime()) ? null : dt.getTime();
}

export function validateDeadlineOrder(
  sgdbDate: string | null | undefined,
  dueDate: string | null | undefined,
): string | null {
  const sgdbMs = parseDeadlineToEpochMs(sgdbDate);
  const dueMs = parseDeadlineToEpochMs(dueDate);
  if (sgdbMs !== null && dueMs !== null && sgdbMs > dueMs) {
    return "Should Get Done By (SGDB) date/time must be on or before the Hard Due Date.";
  }
  return null;
}

export function formatDeadlineDisplay(raw: string | null | undefined): string {
  if (!raw) return "—";
  const s = raw.trim();
  if (!s) return "—";
  if (/^\d{4}-\d{2}-\d{2}$/.test(s)) {
    return s;
  }
  const dt = new Date(s);
  if (Number.isNaN(dt.getTime())) {
    return s;
  }
  const yyyy = dt.getFullYear();
  const mm = String(dt.getMonth() + 1).padStart(2, "0");
  const dd = String(dt.getDate()).padStart(2, "0");
  const hh = String(dt.getHours()).padStart(2, "0");
  const min = String(dt.getMinutes()).padStart(2, "0");
  return `${yyyy}-${mm}-${dd} ${hh}:${min}`;
}

export async function getSessionState(): Promise<AppSessionSnapshot> {
  return invoke<AppSessionSnapshot>("get_session_state");
}

export async function setActiveUser(user: string): Promise<AppSessionSnapshot> {
  return invoke<AppSessionSnapshot>("set_active_user", { user });
}

export async function setFocusPreset(
  preset: FocusPreset,
): Promise<AppSessionSnapshot> {
  return invoke<AppSessionSnapshot>("set_focus_preset", { preset });
}

export async function setFocusLimit(
  limit: number,
): Promise<AppSessionSnapshot> {
  return invoke<AppSessionSnapshot>("set_focus_limit", { limit });
}

export async function switchDatabase(
  path: string | null,
  useGlobal: boolean,
): Promise<AppSessionSnapshot> {
  return invoke<AppSessionSnapshot>("switch_database", {
    path,
    useGlobal,
  });
}

export async function pickWorkspaceDatabaseDialog(): Promise<string | null> {
  return invoke<string | null>("pick_workspace_database_dialog");
}

export async function pickAttachmentFileDialog(): Promise<string | null> {
  return invoke<string | null>("pick_attachment_file_dialog");
}

export async function getFocusViewData(
  user?: string,
  preset?: FocusPreset,
  limit?: number,
): Promise<FocusQueueResponse> {
  return invoke<FocusQueueResponse>("get_focus_view_data", {
    user: user ?? null,
    preset: preset ?? null,
    limit: limit ?? null,
  });
}

export async function getBlockedTasksData(
  user?: string,
  tag?: string,
): Promise<BlockedTaskEntry[]> {
  return invoke<BlockedTaskEntry[]>("get_blocked_tasks_data", {
    user: user ?? null,
    tag: tag ?? null,
  });
}

export async function getAllTasksData(): Promise<Task[]> {
  return invoke<Task[]>("get_all_tasks_data");
}

export async function getEisenhowerData(
  userFilter?: string | null,
): Promise<EisenhowerClassification[]> {
  return invoke<EisenhowerClassification[]>("get_eisenhower_data", {
    userFilter: userFilter ?? null,
  });
}

export async function searchTasksData(params: {
  query: string;
  status?: TaskStatus | null;
  tag?: string | null;
  assignee?: string | null;
  reporter?: string | null;
  evaluator?: string | null;
  limit?: number | null;
}): Promise<SearchHit[]> {
  return invoke<SearchHit[]>("search_tasks_data", {
    query: params.query,
    status: params.status ?? null,
    tag: params.tag ?? null,
    assignee: params.assignee ?? null,
    reporter: params.reporter ?? null,
    evaluator: params.evaluator ?? null,
    limit: params.limit ?? null,
  });
}

export async function getTaskDetailData(id: string): Promise<TaskFullDetail> {
  return invoke<TaskFullDetail>("get_task_detail_data", { id });
}

export async function createNewTask(
  payload: GuiCreateTaskPayload,
): Promise<Task> {
  return invoke<Task>("create_new_task", { payload });
}

export async function updateExistingTask(
  payload: GuiUpdateTaskPayload,
): Promise<Task> {
  return invoke<Task>("update_existing_task", { payload });
}

export async function transitionStatus(
  id: string,
  status: TaskStatus,
): Promise<StatusTransitionOutcome> {
  return invoke<StatusTransitionOutcome>("transition_status", { id, status });
}

export async function removeTask(id: string): Promise<boolean> {
  return invoke<boolean>("remove_task", { id });
}

export async function addCommentNote(params: {
  taskId: string;
  author?: string | null;
  body: string;
  spawnSubtasks: boolean;
  attachments: AttachmentInput[];
}): Promise<AddTaskNoteOutcome> {
  return invoke<AddTaskNoteOutcome>("add_comment_note", {
    taskId: params.taskId,
    author: params.author ?? null,
    body: params.body,
    spawnSubtasks: params.spawnSubtasks,
    attachments: params.attachments,
  });
}

export async function removeCommentNote(noteId: number): Promise<boolean> {
  return invoke<boolean>("remove_comment_note", { noteId });
}

export async function addRelation(
  sourceId: string,
  relation: RelationInputKind,
  targetId: string,
): Promise<TaskRelation> {
  return invoke<TaskRelation>("add_relation", {
    sourceId,
    relation,
    targetId,
  });
}

export async function removeRelation(
  sourceId: string,
  relation: RelationInputKind,
  targetId: string,
): Promise<boolean> {
  return invoke<boolean>("remove_relation", {
    sourceId,
    relation,
    targetId,
  });
}

export async function dismissSubtaskBlocker(
  subtaskId: string,
  dismiss: boolean,
): Promise<Task> {
  return invoke<Task>("dismiss_subtask_blocker", { subtaskId, dismiss });
}

export async function attachFileFromDisk(params: {
  taskId: string;
  noteId?: number | null;
  filePath: string;
  description?: string | null;
}): Promise<TaskAttachment> {
  return invoke<TaskAttachment>("attach_file_from_disk", {
    taskId: params.taskId,
    noteId: params.noteId ?? null,
    filePath: params.filePath,
    description: params.description ?? null,
  });
}

export async function attachClipboardBytes(params: {
  taskId: string;
  noteId?: number | null;
  base64Data: string;
  suggestedName?: string | null;
  description?: string | null;
}): Promise<TaskAttachment> {
  return invoke<TaskAttachment>("attach_clipboard_bytes", {
    taskId: params.taskId,
    noteId: params.noteId ?? null,
    base64Data: params.base64Data,
    suggestedName: params.suggestedName ?? null,
    description: params.description ?? null,
  });
}

export async function saveClipboardBytesForDraft(
  base64Data: string,
  suggestedName?: string | null,
): Promise<string> {
  return invoke<string>("save_clipboard_bytes_for_draft", {
    base64Data,
    suggestedName: suggestedName ?? null,
  });
}

export async function editAttachmentAltDescription(
  attachmentId: number,
  description: string,
): Promise<TaskAttachment> {
  return invoke<TaskAttachment>("edit_attachment_alt_description", {
    attachmentId,
    description,
  });
}

export async function deleteAttachment(attachmentId: number): Promise<boolean> {
  return invoke<boolean>("delete_attachment", { attachmentId });
}

export async function openAttachmentNative(params: {
  attachmentId?: number | null;
  filePath?: string | null;
}): Promise<void> {
  return invoke<void>("open_attachment_native", {
    attachmentId: params.attachmentId ?? null,
    filePath: params.filePath ?? null,
  });
}

export async function readAttachmentPreviewDataUrl(
  attachmentId: number,
): Promise<string> {
  return invoke<string>("read_attachment_preview_data_url", { attachmentId });
}

export interface LicenseOverviewCount {
  count: number;
  id: string;
  name: string;
}

export interface CrateAttribution {
  name: string;
  version: string;
  authors: string[];
  repository: string | null;
}

export interface LicenseGroup {
  id: string;
  name: string;
  text: string;
  used_by: CrateAttribution[];
}

export interface RustLicenseBundle {
  overview: LicenseOverviewCount[];
  licenses: LicenseGroup[];
}

export interface AboutMetadata {
  app_name: string;
  app_full_name: string;
  version: string;
  license_spdx: string;
  repository_url: string;
  license_text: string;
  rust_licenses: RustLicenseBundle;
}

export interface FrontendDependencyLicense {
  name: string;
  version: string | null;
  license: string | null;
  repository: string | null;
  author: string | null;
  licenseText: string | null;
  noticeText: string | null;
}

export async function getAboutInfo(): Promise<AboutMetadata> {
  return invoke<AboutMetadata>("get_about_info");
}

export async function subscribeToExternalDbChanges(
  onChanged: () => void,
): Promise<UnlistenFn> {
  return listen("wsid://db-changed", () => {
    onChanged();
  });
}

export async function subscribeToMenuActions(
  onAction: (action: string) => void,
): Promise<UnlistenFn> {
  return listen<string>("wsid://menu-action", (event) => {
    if (typeof event.payload === "string") {
      onAction(event.payload);
    }
  });
}

export type UiThemeMode = "system" | "dark" | "light";
export type UiContrastMode = "system" | "more";
export type UiColorPalette = "default" | "okabe-ito";

export interface UiPreferences {
  theme: UiThemeMode;
  contrast: UiContrastMode;
  palette: UiColorPalette;
  fontScalePercent: number;
}

const UI_PREFS_STORAGE_KEY = "wsid.ui.preferences.v1";

export const DEFAULT_UI_PREFERENCES: UiPreferences = {
  theme: "system",
  contrast: "system",
  palette: "default",
  fontScalePercent: 100,
};

export function loadUiPreferences(): UiPreferences {
  if (typeof window === "undefined") {
    return { ...DEFAULT_UI_PREFERENCES };
  }
  try {
    const raw = window.localStorage.getItem(UI_PREFS_STORAGE_KEY);
    if (!raw) return { ...DEFAULT_UI_PREFERENCES };
    const parsed = JSON.parse(raw) as Partial<UiPreferences>;
    return {
      theme:
        parsed.theme === "dark" || parsed.theme === "light"
          ? parsed.theme
          : "system",
      contrast: parsed.contrast === "more" ? "more" : "system",
      palette: parsed.palette === "okabe-ito" ? "okabe-ito" : "default",
      fontScalePercent:
        typeof parsed.fontScalePercent === "number" &&
        parsed.fontScalePercent >= 85 &&
        parsed.fontScalePercent <= 150
          ? parsed.fontScalePercent
          : 100,
    };
  } catch {
    return { ...DEFAULT_UI_PREFERENCES };
  }
}

export function saveUiPreferences(prefs: UiPreferences): void {
  if (typeof window !== "undefined") {
    try {
      window.localStorage.setItem(UI_PREFS_STORAGE_KEY, JSON.stringify(prefs));
    } catch {
      // Ignore storage write errors
    }
  }
  applyUiPreferencesToDom(prefs);
}

export function applyUiPreferencesToDom(prefs: UiPreferences): void {
  if (typeof document === "undefined") return;
  const root = document.documentElement;
  if (prefs.theme === "system") {
    root.removeAttribute("data-theme");
  } else {
    root.setAttribute("data-theme", prefs.theme);
  }
  if (prefs.contrast === "more") {
    root.setAttribute("data-contrast", "more");
  } else {
    root.removeAttribute("data-contrast");
  }
  if (prefs.palette === "okabe-ito") {
    root.setAttribute("data-palette", "okabe-ito");
  } else {
    root.removeAttribute("data-palette");
  }
  const scale = Math.max(85, Math.min(150, prefs.fontScalePercent)) / 100;
  root.style.setProperty("--ui-font-scale", String(scale));
}

export function fileToBase64(file: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      if (typeof reader.result === "string") {
        resolve(reader.result);
      } else {
        reject(new Error("Failed to read file as base64 string"));
      }
    };
    reader.onerror = () =>
      reject(reader.error ?? new Error("FileReader error"));
    reader.readAsDataURL(file);
  });
}
