<script lang="ts">
  import { Command, Dialog } from "bits-ui";
  import UserPill from "./UserPill.svelte";
  import {
    type AppSessionSnapshot,
    userKindOf,
    validateUsername,
  } from "../lib/ipc";

  type PaletteView =
    "focus" | "kanban" | "eisenhower" | "database" | "entry" | "settings";
  type PaletteAction = "open_workspace_db" | "switch_global_db" | "open_about";

  interface Props {
    open: boolean;
    /** Text the palette opens with: "@" for users (Ctrl+U), "#" for projects (Ctrl+P), "" otherwise. */
    initialQuery?: string;
    session: AppSessionSnapshot;
    onSelectUser: (user: string) => void | Promise<void>;
    onSelectProject: (project: string) => void | Promise<void>;
    onSwitchView: (view: PaletteView) => void;
    onAction: (action: PaletteAction) => void;
  }

  let {
    open = $bindable(false),
    initialQuery = "",
    session,
    onSelectUser,
    onSelectProject,
    onSwitchView,
    onAction,
  }: Props = $props();

  let query = $state("");
  let highlighted = $state("");

  // Each time the palette opens, it starts from the prefix that opened it.
  $effect(() => {
    if (open) {
      query = initialQuery;
    }
  });

  const VIEWS: { value: PaletteView; label: string }[] = [
    { value: "focus", label: "Focus" },
    { value: "kanban", label: "Kanban" },
    { value: "eisenhower", label: "Eisenhower" },
    { value: "database", label: "Database" },
    { value: "entry", label: "New Task" },
    { value: "settings", label: "Settings" },
  ];

  const ACTIONS: { value: PaletteAction; label: string }[] = [
    { value: "open_workspace_db", label: "Open workspace database…" },
    { value: "switch_global_db", label: "Switch to global database" },
    { value: "open_about", label: "About & third-party licenses" },
  ];

  let trimmed = $derived(query.trim());
  let mode: "all" | "user" | "project" = $derived(
    trimmed.startsWith("@")
      ? "user"
      : trimmed.startsWith("#")
        ? "project"
        : "all",
  );
  let needle = $derived(trimmed.replace(/^[@#]+/, "").toLowerCase());

  let users = $derived(
    [...new Set(session.known_users.map((u) => u.toLowerCase()))]
      .sort()
      .filter((u) => !needle || u.includes(needle)),
  );
  let projects = $derived(
    [...new Set(session.known_projects)]
      .sort()
      .filter((p) => !needle || p.toLowerCase().includes(needle)),
  );
  let views = $derived(
    VIEWS.filter((v) => !needle || v.label.toLowerCase().includes(needle)),
  );
  let actions = $derived(
    ACTIONS.filter((a) => !needle || a.label.toLowerCase().includes(needle)),
  );

  /** Offer to create a user or project that does not exist yet, when the typed name is valid. */
  let createUser = $derived.by(() => {
    if (mode === "project" || !needle) return null;
    const check = validateUsername(needle);
    if (check.error || !check.normalized) return null;
    return session.known_users.some((u) => u.toLowerCase() === check.normalized)
      ? null
      : check.normalized;
  });
  let createProject = $derived.by(() => {
    if (mode === "user" || !needle) return null;
    const exists = session.known_projects.some(
      (p) => p.toLowerCase() === needle,
    );
    return exists ? null : needle;
  });

  let showUsers = $derived(mode !== "project");
  let showProjects = $derived(mode !== "user");
  let showViewsAndActions = $derived(mode === "all");

  function finish() {
    open = false;
  }

  async function pickUser(user: string) {
    finish();
    await onSelectUser(user);
  }

  async function pickProject(project: string) {
    finish();
    await onSelectProject(project);
  }
</script>

<Dialog.Root bind:open>
  <Dialog.Portal>
    <Dialog.Overlay class="command-palette-overlay" />
    <Dialog.Content class="command-palette" interactOutsideBehavior="close">
      <Dialog.Title class="sr-only">Command palette</Dialog.Title>
      <Dialog.Description class="sr-only">
        Type @ for users, # for projects, or a view or action name. Use arrow
        keys to move and Enter to choose. Escape closes.
      </Dialog.Description>

      <Command.Root
        shouldFilter={false}
        loop
        bind:value={highlighted}
        class="command-palette-root"
      >
        <Command.Input
          bind:value={query}
          class="command-palette-input"
          placeholder="Search users (@), projects (#), views, and actions…"
          aria-label="Search users, projects, views, and actions"
          autocomplete="off"
        />

        <Command.List class="command-palette-list" aria-label="Results">
          <Command.Empty class="command-palette-empty">
            No matches.
          </Command.Empty>

          {#if showUsers && (users.length > 0 || createUser)}
            <Command.Group>
              <Command.GroupHeading class="command-palette-heading">
                Users
              </Command.GroupHeading>
              <Command.GroupItems>
                {#each users as user (user)}
                  <Command.Item
                    value={`user:${user}`}
                    class="command-palette-item"
                    onSelect={() => pickUser(user)}
                  >
                    <UserPill
                      {user}
                      kind={userKindOf(session.user_kinds, user)}
                    />
                    {#if user === session.active_user.toLowerCase()}
                      <span class="badge">Active</span>
                    {/if}
                  </Command.Item>
                {/each}
                {#if createUser}
                  <Command.Item
                    value={`create-user:${createUser}`}
                    class="command-palette-item"
                    onSelect={() => pickUser(createUser)}
                  >
                    <span
                      >Create &amp; switch to <strong>@{createUser}</strong
                      ></span
                    >
                    <span class="badge">New user</span>
                  </Command.Item>
                {/if}
              </Command.GroupItems>
            </Command.Group>
          {/if}

          {#if showProjects && (projects.length > 0 || createProject)}
            <Command.Group>
              <Command.GroupHeading class="command-palette-heading">
                Projects
              </Command.GroupHeading>
              <Command.GroupItems>
                {#each projects as project (project)}
                  <Command.Item
                    value={`project:${project}`}
                    class="command-palette-item"
                    onSelect={() => pickProject(project)}
                  >
                    <span>#{project}</span>
                    {#if project === session.active_project}
                      <span class="badge">Active</span>
                    {/if}
                  </Command.Item>
                {/each}
                {#if createProject}
                  <Command.Item
                    value={`create-project:${createProject}`}
                    class="command-palette-item"
                    onSelect={() => pickProject(createProject)}
                  >
                    <span
                      >Create &amp; switch to <strong>#{createProject}</strong
                      ></span
                    >
                    <span class="badge">New project</span>
                  </Command.Item>
                {/if}
              </Command.GroupItems>
            </Command.Group>
          {/if}

          {#if showViewsAndActions && views.length > 0}
            <Command.Group>
              <Command.GroupHeading class="command-palette-heading">
                Views
              </Command.GroupHeading>
              <Command.GroupItems>
                {#each views as view (view.value)}
                  <Command.Item
                    value={`view:${view.value}`}
                    class="command-palette-item"
                    onSelect={() => {
                      finish();
                      onSwitchView(view.value);
                    }}
                  >
                    {view.label}
                  </Command.Item>
                {/each}
              </Command.GroupItems>
            </Command.Group>
          {/if}

          {#if showViewsAndActions && actions.length > 0}
            <Command.Group>
              <Command.GroupHeading class="command-palette-heading">
                Actions
              </Command.GroupHeading>
              <Command.GroupItems>
                {#each actions as action (action.value)}
                  <Command.Item
                    value={`action:${action.value}`}
                    class="command-palette-item"
                    onSelect={() => {
                      finish();
                      onAction(action.value);
                    }}
                  >
                    {action.label}
                  </Command.Item>
                {/each}
              </Command.GroupItems>
            </Command.Group>
          {/if}
        </Command.List>
      </Command.Root>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>
