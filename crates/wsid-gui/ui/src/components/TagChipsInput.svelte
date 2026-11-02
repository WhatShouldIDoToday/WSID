<script lang="ts">
  interface Props {
    tags?: string[];
    knownTags?: string[];
    inputText?: string;
    id?: string;
    label?: string;
    placeholder?: string;
  }

  let {
    tags = $bindable<string[]>([]),
    knownTags = [],
    inputText = $bindable(""),
    id = "tags-chip-input",
    label = "Tags",
    placeholder = "Add tag...",
  }: Props = $props();

  let tagError = $state<string | null>(null);

  const VALID_TAG_REGEX = /^[\p{L}\p{N}_-]+$/u;

  function isValidTagToken(token: string): boolean {
    return VALID_TAG_REGEX.test(token);
  }

  function commitToken(raw: string): boolean {
    const pieces = raw
      .split(/[,\s]+/)
      .map((s) => s.trim().toLowerCase())
      .filter((s) => s.length > 0);
    if (pieces.length === 0) return true;

    const next = [...tags];
    const invalidTokens: string[] = [];

    for (const p of pieces) {
      if (!isValidTagToken(p)) {
        invalidTokens.push(p);
        continue;
      }
      if (!next.includes(p)) {
        next.push(p);
      }
    }
    tags = next;

    if (invalidTokens.length > 0) {
      tagError = `Invalid tag '${invalidTokens[0]}': tags must contain only letters, digits, '-', or '_'.`;
      inputText = invalidTokens.join(" ");
      return false;
    }

    tagError = null;
    return true;
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
      e.preventDefault();
      if (inputText.trim().length > 0) {
        if (commitToken(inputText)) {
          inputText = "";
        }
      }
      return;
    }
    if (e.key === "," || e.key === " ") {
      e.preventDefault();
      if (inputText.trim().length > 0) {
        if (commitToken(inputText)) {
          inputText = "";
        }
      }
    } else if (
      e.key === "Backspace" &&
      inputText.length === 0 &&
      tags.length > 0
    ) {
      e.preventDefault();
      tags = tags.slice(0, -1);
      tagError = null;
    }
  }

  function handleInput(e: Event) {
    const target = e.currentTarget as HTMLInputElement;
    const val = target.value;
    if (val.trim().length === 0) {
      tagError = null;
      return;
    }
    if (val.includes(",") || val.includes(" ")) {
      if (commitToken(val)) {
        inputText = "";
      }
    } else if (isValidTagToken(val.trim())) {
      tagError = null;
    }
  }

  function handleBlur() {
    if (inputText.trim().length > 0) {
      if (commitToken(inputText)) {
        inputText = "";
      }
    } else {
      tagError = null;
    }
  }

  function removeTag(tagToRemove: string) {
    tags = tags.filter((t) => t !== tagToRemove);
  }

  let availableSuggestions = $derived(
    knownTags.filter((t) => !tags.includes(t.toLowerCase())),
  );
</script>

<div class="form-field tag-field-stack">
  <label for={id} class="field-label">{label}</label>
  <input
    {id}
    type="text"
    class="tag-compact-input"
    bind:value={inputText}
    onkeydown={handleKeyDown}
    oninput={handleInput}
    onblur={handleBlur}
    list={`${id}-datalist`}
    aria-invalid={tagError ? "true" : undefined}
    aria-describedby={tagError ? `${id}-error` : undefined}
    {placeholder}
  />
  {#if tagError}
    <span id={`${id}-error`} class="field-error" role="alert">{tagError}</span>
  {/if}
  {#if tags.length > 0}
    <div class="tag-chips-below" role="list" aria-label="Selected tags">
      {#each tags as tag (tag)}
        <span class="tag-chip" role="listitem">
          <span>{tag}</span>
          <button
            type="button"
            class="tag-chip-remove"
            onclick={() => removeTag(tag)}
            aria-label={`Remove tag ${tag}`}
            title={`Remove tag ${tag}`}
          >
            ×
          </button>
        </span>
      {/each}
    </div>
  {/if}
  <datalist id={`${id}-datalist`}>
    {#each availableSuggestions as tag}
      <option value={tag}></option>
    {/each}
  </datalist>
</div>
