<script lang="ts">
  import { onMount } from "svelte";
  import {
    getSessionState,
    setActivePersona,
    setFocusPreset,
    switchDatabase,
    pickWorkspaceDatabaseDialog,
    getFocusViewData,
    getBlockedTasksData,
    getAllTasksData,
    getEisenhowerData,
    subscribeToExternalDbChanges,
    type AppSessionSnapshot,
    type FocusPreset,
    type FocusQueueResponse,
    type BlockedTaskEntry,
    type Task,
    type EisenhowerClassification,
  } from "./lib/ipc";
  import FocusView from "./components/FocusView.svelte";
  import KanbanView from "./components/KanbanView.svelte";
  import EisenhowerView from "./components/EisenhowerView.svelte";
  import DatabaseView from "./components/DatabaseView.svelte";
  import DataEntryView from "./components/DataEntryView.svelte";
  import TaskDetailDrawer from "./components/TaskDetailDrawer.svelte";
  import AboutModal from "./components/AboutModal.svelte";

  type ViewTab = "focus" | "kanban" | "eisenhower" | "database" | "entry";

  let activeTab = $state<ViewTab>("focus");
  let session = $state<AppSessionSnapshot | null>(null);
  let personaInput = $state<string>("");
  let filterMyTasksOnly = $state<boolean>(false);
  let focusQueue = $state<FocusQueueResponse | null>(null);
  let blockedTasks = $state<BlockedTaskEntry[]>([]);
  let allTasks = $state<Task[]>([]);
  let eisenhowerItems = $state<EisenhowerClassification[]>([]);
  let selectedTaskId = $state<string | null>(null);
  let aboutModalOpen = $state<boolean>(false);
  let statusAnnouncement = $state<string>("Ready.");
  let loading = $state<boolean>(true);
  let refreshTrigger = $state<number>(0);

  const tabs: { id: ViewTab; label: string; shortcut: string }[] = [
    { id: "focus", label: "Focus View", shortcut: "1" },
    { id: "kanban", label: "Kanban Board", shortcut: "2" },
    { id: "eisenhower", label: "Eisenhower Matrix", shortcut: "3" },
    { id: "database", label: "Database Search", shortcut: "4" },
    { id: "entry", label: "New Task", shortcut: "5" },
  ];

  function announce(msg: string) {
    statusAnnouncement = msg;
  }

  async function refreshAllData() {
    try {
      const snap = await getSessionState();
      session = snap;
      personaInput = snap.active_persona;
      const personaFilter = filterMyTasksOnly ? snap.active_persona : null;
      const [fq, blocked, tasks, matrix] = await Promise.all([
        getFocusViewData(snap.active_persona, snap.active_preset),
        getBlockedTasksData(snap.active_persona),
        getAllTasksData(),
        getEisenhowerData(personaFilter),
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

  onMount(() => {
    refreshAllData();
    let unlisten: (() => void) | null = null;
    subscribeToExternalDbChanges(() => {
      announce("Database updated externally; refreshing views.");
      refreshAllData();
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      if (unlisten) unlisten();
    };
  });

  async function handlePersonaApply(e: SubmitEvent) {
    e.preventDefault();
    const trimmed = personaInput.trim();
    if (!trimmed) return;
    try {
      session = await setActivePersona(trimmed);
      announce(`Switched active persona to @${session.active_persona}.`);
      await refreshAllData();
    } catch (err) {
      announce(`Failed to change persona: ${String(err)}`);
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

  function openTaskDetail(taskId: string) {
    selectedTaskId = taskId;
    announce(`Opened details for task ${taskId}.`);
  }

  function closeTaskDetail() {
    selectedTaskId = null;
    announce("Closed task details.");
  }

  function handleGlobalKeydown(e: KeyboardEvent) {
    const target = e.target as HTMLElement | null;
    const isInput =
      target &&
      (target.tagName === "INPUT" ||
        target.tagName === "TEXTAREA" ||
        target.tagName === "SELECT" ||
        target.isContentEditable);

    if (isInput || selectedTaskId || aboutModalOpen) return;

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

<div class="app-shell">
  <header class="top-bar">
    <div class="brand">
      <span class="brand-badge">WSID</span>
      <span>What Should I Do?</span>
    </div>

    {#if session}
      <div
        class="db-switcher"
        role="group"
        aria-label="Database and Persona Switcher"
      >
        <span
          class="badge"
          class:badge-urgent={!session.is_global_db}
          title={session.db_path}
        >
          {session.is_global_db ? "Global DB" : "Workspace DB"}:
          <code>{session.db_path.split(/[\\/]/).pop()}</code>
        </span>

        <button type="button" class="btn-sm" onclick={handlePickWorkspaceDb}>
          Open Workspace DB...
        </button>

        {#if !session.is_global_db}
          <button
            type="button"
            class="btn-sm"
            onclick={handleSwitchToGlobalDb}
          >
            Switch to Global DB
          </button>
        {/if}

        {#if session.recent_workspaces.length > 0}
          <label class="sr-only" for="recent-db-select">Recent Databases</label>
          <select
            id="recent-db-select"
            aria-label="Recent Workspace Databases"
            value=""
            onchange={(e) => handleSwitchRecentDb(e.currentTarget.value)}
          >
            <option value="" disabled>Recent DBs...</option>
            <option value={session.global_db_path}>
              Global ({session.global_db_path})
            </option>
            {#each session.recent_workspaces as rw}
              <option value={rw}>{rw}</option>
            {/each}
          </select>
        {/if}

        <form class="inline-actions" onsubmit={handlePersonaApply}>
          <label for="persona-input" class="text-sm text-muted">Persona:</label>
          <input
            id="persona-input"
            type="text"
            style="width: 7.5rem;"
            bind:value={personaInput}
            aria-label="Active Persona"
          />
          <button type="submit" class="btn-sm">Set</button>
        </form>

        <button
          type="button"
          class="btn-sm"
          onclick={() => {
            aboutModalOpen = true;
            announce("Opened About and Third-Party Licenses dialog.");
          }}
          aria-label="About WSID and Third-Party Licenses (Shortcut: ?)"
        >
          About &amp; Licenses (?)
        </button>
      </div>
    {/if}
  </header>

  <nav class="nav-tabs" aria-label="Primary Views">
    {#each tabs as tab}
      <button
        type="button"
        class="tab-btn"
        aria-current={activeTab === tab.id ? "page" : undefined}
        onclick={() => {
          activeTab = tab.id;
          announce(`Switched to ${tab.label}.`);
        }}
      >
        [{tab.shortcut}] {tab.label}
      </button>
    {/each}
  </nav>

  <main id="main-content" class="main-content" tabindex="-1">
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
        activePersona={session?.active_persona ?? "human"}
        onSelectTask={openTaskDetail}
        onDataChanged={refreshAllData}
        {announce}
      />
    {:else if activeTab === "eisenhower"}
      <EisenhowerView
        items={eisenhowerItems}
        activePersona={session?.active_persona ?? "human"}
        filterByPersona={filterMyTasksOnly}
        onTogglePersonaFilter={async (enabled) => {
          filterMyTasksOnly = enabled;
          await refreshAllData();
        }}
        onSelectTask={openTaskDetail}
      />
    {:else if activeTab === "database"}
      <DatabaseView
        knownTags={session?.known_tags ?? []}
        knownPersonas={session?.known_personas ?? []}
        {refreshTrigger}
        onSelectTask={openTaskDetail}
        {announce}
      />
    {:else if activeTab === "entry"}
      <DataEntryView
        activePersona={session?.active_persona ?? "human"}
        knownPersonas={session?.known_personas ?? []}
        knownTags={session?.known_tags ?? []}
        onCreated={async (createdId) => {
          await refreshAllData();
          activeTab = "focus";
          openTaskDetail(createdId);
        }}
        {announce}
      />
    {/if}
  </main>

  <footer class="status-bar">
    <span>
      Active Persona: <strong>@{session?.active_persona ?? "human"}</strong> |
      Total Tasks: <strong>{allTasks.length}</strong>
    </span>
    <span>{statusAnnouncement}</span>
  </footer>
</div>

{#if selectedTaskId && session}
  <TaskDetailDrawer
    taskId={selectedTaskId}
    activePersona={session.active_persona}
    onClose={closeTaskDetail}
    onSelectTask={openTaskDetail}
    onDataChanged={refreshAllData}
    {announce}
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
