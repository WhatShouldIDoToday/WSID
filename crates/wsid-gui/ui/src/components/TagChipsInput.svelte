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
    placeholder = "Type a tag and press Enter or comma...",
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
    if (e.key === "Enter" || e.key === "," || e.key === " ") {
      if (inputText.trim().length > 0) {
        e.preventDefault();
        commitToken(inputText);
        inputText = "";
      } else if (e.key === "," || e.key === " ") {
        e.preventDefault();
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

<div class="form-field">
  <label for={id} class="field-label">{label}</label>
  <div class="tag-chips-box">
    {#each tags as tag (tag)}
      <span class="tag-chip">
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
    <input
      {id}
      type="text"
      class="tag-chips-input"
      bind:value={inputText}
      onkeydown={handleKeyDown}
      oninput={handleInput}
      onblur={handleBlur}
      list={`${id}-datalist`}
      placeholder={tags.length === 0 ? placeholder : "Add tag..."}
    />
  </div>
  <datalist id={`${id}-datalist`}>
    {#each availableSuggestions as tag}
      <option value={tag}></option>
    {/each}
  </datalist>
</div>
