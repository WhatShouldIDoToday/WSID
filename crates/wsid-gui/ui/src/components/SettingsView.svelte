<script lang="ts">
  import {
    type AppSessionSnapshot,
    type FocusPreset,
    type UiPreferences,
    PRESET_LABELS,
  } from "../lib/ipc";

  interface Props {
    session: AppSessionSnapshot;
    uiPrefs: UiPreferences;
    onUpdateUiPrefs: (prefs: UiPreferences) => void;
    onOpenUserModal: () => void;
    onPickWorkspaceDb: () => void | Promise<void>;
    onSwitchGlobalDb: () => void | Promise<void>;
    onSwitchRecentDb: (path: string) => void | Promise<void>;
    onPresetChange: (preset: FocusPreset) => void | Promise<void>;
    onFocusLimitChange: (limit: number) => void | Promise<void>;
    onOpenAbout: () => void;
  }

  let {
    session,
    uiPrefs,
    onUpdateUiPrefs,
    onOpenUserModal,
    onPickWorkspaceDb,
    onSwitchGlobalDb,
    onSwitchRecentDb,
    onPresetChange,
    onFocusLimitChange,
    onOpenAbout,
  }: Props = $props();

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
        <label for="settings-focus-preset" class="field-label">
          Default Focus Preset
        </label>
        <select
          id="settings-focus-preset"
          value={session.active_preset}
          onchange={(e) =>
            onPresetChange(
              (e.currentTarget as HTMLSelectElement).value as FocusPreset,
            )}
        >
          {#each Object.entries(PRESET_LABELS) as [key, label]}
            <option value={key}>{label}</option>
          {/each}
        </select>
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
    <section class="card" aria-labelledby="settings-a11y-heading">
      <h3 id="settings-a11y-heading" style="margin: 0 0 0.85rem 0;">
        Appearance & Accessibility
      </h3>

      <div class="form-field" style="margin-bottom: 1rem;">
        <label for="settings-theme-select" class="field-label">
          Color Scheme
        </label>
        <select
          id="settings-theme-select"
          value={uiPrefs.theme}
          onchange={(e) =>
            setTheme(
              (e.currentTarget as HTMLSelectElement)
                .value as UiPreferences["theme"],
            )}
        >
          <option value="system">System Default (prefers-color-scheme)</option>
          <option value="dark">Dark</option>
          <option value="light">Light</option>
        </select>
      </div>

      <div class="form-field" style="margin-bottom: 1rem;">
        <label for="settings-contrast-select" class="field-label">
          Contrast Level
        </label>
        <select
          id="settings-contrast-select"
          value={uiPrefs.contrast}
          onchange={(e) =>
            setContrast(
              (e.currentTarget as HTMLSelectElement)
                .value as UiPreferences["contrast"],
            )}
        >
          <option value="system">System Default (prefers-contrast)</option>
          <option value="more">High Contrast</option>
        </select>
      </div>

      <div class="form-field" style="margin-bottom: 1rem;">
        <label for="settings-palette-select" class="field-label">
          Status & Priority Color Palette
        </label>
        <select
          id="settings-palette-select"
          value={uiPrefs.palette}
          onchange={(e) =>
            setPalette(
              (e.currentTarget as HTMLSelectElement)
                .value as UiPreferences["palette"],
            )}
        >
          <option value="default">Standard Palette</option>
          <option value="okabe-ito">Okabe-Ito (Colorblind-Safe)</option>
        </select>
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
          <select
            id="settings-recent-db"
            value={session.is_global_db ? "" : session.db_path}
            onchange={(e) => {
              const target = e.currentTarget as HTMLSelectElement;
              if (target.value) {
                void onSwitchRecentDb(target.value);
              }
            }}
          >
            <option value="">Select a recent workspace database...</option>
            {#each session.recent_workspaces as ws}
              <option value={ws}>{ws}</option>
            {/each}
          </select>
        </div>
      {/if}
    </section>

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
