<script lang="ts">
  interface Props {
    tags?: string[];
    knownTags?: string[];
    id?: string;
    label?: string;
    placeholder?: string;
  }

  let {
    tags = $bindable<string[]>([]),
    knownTags = [],
    id = "tags-chip-input",
    label = "Tags",
    placeholder = "Add tag...",
  }: Props = $props();

  let inputText = $state("");

  function commitToken(raw: string) {
    const pieces = raw
      .split(/[,\s]+/)
      .map((s) => s.trim().toLowerCase())
      .filter((s) => s.length > 0);
    if (pieces.length === 0) return;

    const next = [...tags];
    for (const p of pieces) {
      if (!next.includes(p)) {
        next.push(p);
      }
    }
    tags = next;
  }

  function handleKeyDown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
      e.preventDefault();
      if (inputText.trim().length > 0) {
        commitToken(inputText);
        inputText = "";
      }
      return;
    }
    if (e.key === "," || e.key === " ") {
      e.preventDefault();
      if (inputText.trim().length > 0) {
        commitToken(inputText);
        inputText = "";
      }
    } else if (
      e.key === "Backspace" &&
      inputText.length === 0 &&
      tags.length > 0
    ) {
      e.preventDefault();
      tags = tags.slice(0, -1);
    }
  }

  function handleInput(e: Event) {
    const target = e.currentTarget as HTMLInputElement;
    const val = target.value;
    if (val.includes(",") || val.includes(" ")) {
      commitToken(val);
      inputText = "";
    }
  }

  function handleBlur() {
    if (inputText.trim().length > 0) {
      commitToken(inputText);
      inputText = "";
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
    {placeholder}
  />
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
