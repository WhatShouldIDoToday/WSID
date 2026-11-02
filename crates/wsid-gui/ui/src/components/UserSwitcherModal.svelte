<script lang="ts">
  import { onMount } from "svelte";

  interface Props {
    activeUser: string;
    knownUsers: string[];
    onSelectUser: (user: string) => void | Promise<void>;
    onClose: () => void;
  }

  let { activeUser, knownUsers, onSelectUser, onClose }: Props = $props();

  let filterText = $state("");
  let highlightedIndex = $state<number>(0);
  let inputEl = $state<HTMLInputElement | null>(null);

  let allUsers = $derived.by(() => {
    const set = new Set<string>();
    if (activeUser) set.add(activeUser.toLowerCase());
    for (const u of knownUsers) {
      if (u.trim()) set.add(u.trim().replace(/^@/, "").toLowerCase());
    }
    return Array.from(set).sort();
  });

  let filteredUsers = $derived.by(() => {
    const q = filterText.trim().replace(/^@/, "").toLowerCase();
    if (!q) return allUsers;
    return allUsers.filter((u) => u.includes(q));
  });

  let normalizedTyped = $derived(
    filterText.trim().replace(/^@/, "").toLowerCase(),
  );

  onMount(() => {
    inputEl?.focus();
  });

  async function chooseUser(raw: string) {
    const clean = raw.trim().replace(/^@/, "").toLowerCase();
    if (!clean) return;
    await onSelectUser(clean);
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
      if (normalizedTyped) {
        if (
          filteredUsers.length > 0 &&
          highlightedIndex >= 0 &&
          highlightedIndex < filteredUsers.length &&
          filteredUsers[highlightedIndex] === normalizedTyped
        ) {
          void chooseUser(filteredUsers[highlightedIndex]);
        } else {
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
      placeholder="Type a user name and press Enter..."
      aria-label="Filter users or enter a new user name"
      bind:value={filterText}
      oninput={() => {
        highlightedIndex = 0;
      }}
    />

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
            {#if user === activeUser}
              <span class="badge">Active</span>
            {/if}
          </button>
        </li>
      {/each}

      {#if normalizedTyped && !allUsers.includes(normalizedTyped)}
        <li role="option" aria-selected="false">
          <button
            type="button"
            class="user-modal-option is-highlighted"
            onclick={() => chooseUser(normalizedTyped)}
          >
            <span>Switch to <strong>@{normalizedTyped}</strong></span>
            <span class="badge">Enter ↵</span>
          </button>
        </li>
      {/if}
    </ul>
  </div>
</div>
