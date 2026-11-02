<script lang="ts">
  import { onMount } from "svelte";
  import {
    getSessionState,
    setActiveUser,
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

  type ViewTab =
    "focus" | "kanban" | "eisenhower" | "database" | "entry" | "settings";

  let activeTab = $state<ViewTab>("focus");
  let session = $state<AppSessionSnapshot | null>(null);
  let filterMyTasksOnly = $state<boolean>(false);
  let focusQueue = $state<FocusQueueResponse | null>(null);
  let blockedTasks = $state<BlockedTaskEntry[]>([]);
  let allTasks = $state<Task[]>([]);
  let eisenhowerItems = $state<EisenhowerClassification[]>([]);
  let selectedTaskId = $state<string | null>(null);
  let aboutModalOpen = $state<boolean>(false);
  let userSwitcherOpen = $state<boolean>(false);
  let statusAnnouncement = $state<string>("Ready.");
  let loading = $state<boolean>(true);
  let refreshTrigger = $state<number>(0);
  let uiPreferences = $state<UiPreferences>(loadUiPreferences());

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

  function handleUpdateUiPreferences(next: UiPreferences) {
    uiPreferences = next;
    saveUiPreferences(next);
    applyUiPreferencesToDom(next);
  }

  async function refreshAllData() {
    try {
      const snap = await getSessionState();
      session = snap;
      const userFilter = filterMyTasksOnly ? snap.active_user : null;
      const [fq, blocked, tasks, matrix] = await Promise.all([
        getFocusViewData(
          snap.active_user,
          snap.active_preset,
          snap.focus_limit,
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
    try {
      session = await setActiveUser(trimmed);
      userSwitcherOpen = false;
      announce(`Switched active user to @${session.active_user}.`);
      await refreshAllData();
    } catch (err) {
      announce(`Failed to change user: ${String(err)}`);
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

  async function handlePickWorkspaceDb() {
    try {
      const picked = await pickWorkspaceDatabaseDialog();
      if (!picked) return;
      session = await switchDatabase(picked, false);
      announce(`Switched active database to ${session.db_path}.`);
      await refreshAllData();
    } catch (err) {
      announce(`Failed to switch database: ${String(err)}`);
    }
  }

  async function handleSwitchToGlobalDb() {
    try {
      session = await switchDatabase(null, true);
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
      activeTab = "settings";
      announce("Opened Settings.");
    } else if (action === "open_about") {
      aboutModalOpen = true;
      announce("Opened About and Third-Party Licenses dialog.");
    } else if (action.startsWith("view:")) {
      const view = action.slice("view:".length) as ViewTab;
      activeTab = view;
      announce(`Switched to ${view} view.`);
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
      activeTab = "settings";
      announce("Opened Settings.");
      return;
    }

    const target = e.target as HTMLElement | null;
    const isInput =
      target &&
      (target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.tagName === "SELECT" ||
        target.isContentEditable);

    if (isInput || selectedTaskId || aboutModalOpen || userSwitcherOpen) return;

    if (e.key === "1") activeTab = "focus";
    else if (e.key === "2") activeTab = "kanban";
    else if (e.key === "3") activeTab = "eisenhower";
    else if (e.key === "4") activeTab = "database";
    else if (e.key === "5" || e.key === "n") activeTab = "entry";
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
    Active user: @{session.active_user}. Connected database: {session.is_global_db
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
            activeTab = tab.id;
            announce(`Switched to ${tab.label}.`);
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
          activeTab = "settings";
          announce("Opened Settings.");
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
      <FocusView
        focusData={focusQueue}
        {blockedTasks}
        onSelectTask={openTaskDetail}
        onPresetChange={handlePresetChange}
        onDataChanged={refreshAllData}
        {announce}
      />
    {:else if activeTab === "kanban"}
      <KanbanView
        tasks={allTasks}
        activeUser={session?.active_user ?? "human"}
        onSelectTask={openTaskDetail}
        onDataChanged={refreshAllData}
        {announce}
      />
    {:else if activeTab === "eisenhower"}
      <EisenhowerView
        items={eisenhowerItems}
        activeUser={session?.active_user ?? "human"}
        filterByUser={filterMyTasksOnly}
        onToggleUserFilter={async (enabled) => {
          filterMyTasksOnly = enabled;
          await refreshAllData();
        }}
        onSelectTask={openTaskDetail}
      />
    {:else if activeTab === "database"}
      <DatabaseView
        knownTags={session?.known_tags ?? []}
        knownUsers={session?.known_users ?? []}
        {refreshTrigger}
        onSelectTask={openTaskDetail}
        {announce}
      />
    {:else if activeTab === "entry"}
      <DataEntryView
        activeUser={session?.active_user ?? "human"}
        knownUsers={session?.known_users ?? []}
        knownTags={session?.known_tags ?? []}
        onCreated={async (createdId) => {
          await refreshAllData();
          activeTab = "focus";
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
        onPresetChange={handlePresetChange}
        onFocusLimitChange={handleFocusLimitChange}
        onPickWorkspaceDb={handlePickWorkspaceDb}
        onSwitchGlobalDb={handleSwitchToGlobalDb}
        onSwitchRecentDb={handleSwitchRecentDb}
        onOpenAbout={() => {
          aboutModalOpen = true;
          announce("Opened About and Third-Party Licenses dialog.");
        }}
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
        title="Switch active user"
        aria-label={`Active user @${session?.active_user ?? "human"}. Click to switch user.`}
      >
        @{session?.active_user ?? "human"}
      </button>
    </div>
  </footer>
</div>

{#if selectedTaskId && session}
  <TaskDetailDrawer
    taskId={selectedTaskId}
    activeUser={session.active_user}
    knownTags={session.known_tags}
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

{#if aboutModalOpen}
  <AboutModal
    onClose={() => {
      aboutModalOpen = false;
      announce("Closed About and Third-Party Licenses dialog.");
    }}
  />
{/if}
