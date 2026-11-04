<script lang="ts">
  import { userKindMismatch, userOptionLabel } from "../lib/ipc";

  interface Props {
    id: string;
    value: string;
    knownUsers: string[];
    userKinds: Record<string, string>;
    placeholder?: string;
    ariaLabel?: string;
    disabled?: boolean;
    className?: string;
    /** Error from the parent (e.g. a failed validation); replaces the live warning. */
    error?: string | null;
    onInput?: () => void;
    onEnter?: (value: string) => void;
    onEscape?: () => void;
  }

  let {
    id,
    value = $bindable(""),
    knownUsers,
    userKinds,
    placeholder,
    ariaLabel,
    disabled = false,
    className = "",
    error = null,
    onInput,
    onEnter,
    onEscape,
  }: Props = $props();

  /** Live warning while typing: the typed kind does not match the existing user's kind. */
  let liveWarning = $derived(userKindMismatch(value, knownUsers, userKinds));
  let listId = $derived(`${id}-options`);

  function handleKeydown(e: KeyboardEvent) {
    if (e.key === "Enter" && onEnter) {
      e.preventDefault();
      onEnter(value);
    } else if (e.key === "Escape" && onEscape) {
      e.preventDefault();
      e.stopPropagation();
      onEscape();
    }
  }
</script>

<div class="user-ref">
  <input
    {id}
    type="text"
    class={className}
    list={listId}
    {placeholder}
    {disabled}
    aria-label={ariaLabel}
    aria-invalid={error ? "true" : undefined}
    autocomplete="off"
    autocorrect="off"
    autocapitalize="off"
    spellcheck="false"
    bind:value
    oninput={() => onInput?.()}
    onkeydown={handleKeydown}
  />
  <datalist id={listId}>
    {#each knownUsers as u (u)}
      <option value={u} label={userOptionLabel(u, userKinds)}>
        {userOptionLabel(u, userKinds)}
      </option>
    {/each}
  </datalist>
  {#if error}
    <span class="field-error" role="alert">{error}</span>
  {:else if liveWarning}
    <span class="user-ref-warning">{liveWarning}</span>
  {/if}
</div>
