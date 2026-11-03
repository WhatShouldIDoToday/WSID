<script lang="ts">
  import { onMount } from "svelte";
  import { DEFAULT_PROJECT, validateProject } from "../lib/ipc";

  interface Props {
    activeProject: string;
    knownProjects: string[];
    onSelectProject: (project: string) => void | Promise<void>;
    onClose: () => void;
  }

  let { activeProject, knownProjects, onSelectProject, onClose }: Props =
    $props();

  let filterText = $state("");
  let highlightedIndex = $state<number>(0);
  let inputEl = $state<HTMLInputElement | null>(null);
  let validationError = $state<string | null>(null);

  let allProjects = $derived.by(() => {
    const set = new Set<string>([DEFAULT_PROJECT]);
    if (activeProject) set.add(activeProject.toLowerCase());
    for (const p of knownProjects) {
      const check = validateProject(p);
      if (check.normalized) set.add(check.normalized);
    }
    return Array.from(set).sort((a, b) => {
      if (a === DEFAULT_PROJECT) return -1;
      if (b === DEFAULT_PROJECT) return 1;
      return a.localeCompare(b);
    });
  });

  let filteredProjects = $derived.by(() => {
    const q = filterText.trim().replace(/^#+/, "").toLowerCase();
    if (!q) return allProjects;
    return allProjects.filter((p) => p.includes(q));
  });

  let typedValidation = $derived.by(() => {
    const raw = filterText.trim();
    if (!raw) return { normalized: null, error: null };
    return validateProject(raw);
  });

  let normalizedTyped = $derived(typedValidation.normalized ?? "");

  onMount(() => {
    inputEl?.focus();
  });

  async function chooseProject(raw: string) {
    const check = validateProject(raw);
    if (check.error || !check.normalized) {
      validationError = check.error;
      return;
    }
    validationError = null;
    await onSelectProject(check.normalized);
    onClose();
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.preventDefault();
      onClose();
      return;
    }
    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (filteredProjects.length > 0) {
        highlightedIndex = (highlightedIndex + 1) % filteredProjects.length;
      }
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      if (filteredProjects.length > 0) {
        highlightedIndex =
          (highlightedIndex - 1 + filteredProjects.length) %
          filteredProjects.length;
      }
      return;
    }
    if (e.key === "Enter") {
      e.preventDefault();
      if (filterText.trim()) {
        if (typedValidation.error) {
          validationError = typedValidation.error;
          return;
        }
        if (
          filteredProjects.length > 0 &&
          highlightedIndex >= 0 &&
          highlightedIndex < filteredProjects.length &&
          filteredProjects[highlightedIndex] === normalizedTyped
        ) {
          void chooseProject(filteredProjects[highlightedIndex]);
        } else if (normalizedTyped) {
          void chooseProject(normalizedTyped);
        }
      } else if (
        filteredProjects.length > 0 &&
        highlightedIndex >= 0 &&
        highlightedIndex < filteredProjects.length
      ) {
        void chooseProject(filteredProjects[highlightedIndex]);
      }
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="switcher-modal-backdrop"
  role="dialog"
  aria-modal="true"
  aria-labelledby="project-switcher-title"
  tabindex="-1"
  onkeydown={handleKeyDown}
  onclick={(e) => {
    if (e.target === e.currentTarget) onClose();
  }}
>
  <div class="switcher-modal-card">
    <div class="switcher-modal-header">
      <h2 id="project-switcher-title">Switch Active Project</h2>
      <button
        type="button"
        class="btn-sm"
        onclick={onClose}
        aria-label="Close project switcher"
      >
        Esc
      </button>
    </div>

    <input
      bind:this={inputEl}
      type="text"
      class="switcher-modal-input"
      placeholder="Type a project name (letters, numbers, _, -) and press Enter..."
      aria-label="Filter projects or enter a new project name"
      aria-invalid={typedValidation.error || validationError ? "true" : "false"}
      bind:value={filterText}
      oninput={() => {
        highlightedIndex = 0;
        validationError = null;
      }}
    />

    {#if typedValidation.error || validationError}
      <div class="field-error" role="alert">
        {typedValidation.error ?? validationError}
      </div>
    {/if}

    <ul class="switcher-modal-list" role="listbox" aria-label="Known projects">
      {#each filteredProjects as project, idx (project)}
        <li role="option" aria-selected={project === activeProject}>
          <button
            type="button"
            class="switcher-modal-option"
            class:is-highlighted={idx === highlightedIndex}
            class:is-current={project === activeProject}
            onclick={() => chooseProject(project)}
          >
            <span>#{project}</span>
            <span style="display: inline-flex; gap: 0.35rem;">
              {#if project === DEFAULT_PROJECT}
                <span class="badge">Default</span>
              {/if}
              {#if project === activeProject}
                <span class="badge">Active</span>
              {/if}
            </span>
          </button>
        </li>
      {/each}

      {#if normalizedTyped && !typedValidation.error && !allProjects.includes(normalizedTyped)}
        <li role="option" aria-selected="false">
          <button
            type="button"
            class="switcher-modal-option is-highlighted"
            onclick={() => chooseProject(normalizedTyped)}
          >
            <span>Create & switch to <strong>#{normalizedTyped}</strong></span>
            <span class="badge">New Project · Enter ↵</span>
          </button>
        </li>
      {/if}
    </ul>
  </div>
</div>
