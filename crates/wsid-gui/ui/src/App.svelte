<script lang="ts">
  import { onMount } from "svelte";
  import {
    getSessionState,
    setActiveUser,
    setActiveProject,
    setFocusPreset,
    setFocusLimit,
    switchDatabase,
    pickWorkspaceDatabaseDialog,
    getFocusViewData,
    getBlockedTasksData,
    getAllTasksData,
    getEisenhowerData,
    subscribeToExternalDbChanges,
    subscribeToMenuActions,
    loadUiPreferences,
    saveUiPreferences,
    applyUiPreferencesToDom,
    type AppSessionSnapshot,
    type FocusPreset,
    type FocusQueueResponse,
    type BlockedTaskEntry,
    type Task,
    type EisenhowerClassification,
    type UiPreferences,
  } from "./lib/ipc";
  import FocusView from "./components/FocusView.svelte";
  import KanbanView from "./components/KanbanView.svelte";
  import EisenhowerView from "./components/EisenhowerView.svelte";
  import DatabaseView from "./components/DatabaseView.svelte";
  import DataEntryView from "./components/DataEntryView.svelte";
  import SettingsView from "./components/SettingsView.svelte";
  import TaskDetailDrawer from "./components/TaskDetailDrawer.svelte";
  import AboutModal from "./components/AboutModal.svelte";
  import UserSwitcherModal from "./components/UserSwitcherModal.svelte";
  import ProjectSwitcherModal from "./components/ProjectSwitcherModal.svelte";
  import ConfirmModal from "./components/ConfirmModal.svelte";

  type ViewTab =
    "focus" | "kanban" | "eisenhower" | "database" | "entry" | "settings";

  let activeTab = $state<ViewTab>("focus");
  let session = $state<AppSessionSnapshot | null>(null);
  /** Assignee filter for the Eisenhower matrix; `all` means every user. */
  let eisenhowerUser = $state<string>("all");
  let focusQueue = $state<FocusQueueResponse | null>(null);
  /** Focus queue project scope; unlike Kanban/Eisenhower it starts on all. */
  let focusProject = $state<string>("all");
  let blockedTasks = $state<BlockedTaskEntry[]>([]);
  let allTasks = $state<Task[]>([]);
  let eisenhowerItems = $state<EisenhowerClassification[]>([]);
  let selectedTaskId = $state<string | null>(null);
  let aboutModalOpen = $state<boolean>(false);
  let userSwitcherOpen = $state<boolean>(false);
  let projectSwitcherOpen = $state<boolean>(false);
  /** Bumped on database switch so mounted views drop their temporary filter scope. */
  let viewEpoch = $state<number>(0);
  let statusAnnouncement = $state<string>("Ready.");
  let loading = $state<boolean>(true);
  let refreshTrigger = $state<number>(0);
  let uiPreferences = $state<UiPreferences>(loadUiPreferences());
  let entryFormDirty = $state<boolean>(false);
  let pendingTabSwitch = $state<{
    nextTab: ViewTab;
    announcement?: string;
  } | null>(null);
  let confirmLeaveOpen = $state<boolean>(false);

  const tabs: { id: ViewTab; label: string; shortcut: string }[] = [
    { id: "focus", label: "Focus", shortcut: "1" },
    { id: "kanban", label: "Kanban", shortcut: "2" },
    { id: "eisenhower", label: "Eisenhower", shortcut: "3" },
    { id: "database", label: "Database", shortcut: "4" },
    { id: "entry", label: "New Task", shortcut: "5" },
  ];

  function announce(msg: string) {
    statusAnnouncement = msg;
  }

  /**
   * Enters a view through one path so its temporary scope resets like the TUI `apply_view_scope`.
   * Focus returns to all projects. Kanban and Eisenhower initialize from the active project and
   * Database remounts with cleared filters, so those views need no work here.
   */
  function enterView(nextTab: ViewTab) {
    if (nextTab === "focus" && focusProject !== "all") {
      focusProject = "all";
      void refreshAllData();
    }
    if (nextTab === "eisenhower" && eisenhowerUser !== "all") {
      eisenhowerUser = "all";
      void refreshAllData();
    }
    activeTab = nextTab;
  }

  function switchTab(nextTab: ViewTab, announcement?: string) {
    if (nextTab === activeTab) return;
    if (activeTab === "entry" && entryFormDirty) {
      pendingTabSwitch = { nextTab, announcement };
      confirmLeaveOpen = true;
      return;
    }
    enterView(nextTab);
    if (announcement) {
      announce(announcement);
    }
  }

  function confirmLeaveNewTask() {
    const pending = pendingTabSwitch;
    pendingTabSwitch = null;
    confirmLeaveOpen = false;
    entryFormDirty = false;
    if (pending) {
      enterView(pending.nextTab);
      if (pending.announcement) {
        announce(pending.announcement);
      }
    }
  }

  function cancelLeaveNewTask() {
    pendingTabSwitch = null;
    confirmLeaveOpen = false;
  }

  function handleUpdateUiPreferences(next: UiPreferences) {
    uiPreferences = next;
    saveUiPreferences(next);
    applyUiPreferencesToDom(next);
  }

  async function refreshAllData() {
    try {
      const snap = await getSessionState();
      session = snap;
      const userFilter = eisenhowerUser === "all" ? null : eisenhowerUser;
      const [fq, blocked, tasks, matrix] = await Promise.all([
        getFocusViewData(
          snap.active_user,
          snap.active_preset,
          snap.focus_limit,
          focusProject,
        ),
        getBlockedTasksData(snap.active_user),
        getAllTasksData(),
        getEisenhowerData(userFilter),
      ]);
      focusQueue = fq;
      blockedTasks = blocked;
      allTasks = tasks;
      eisenhowerItems = matrix;
      refreshTrigger += 1;
    } catch (err) {
      announce(`Error loading data: ${String(err)}`);
    } finally {
      loading = false;
    }
  }

  async function handleSelectUser(user: string) {
    const trimmed = user.trim();
    if (!trimmed) return;
    const previousUser = session?.active_user ?? null;
    try {
      session = await setActiveUser(trimmed);
      // An Eisenhower user filter that named the previous active user follows the switch.
      if (eisenhowerUser === previousUser) eisenhowerUser = session.active_user;
      userSwitcherOpen = false;
      announce(`Switched active user to @${session.active_user}.`);
      await refreshAllData();
    } catch (err) {
      announce(`Failed to change user: ${String(err)}`);
    }
  }

  async function handleSelectProject(project: string) {
    const trimmed = project.trim();
    if (!trimmed) return;
    try {
      session = await setActiveProject(trimmed);
      projectSwitcherOpen = false;
      announce(`Switched active project to #${session.active_project}.`);
      await refreshAllData();
    } catch (err) {
      announce(`Failed to change project: ${String(err)}`);
    }
  }

  async function handlePresetChange(preset: FocusPreset) {
    try {
      session = await setFocusPreset(preset);
      announce(`Changed Focus scoring preset to ${preset}.`);
      await refreshAllData();
    } catch (err) {
      announce(`Failed to change preset: ${String(err)}`);
    }
  }

  async function handleFocusLimitChange(limit: number) {
    try {
      session = await setFocusLimit(limit);
      announce(`Updated Focus queue limit to ${session.focus_limit}.`);
      await refreshAllData();
    } catch (err) {
      announce(`Failed to update Focus limit: ${String(err)}`);
    }
  }

  /** Database switches reset every view's temporary scope, matching the TUI `reset_after_database_switch`. */
  function resetViewScopeAfterDatabaseSwitch() {
    focusProject = "all";
    eisenhowerUser = "all";
    viewEpoch += 1;
  }

  async function handlePickWorkspaceDb() {
    try {
      const picked = await pickWorkspaceDatabaseDialog();
      if (!picked) return;
      session = await switchDatabase(picked, false);
      resetViewScopeAfterDatabaseSwitch();
      announce(`Switched active database to ${session.db_path}.`);
      await refreshAllData();
    } catch (err) {
      announce(`Failed to switch database: ${String(err)}`);
    }
  }

  async function handleSwitchToGlobalDb() {
    try {
      session = await switchDatabase(null, true);
      resetViewScopeAfterDatabaseSwitch();
      announce(`Switched to global database (${session.global_db_path}).`);
      await refreshAllData();
    } catch (err) {
      announce(`Failed to switch database: ${String(err)}`);
    }
  }

  async function handleSwitchRecentDb(path: string) {
    if (!path) return;
    try {
      const isGlobal = session ? path === session.global_db_path : false;
      session = await switchDatabase(isGlobal ? null : path, isGlobal);
      resetViewScopeAfterDatabaseSwitch();
      announce(`Switched active database to ${session.db_path}.`);
      await refreshAllData();
    } catch (err) {
      announce(`Failed to switch database: ${String(err)}`);
    }
  }

  function handleMenuAction(action: string) {
    if (action === "open_workspace_db") {
      void handlePickWorkspaceDb();
    } else if (action === "switch_global_db") {
      void handleSwitchToGlobalDb();
    } else if (action === "open_settings") {
      switchTab("settings", "Opened Settings.");
    } else if (action === "switch_user") {
      userSwitcherOpen = true;
    } else if (action === "switch_project") {
      projectSwitcherOpen = true;
    } else if (action === "open_about") {
      aboutModalOpen = true;
      announce("Opened About and Third-Party Licenses dialog.");
    } else if (action.startsWith("view:")) {
      const view = action.slice("view:".length) as ViewTab;
      switchTab(view, `Switched to ${view} view.`);
    } else if (action.startsWith("switch_db:")) {
      const path = action.slice("switch_db:".length);
      void handleSwitchRecentDb(path);
    }
  }

  onMount(() => {
    applyUiPreferencesToDom(uiPreferences);
    void refreshAllData();
    let unlistenDb: (() => void) | null = null;
    let unlistenMenu: (() => void) | null = null;

    subscribeToExternalDbChanges(() => {
      announce("Database updated externally; refreshing views.");
      void refreshAllData();
    }).then((fn) => {
      unlistenDb = fn;
    });

    subscribeToMenuActions((action) => {
      handleMenuAction(action);
    }).then((fn) => {
      unlistenMenu = fn;
    });

    return () => {
      if (unlistenDb) unlistenDb();
      if (unlistenMenu) unlistenMenu();
    };
  });

  let activeProject = $derived(session?.active_project ?? "default");

  function openTaskDetail(taskId: string) {
    selectedTaskId = taskId;
    announce(`Opened details for task ${taskId}.`);
  }

  function closeTaskDetail() {
    selectedTaskId = null;
    announce("Closed task details.");
  }

  function handleGlobalKeydown(e: KeyboardEvent) {
    if ((e.metaKey || e.ctrlKey) && e.key === ",") {
      e.preventDefault();
      switchTab("settings", "Opened Settings.");
      return;
    }

    // Ctrl+U / Ctrl+P switch the active user / project from any view, including text inputs.
    if (
      e.ctrlKey &&
      !e.metaKey &&
      !e.altKey &&
      (e.key === "u" || e.key === "p")
    ) {
      if (
        selectedTaskId ||
        aboutModalOpen ||
        confirmLeaveOpen ||
        userSwitcherOpen ||
        projectSwitcherOpen
      )
        return;
      e.preventDefault();
      if (e.key === "u") {
        userSwitcherOpen = true;
      } else {
        projectSwitcherOpen = true;
      }
      return;
    }

    const target = e.target as HTMLElement | null;
    const isInput =
      target &&
      (target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.tagName === "SELECT" ||
        target.closest("[data-select-trigger], [data-select-content]") ||
        target.isContentEditable);

    if (
      isInput ||
      selectedTaskId ||
      aboutModalOpen ||
      userSwitcherOpen ||
      projectSwitcherOpen ||
      confirmLeaveOpen
    )
      return;

    if (e.key === "1") switchTab("focus", "Switched to Focus.");
    else if (e.key === "2") switchTab("kanban", "Switched to Kanban.");
    else if (e.key === "3") switchTab("eisenhower", "Switched to Eisenhower.");
    else if (e.key === "4") switchTab("database", "Switched to Database.");
    else if (e.key === "5" || e.key === "n")
      switchTab("entry", "Switched to New Task.");
    else if (e.key === "6") switchTab("settings", "Opened Settings.");
    else if (e.key === "?") {
      aboutModalOpen = true;
      announce("Opened About and Third-Party Licenses dialog.");
    }
  }
</script>

<svelte:window onkeydown={handleGlobalKeydown} />

<a class="skip-link" href="#main-content">Skip to main content</a>

<div class="sr-only" aria-live="polite" aria-atomic="true">
  {statusAnnouncement}
</div>

{#if session}
  <div class="sr-only">
    Active user: @{session.active_user}. Active project: #{session.active_project}.
    Connected database: {session.is_global_db
      ? "Global database"
      : "Workspace database"} ({session.db_path}).
  </div>
{/if}

<div class="app-shell">
  <header class="top-bar">
    <div class="brand">
      <span class="brand-badge">WSID</span>
    </div>

    <nav class="nav-tabs" aria-label="Primary Views">
      {#each tabs as tab}
        <button
          type="button"
          class="tab-btn"
          aria-current={activeTab === tab.id ? "page" : undefined}
          aria-keyshortcuts={tab.shortcut}
          title={`${tab.label} (Shortcut: ${tab.shortcut})`}
          onclick={() => {
            switchTab(tab.id, `Switched to ${tab.label}.`);
          }}
        >
          {tab.label}
        </button>
      {/each}
    </nav>

    <div class="inline-actions">
      <button
        type="button"
        class="tab-btn"
        aria-current={activeTab === "settings" ? "page" : undefined}
        aria-keyshortcuts="Meta+Comma Control+Comma"
        title="Settings (Cmd/Ctrl+,)"
        onclick={() => {
          switchTab("settings", "Opened Settings.");
        }}
      >
        Settings
      </button>
    </div>
  </header>

  <main
    id="main-content"
    class="main-content"
    class:main-content-kanban={activeTab === "kanban"}
    tabindex="-1"
  >
    {#if loading}
      <p role="status">Loading WSID workspace...</p>
    {:else if activeTab === "focus"}
      {#key viewEpoch}
        <FocusView
          focusData={focusQueue}
          {blockedTasks}
          knownProjects={session?.known_projects ?? []}
          projectFilter={focusProject}
          onProjectChange={async (project) => {
            focusProject = project;
            announce(`Focus queue project: ${project}`);
            await refreshAllData();
          }}
          onSelectTask={openTaskDetail}
          onPresetChange={handlePresetChange}
          onDataChanged={refreshAllData}
          {announce}
        />
      {/key}
    {:else if activeTab === "kanban"}
      {#key viewEpoch}
        <KanbanView
          tasks={allTasks}
          activeUser={session?.active_user ?? "human"}
          {activeProject}
          knownTags={session?.known_tags ?? []}
          knownUsers={session?.known_users ?? []}
          knownProjects={session?.known_projects ?? []}
          onSelectTask={openTaskDetail}
          onDataChanged={refreshAllData}
          {announce}
        />
      {/key}
    {:else if activeTab === "eisenhower"}
      {#key viewEpoch}
        <EisenhowerView
          items={eisenhowerItems}
          activeUser={session?.active_user ?? "human"}
          {activeProject}
          userFilter={eisenhowerUser}
          onUserFilterChange={async (user) => {
            eisenhowerUser = user;
            announce(
              "Matrix user filter: " +
                (user === "all" ? "all users" : "@" + user),
            );
            await refreshAllData();
          }}
          knownTags={session?.known_tags ?? []}
          knownUsers={session?.known_users ?? []}
          knownProjects={session?.known_projects ?? []}
          onSelectTask={openTaskDetail}
          onDataChanged={refreshAllData}
          {announce}
        />
      {/key}
    {:else if activeTab === "database"}
      {#key viewEpoch}
        <DatabaseView
          knownTags={session?.known_tags ?? []}
          knownUsers={session?.known_users ?? []}
          knownProjects={session?.known_projects ?? []}
          {refreshTrigger}
          onSelectTask={openTaskDetail}
          onDataChanged={refreshAllData}
          {announce}
        />
      {/key}
    {:else if activeTab === "entry"}
      <DataEntryView
        activeUser={session?.active_user ?? "human"}
        {activeProject}
        knownUsers={session?.known_users ?? []}
        knownTags={session?.known_tags ?? []}
        knownProjects={session?.known_projects ?? []}
        bind:isDirty={entryFormDirty}
        onCreated={async (createdId) => {
          entryFormDirty = false;
          await refreshAllData();
          enterView("focus");
          openTaskDetail(createdId);
        }}
        {announce}
      />
    {:else if activeTab === "settings" && session}
      <SettingsView
        {session}
        uiPrefs={uiPreferences}
        onUpdateUiPrefs={handleUpdateUiPreferences}
        onOpenUserModal={() => (userSwitcherOpen = true)}
        onOpenProjectModal={() => (projectSwitcherOpen = true)}
        onPresetChange={handlePresetChange}
        onFocusLimitChange={handleFocusLimitChange}
        onPickWorkspaceDb={handlePickWorkspaceDb}
        onSwitchGlobalDb={handleSwitchToGlobalDb}
        onSwitchRecentDb={handleSwitchRecentDb}
        onOpenAbout={() => {
          aboutModalOpen = true;
          announce("Opened About and Third-Party Licenses dialog.");
        }}
        onUserDeleted={refreshAllData}
        onSelectTask={openTaskDetail}
        {announce}
      />
    {/if}
  </main>

  <footer class="status-bar">
    <span class="status-announcement">{statusAnnouncement}</span>
    <div class="status-right">
      <span class="text-muted">Tasks: {allTasks.length}</span>
      <button
        type="button"
        class="status-user-btn"
        onclick={() => (userSwitcherOpen = true)}
        title="Switch active user (Ctrl+U)"
        aria-keyshortcuts="Control+U"
        aria-label={`Active user @${session?.active_user ?? "human"}. Click to switch user.`}
      >
        @{session?.active_user ?? "human"}
      </button>
      <button
        type="button"
        class="status-user-btn"
        onclick={() => (projectSwitcherOpen = true)}
        title="Switch active project (Ctrl+P)"
        aria-keyshortcuts="Control+P"
        aria-label={`Active project #${activeProject}. Click to switch project.`}
      >
        #{activeProject}
      </button>
    </div>
  </footer>
</div>

{#if selectedTaskId && session}
  <TaskDetailDrawer
    taskId={selectedTaskId}
    activeUser={session.active_user}
    knownTags={session.known_tags}
    knownUsers={session.known_users}
    knownProjects={session.known_projects}
    onClose={closeTaskDetail}
    onSelectTask={openTaskDetail}
    onDataChanged={refreshAllData}
    {announce}
  />
{/if}

{#if userSwitcherOpen && session}
  <UserSwitcherModal
    activeUser={session.active_user}
    knownUsers={session.known_users}
    onSelectUser={handleSelectUser}
    onClose={() => (userSwitcherOpen = false)}
  />
{/if}

{#if projectSwitcherOpen && session}
  <ProjectSwitcherModal
    activeProject={session.active_project}
    knownProjects={session.known_projects}
    onSelectProject={handleSelectProject}
    onClose={() => (projectSwitcherOpen = false)}
  />
{/if}

{#if aboutModalOpen}
  <AboutModal
    activeUser={session?.active_user ?? "human"}
    {activeProject}
    onClose={() => {
      aboutModalOpen = false;
      announce("Closed About and Third-Party Licenses dialog.");
    }}
  />
{/if}

<ConfirmModal
  bind:open={confirmLeaveOpen}
  title="Discard Unsaved Task?"
  description="You have unsaved data in the New Task form. Leaving this page will clear the form."
  confirmLabel="Clear Form & Leave"
  cancelLabel="Stay on Page"
  danger={true}
  onConfirm={confirmLeaveNewTask}
  onCancel={cancelLeaveNewTask}
/>
