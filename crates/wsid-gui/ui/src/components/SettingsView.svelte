<script lang="ts">
  import Select from "./ui/Select.svelte";
  import UserPill from "./UserPill.svelte";
  import { Dialog } from "bits-ui";
  import { knownUserKinds, setUserKind, userKindOf } from "../lib/ipc";
  import {
    type AppSessionSnapshot,
    type FocusPreset,
    type UiPreferences,
    type UserDeletionPreview,
    PRESET_LABELS,
    STATUS_LABELS,
    previewDeleteUser,
    deleteUser,
  } from "../lib/ipc";

  interface Props {
    session: AppSessionSnapshot;
    uiPrefs: UiPreferences;
    onUpdateUiPrefs: (prefs: UiPreferences) => void;
    onOpenUserModal: () => void;
    onOpenProjectModal: () => void;
    onPickWorkspaceDb: () => void | Promise<void>;
    onSwitchGlobalDb: () => void | Promise<void>;
    onSwitchRecentDb: (path: string) => void | Promise<void>;
    onPresetChange: (preset: FocusPreset) => void | Promise<void>;
    onFocusLimitChange: (limit: number) => void | Promise<void>;
    onOpenAbout: () => void;
    onUserDeleted: () => void | Promise<void>;
    onUserKindChanged: () => void | Promise<void>;
    onSelectTask?: (id: string) => void;
    announce: (msg: string) => void;
  }

  let {
    session,
    uiPrefs,
    onUpdateUiPrefs,
    onOpenUserModal,
    onOpenProjectModal,
    onPickWorkspaceDb,
    onSwitchGlobalDb,
    onSwitchRecentDb,
    onPresetChange,
    onFocusLimitChange,
    onOpenAbout,
    onUserDeleted,
    onUserKindChanged,
    onSelectTask,
    announce,
  }: Props = $props();

  /** Unsaved kind edits per user; a user without a draft shows their saved kind. */
  let kindDrafts = $state<Record<string, string>>({});
  let kindOptions = $derived(knownUserKinds(session.user_kinds));

  async function saveUserKind(username: string) {
    const kind = (
      kindDrafts[username] ?? userKindOf(session.user_kinds, username)
    ).trim();
    userMgmtError = null;
    try {
      await setUserKind(username, kind);
      delete kindDrafts[username];
      announce(`Set @${username} kind to ${kind.toLowerCase()}.`);
      await onUserKindChanged();
    } catch (err) {
      userMgmtError = String(err);
      announce(`Failed to set kind for @${username}: ${String(err)}`);
    }
  }

  let isMainUser = $derived(
    session.active_user.toLowerCase() === session.primary_user.toLowerCase(),
  );

  let deletePreview = $state<UserDeletionPreview | null>(null);
  let deleteAuthoredTasks = $state<boolean>(false);
  let deleteBusy = $state<boolean>(false);
  let userMgmtError = $state<string | null>(null);

  async function openDeleteUserDialog(username: string) {
    userMgmtError = null;
    deleteAuthoredTasks = false;
    try {
      deletePreview = await previewDeleteUser(username);
    } catch (err) {
      userMgmtError = String(err);
      announce(`Failed to preview user deletion: ${String(err)}`);
    }
  }

  function closeDeleteUserDialog() {
    if (deleteBusy) return;
    deletePreview = null;
    deleteAuthoredTasks = false;
  }

  async function confirmDeleteUser() {
    if (!deletePreview || deleteBusy) return;
    deleteBusy = true;
    userMgmtError = null;
    try {
      const outcome = await deleteUser(
        deletePreview.username,
        deleteAuthoredTasks,
      );
      deletePreview = null;
      deleteAuthoredTasks = false;
      announce(
        `Deleted user @${outcome.deleted_user}: reassigned ${outcome.reassigned_task_count} task(s) to @${outcome.fallback_user}${
          outcome.deleted_task_count > 0
            ? ` and permanently deleted ${outcome.deleted_task_count} authored task(s)`
            : ""
        }.`,
      );
      await onUserDeleted();
    } catch (err) {
      userMgmtError = String(err);
      announce(`Failed to delete user: ${String(err)}`);
    } finally {
      deleteBusy = false;
    }
  }

  function setTheme(theme: UiPreferences["theme"]) {
    onUpdateUiPrefs({ ...uiPrefs, theme });
  }

  function setContrast(contrast: UiPreferences["contrast"]) {
    onUpdateUiPrefs({ ...uiPrefs, contrast });
  }

  function setPalette(palette: UiPreferences["palette"]) {
    onUpdateUiPrefs({ ...uiPrefs, palette });
  }

  function setFontScale(fontScalePercent: number) {
    onUpdateUiPrefs({
      ...uiPrefs,
      fontScalePercent: Math.max(85, Math.min(150, fontScalePercent)),
    });
  }
</script>

<section aria-labelledby="settings-view-heading" class="settings-container">
  <h2 id="settings-view-heading" style="margin: 0 0 1.25rem 0;">Settings</h2>

  <div class="settings-grid">
    <!-- User & Focus Defaults -->
    <section class="card" aria-labelledby="settings-user-heading">
      <h3 id="settings-user-heading" style="margin: 0 0 0.85rem 0;">
        User & Focus Queue
      </h3>

      <div class="form-field" style="margin-bottom: 1rem;">
        <span class="field-label">Active User</span>
        <div
          style="display: flex; align-items: center; justify-content: space-between; gap: 0.75rem;"
        >
          <span class="badge">@{session.active_user}</span>
          <button type="button" onclick={onOpenUserModal}>
            Switch User...
          </button>
        </div>
      </div>

      <div class="form-field" style="margin-bottom: 1rem;">
        <span class="field-label">Active Project</span>
        <div
          style="display: flex; align-items: center; justify-content: space-between; gap: 0.75rem;"
        >
          <span class="badge">#{session.active_project}</span>
          <button type="button" onclick={onOpenProjectModal}>
            Switch Project...
          </button>
        </div>
      </div>

      <div class="form-field" style="margin-bottom: 1rem;">
        <label for="settings-focus-preset" class="field-label">
          Default Focus Preset
        </label>
        <Select
          id="settings-focus-preset"
          value={session.active_preset}
          options={Object.entries(PRESET_LABELS).map(([key, label]) => ({
            value: key,
            label,
          }))}
          onValueChange={(v) => onPresetChange(v as FocusPreset)}
        />
      </div>

      <div class="form-field">
        <label for="settings-focus-limit" class="field-label">
          Focus Queue Size ({session.focus_limit} tasks)
        </label>
        <input
          id="settings-focus-limit"
          type="number"
          min="1"
          max="50"
          value={session.focus_limit}
          onchange={(e) => {
            const val = parseInt(
              (e.currentTarget as HTMLInputElement).value,
              10,
            );
            if (!Number.isNaN(val)) {
              void onFocusLimitChange(val);
            }
          }}
        />
      </div>
    </section>

    <!-- Appearance & Accessibility -->
    <section class="card" aria-labelledby="settings-shortcuts-heading">
      <h3 id="settings-shortcuts-heading" style="margin: 0 0 0.85rem 0;">
        Keyboard Shortcuts
      </h3>
      <ul style="margin: 0; padding-left: 1.1rem; font-size: 0.9rem;">
        <li><kbd>Ctrl+U</kbd> switch active user (any view)</li>
        <li><kbd>Ctrl+P</kbd> switch active project (any view)</li>
        <li><kbd>1</kbd>–<kbd>5</kbd> switch views, <kbd>6</kbd> Settings</li>
        <li><kbd>Cmd/Ctrl+,</kbd> open Settings</li>
        <li><kbd>?</kbd> About and third-party licenses</li>
      </ul>
    </section>

    <section class="card" aria-labelledby="settings-a11y-heading">
      <h3 id="settings-a11y-heading" style="margin: 0 0 0.85rem 0;">
        Appearance & Accessibility
      </h3>

      <div class="form-field" style="margin-bottom: 1rem;">
        <label for="settings-theme-select" class="field-label">
          Color Scheme
        </label>
        <Select
          id="settings-theme-select"
          value={uiPrefs.theme}
          options={[
            { value: "system", label: "System Default (prefers-color-scheme)" },
            { value: "dark", label: "Dark" },
            { value: "oled", label: "OLED (Pure Black)" },
            { value: "light", label: "Light" },
          ]}
          onValueChange={(v) => setTheme(v as UiPreferences["theme"])}
        />
      </div>

      <div class="form-field" style="margin-bottom: 1rem;">
        <label for="settings-contrast-select" class="field-label">
          Contrast Level
        </label>
        <Select
          id="settings-contrast-select"
          value={uiPrefs.contrast}
          options={[
            { value: "system", label: "System Default (prefers-contrast)" },
            { value: "more", label: "High Contrast" },
          ]}
          onValueChange={(v) => setContrast(v as UiPreferences["contrast"])}
        />
      </div>

      <div class="form-field" style="margin-bottom: 1rem;">
        <label for="settings-palette-select" class="field-label">
          Status & Priority Color Palette
        </label>
        <Select
          id="settings-palette-select"
          value={uiPrefs.palette}
          options={[
            { value: "default", label: "Standard Palette" },
            { value: "okabe-ito", label: "Okabe-Ito (Colorblind-Safe)" },
          ]}
          onValueChange={(v) => setPalette(v as UiPreferences["palette"])}
        />
      </div>

      <div class="form-field">
        <div
          style="display: flex; justify-content: space-between; align-items: center;"
        >
          <label for="settings-font-scale" class="field-label">
            Relative Font Size ({uiPrefs.fontScalePercent}%)
          </label>
          {#if uiPrefs.fontScalePercent !== 100}
            <button
              type="button"
              class="btn-sm"
              onclick={() => setFontScale(100)}
            >
              Reset (100%)
            </button>
          {/if}
        </div>
        <input
          id="settings-font-scale"
          type="range"
          min="85"
          max="150"
          step="5"
          value={uiPrefs.fontScalePercent}
          oninput={(e) =>
            setFontScale(
              parseInt((e.currentTarget as HTMLInputElement).value, 10),
            )}
        />
      </div>
    </section>

    <!-- Database Connection -->
    <section
      class="card"
      aria-labelledby="settings-db-heading"
      style="grid-column: 1 / -1;"
    >
      <div
        style="display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 0.75rem; margin-bottom: 0.85rem;"
      >
        <h3 id="settings-db-heading" style="margin: 0;">Database Connection</h3>
        <span class="badge">
          {session.is_global_db ? "Global Database" : "Workspace Database"}
        </span>
      </div>

      <p
        style="margin: 0 0 1rem 0; font-family: var(--font-mono); font-size: 0.85rem; color: var(--text-secondary); word-break: break-all;"
      >
        {session.db_path}
      </p>

      <div
        style="display: flex; flex-wrap: wrap; gap: 0.65rem; margin-bottom: 1rem;"
      >
        <button type="button" onclick={onPickWorkspaceDb}>
          Open Workspace DB...
        </button>
        {#if !session.is_global_db}
          <button type="button" onclick={onSwitchGlobalDb}>
            Switch to Global DB
          </button>
        {/if}
      </div>

      {#if session.recent_workspaces.length > 0}
        <div class="form-field">
          <label for="settings-recent-db" class="field-label">
            Recent Workspaces
          </label>
          <Select
            id="settings-recent-db"
            value={session.is_global_db ? "" : session.db_path}
            options={[
              { value: "", label: "Select a recent workspace database..." },
              ...session.recent_workspaces.map((ws) => ({
                value: ws,
                label: ws,
              })),
            ]}
            onValueChange={(v) => {
              if (v) void onSwitchRecentDb(v);
            }}
          />
        </div>
      {/if}
    </section>

    {#if isMainUser}
      <!-- Main User Only: User Management -->
      <section
        class="card"
        aria-labelledby="settings-user-mgmt-heading"
        style="grid-column: 1 / -1;"
      >
        <div
          style="display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 0.75rem; margin-bottom: 0.5rem;"
        >
          <div>
            <h3 id="settings-user-mgmt-heading" style="margin: 0 0 0.25rem 0;">
              User Management
            </h3>
            <p
              style="margin: 0; font-size: 0.88rem; color: var(--text-secondary);"
            >
              Visible only to the main user (<strong
                >@{session.primary_user}</strong
              >). Deleting a user reassigns their tasks to
              <strong>@{session.primary_user}</strong> (with an option to also delete
              tasks they authored).
            </p>
          </div>
          <span class="badge">
            {session.known_users.length}
            {session.known_users.length === 1 ? "User" : "Users"}
          </span>
        </div>

        {#if userMgmtError}
          <div
            class="banner-alert"
            role="alert"
            style="margin-bottom: 0.75rem;"
          >
            <span>{userMgmtError}</span>
            <button
              type="button"
              class="btn-sm"
              onclick={() => (userMgmtError = null)}
            >
              Dismiss
            </button>
          </div>
        {/if}

        <datalist id="user-kind-options">
          {#each kindOptions as kind (kind)}
            <option value={kind}></option>
          {/each}
        </datalist>

        <ul class="user-mgmt-list" aria-label="Registered database users">
          {#each session.known_users as user (user)}
            <li class="user-mgmt-row">
              <div class="user-mgmt-identity">
                <UserPill {user} kind={userKindOf(session.user_kinds, user)} />
                {#if user.toLowerCase() === session.primary_user.toLowerCase()}
                  <span class="badge">Main User</span>
                {/if}
                {#if user.toLowerCase() === session.active_user.toLowerCase()}
                  <span class="badge">Active</span>
                {/if}
              </div>

              <form
                class="user-kind-form"
                onsubmit={(e) => {
                  e.preventDefault();
                  void saveUserKind(user);
                }}
              >
                <label for={`user-kind-${user}`}>Kind for @{user}</label>
                <input
                  id={`user-kind-${user}`}
                  type="text"
                  list="user-kind-options"
                  autocomplete="off"
                  value={kindDrafts[user] ??
                    userKindOf(session.user_kinds, user)}
                  oninput={(e) => {
                    kindDrafts[user] = (
                      e.currentTarget as HTMLInputElement
                    ).value;
                  }}
                />
                <button type="submit" class="btn-sm">Save kind</button>
              </form>

              {#if user.toLowerCase() !== session.primary_user.toLowerCase()}
                <button
                  type="button"
                  class="btn-sm btn-danger"
                  onclick={() => openDeleteUserDialog(user)}
                  aria-label={`Delete user @${user} from database`}
                >
                  Delete User...
                </button>
              {/if}
            </li>
          {/each}
        </ul>
      </section>
    {/if}

    <!-- About & Third-Party Licenses -->
    <section
      class="card"
      aria-labelledby="settings-about-heading"
      style="grid-column: 1 / -1; display: flex; flex-wrap: wrap; justify-content: space-between; align-items: center; gap: 1rem;"
    >
      <div>
        <h3 id="settings-about-heading" style="margin: 0 0 0.25rem 0;">
          About WSID & Open-Source Attribution
        </h3>
        <p style="margin: 0; font-size: 0.88rem; color: var(--text-secondary);">
          View version details, Apache-2.0 license text, and third-party Rust &
          frontend attributions.
        </p>
      </div>
      <button type="button" onclick={onOpenAbout}> About & Licenses... </button>
    </section>
  </div>
</section>

{#if deletePreview}
  <Dialog.Root
    open={true}
    onOpenChange={(next) => {
      if (!next) closeDeleteUserDialog();
    }}
  >
    <Dialog.Portal>
      <Dialog.Overlay class="confirm-modal-backdrop" />
      <Dialog.Content
        class="confirm-modal-dialog user-delete-modal-card"
        role="alertdialog"
      >
        <Dialog.Title class="confirm-modal-title">
          Delete User @{deletePreview.username}?
        </Dialog.Title>

        <Dialog.Description class="confirm-modal-desc">
          By deleting <strong>@{deletePreview.username}</strong>, you (<strong
            >@{deletePreview.fallback_user}</strong
          >) take over all tasks where
          <strong>@{deletePreview.username}</strong> was the author (reporter), assignee,
          or validator (evaluator):
        </Dialog.Description>

        <div class="meta-row" style="margin-bottom: 0.65rem;">
          <span class="badge">
            Affected tasks: {deletePreview.affected_tasks.length}
          </span>
          <span class="badge">
            Author: {deletePreview.authored_task_count}
          </span>
          <span class="badge">
            Assignee: {deletePreview.assigned_task_count}
          </span>
          <span class="badge">
            Validator: {deletePreview.evaluator_task_count}
          </span>
        </div>

        {#if deletePreview.affected_tasks.length === 0}
          <div class="user-delete-empty-box">
            No tasks in this database reference @{deletePreview.username}.
          </div>
        {:else}
          <div
            class="user-delete-task-scroll"
            role="region"
            aria-label={`Tasks affected by deleting @${deletePreview.username}`}
          >
            {#each deletePreview.affected_tasks as task (task.id)}
              <div
                class="user-delete-task-item"
                class:will-be-deleted={deleteAuthoredTasks && task.is_reporter}
              >
                <div
                  style="display: flex; align-items: center; gap: 0.5rem; min-width: 0; flex: 1;"
                >
                  {#if onSelectTask}
                    <button
                      type="button"
                      class="task-id-badge"
                      style="cursor: pointer;"
                      onclick={() => {
                        const id = task.id;
                        closeDeleteUserDialog();
                        onSelectTask(id);
                      }}
                    >
                      {task.id}
                    </button>
                  {:else}
                    <span class="task-id-badge">{task.id}</span>
                  {/if}
                  <span class="user-delete-task-title" title={task.title}>
                    {task.title}
                  </span>
                </div>

                <div class="user-delete-role-badges">
                  <span class="badge">{STATUS_LABELS[task.status]}</span>
                  {#if task.is_reporter}
                    <span class="badge">Author</span>
                  {/if}
                  {#if task.is_assignee}
                    <span class="badge">Assignee</span>
                  {/if}
                  {#if task.is_evaluator}
                    <span class="badge">Validator</span>
                  {/if}
                  {#if deleteAuthoredTasks && task.is_reporter}
                    <span class="badge badge-urgent">Will be deleted</span>
                  {:else}
                    <span class="badge">→ @{deletePreview.fallback_user}</span>
                  {/if}
                </div>
              </div>
            {/each}
          </div>
        {/if}

        <label class="user-delete-authored-toggle">
          <input type="checkbox" bind:checked={deleteAuthoredTasks} />
          <span>
            Also permanently delete all tasks where
            <strong>@{deletePreview.username}</strong> was the author ({deletePreview.authored_task_count}
            {deletePreview.authored_task_count === 1 ? "task" : "tasks"})
          </span>
        </label>

        <div class="confirm-modal-actions">
          <button
            type="button"
            onclick={closeDeleteUserDialog}
            disabled={deleteBusy}
          >
            Cancel
          </button>
          <button
            type="button"
            class="btn-danger"
            onclick={() => void confirmDeleteUser()}
            disabled={deleteBusy}
          >
            {deleteBusy
              ? "Deleting..."
              : deleteAuthoredTasks && deletePreview.authored_task_count > 0
                ? `Delete @${deletePreview.username} & ${deletePreview.authored_task_count} Authored Task(s)`
                : `Delete @${deletePreview.username} & Reassign Tasks`}
          </button>
        </div>
      </Dialog.Content>
    </Dialog.Portal>
  </Dialog.Root>
{/if}
