<script lang="ts">
  import { onMount } from "svelte";
  import { validateUsername } from "../lib/ipc";

  interface Props {
    activeUser: string;
    primaryUser?: string;
    knownUsers: string[];
    onSelectUser: (user: string) => void | Promise<void>;
    onClose: () => void;
  }

  let {
    activeUser,
    primaryUser = "",
    knownUsers,
    onSelectUser,
    onClose,
  }: Props = $props();

  let filterText = $state("");
  let highlightedIndex = $state<number>(0);
  let inputEl = $state<HTMLInputElement | null>(null);
  let validationError = $state<string | null>(null);

  let allUsers = $derived.by(() => {
    const set = new Set<string>();
    if (activeUser) set.add(activeUser.toLowerCase());
    if (primaryUser) set.add(primaryUser.toLowerCase());
    for (const u of knownUsers) {
      const check = validateUsername(u);
      if (check.normalized) set.add(check.normalized);
    }
    return Array.from(set).sort();
  });

  let filteredUsers = $derived.by(() => {
    const q = filterText.trim().replace(/^@+/, "").toLowerCase();
    if (!q) return allUsers;
    return allUsers.filter((u) => u.includes(q));
  });

  let typedValidation = $derived.by(() => {
    const raw = filterText.trim();
    if (!raw) return { normalized: null, error: null };
    return validateUsername(raw);
  });

  let normalizedTyped = $derived(typedValidation.normalized ?? "");

  onMount(() => {
    inputEl?.focus();
  });

  async function chooseUser(raw: string) {
    const check = validateUsername(raw);
    if (check.error || !check.normalized) {
      validationError = check.error;
      return;
    }
    validationError = null;
    await onSelectUser(check.normalized);
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
      if (filteredUsers.length > 0) {
        highlightedIndex = (highlightedIndex + 1) % filteredUsers.length;
      }
      return;
    }
    if (e.key === "ArrowUp") {
      e.preventDefault();
      if (filteredUsers.length > 0) {
        highlightedIndex =
          (highlightedIndex - 1 + filteredUsers.length) % filteredUsers.length;
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
          filteredUsers.length > 0 &&
          highlightedIndex >= 0 &&
          highlightedIndex < filteredUsers.length &&
          filteredUsers[highlightedIndex] === normalizedTyped
        ) {
          void chooseUser(filteredUsers[highlightedIndex]);
        } else if (normalizedTyped) {
          void chooseUser(normalizedTyped);
        }
      } else if (
        filteredUsers.length > 0 &&
        highlightedIndex >= 0 &&
        highlightedIndex < filteredUsers.length
      ) {
        void chooseUser(filteredUsers[highlightedIndex]);
      }
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_element_interactions -->
<div
  class="user-modal-backdrop"
  role="dialog"
  aria-modal="true"
  aria-labelledby="user-switcher-title"
  tabindex="-1"
  onkeydown={handleKeyDown}
  onclick={(e) => {
    if (e.target === e.currentTarget) onClose();
  }}
>
  <div class="user-modal-card">
    <div class="user-modal-header">
      <h2 id="user-switcher-title">Switch Active User</h2>
      <button
        type="button"
        class="btn-sm"
        onclick={onClose}
        aria-label="Close user switcher"
      >
        Esc
      </button>
    </div>

    <input
      bind:this={inputEl}
      type="text"
      class="user-modal-input"
      placeholder="Type a user name (letters, numbers, _, -) and press Enter..."
      aria-label="Filter users or enter a new user name"
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

    <ul class="user-modal-list" role="listbox" aria-label="Known users">
      {#each filteredUsers as user, idx (user)}
        <li role="option" aria-selected={user === activeUser}>
          <button
            type="button"
            class="user-modal-option"
            class:is-highlighted={idx === highlightedIndex}
            class:is-current={user === activeUser}
            onclick={() => chooseUser(user)}
          >
            <span>@{user}</span>
            <span style="display: inline-flex; gap: 0.35rem;">
              {#if primaryUser && user === primaryUser.toLowerCase()}
                <span class="badge">Main</span>
              {/if}
              {#if user === activeUser}
                <span class="badge">Active</span>
              {/if}
            </span>
          </button>
        </li>
      {/each}

      {#if normalizedTyped && !typedValidation.error && !allUsers.includes(normalizedTyped)}
        <li role="option" aria-selected="false">
          <button
            type="button"
            class="user-modal-option is-highlighted"
            onclick={() => chooseUser(normalizedTyped)}
          >
            <span>Create & switch to <strong>@{normalizedTyped}</strong></span>
            <span class="badge">New User · Enter ↵</span>
          </button>
        </li>
      {/if}
    </ul>
  </div>
</div>
