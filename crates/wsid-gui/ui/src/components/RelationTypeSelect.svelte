<script lang="ts">
  import {
    RELATION_INPUT_KINDS,
    RELATION_LABELS,
    type RelationInputKind,
  } from "../lib/ipc";

  interface Props {
    value: RelationInputKind;
    label: string;
    disabled?: boolean;
    onChange: (kind: RelationInputKind) => void | Promise<void>;
  }

  let { value, label, disabled = false, onChange }: Props = $props();

  /** Optimistic selection; cleared after `onChange` so failures snap back to `value`. */
  let pending = $state<RelationInputKind | null>(null);
  let current = $derived(pending ?? value);

  async function handleChange(e: Event) {
    const next = (e.currentTarget as HTMLSelectElement)
      .value as RelationInputKind;
    if (next === value) return;
    pending = next;
    try {
      await onChange(next);
    } finally {
      pending = null;
    }
  }
</script>

<select
  class="relation-type-select"
  aria-label={label}
  {disabled}
  value={current}
  onchange={handleChange}
>
  {#each RELATION_INPUT_KINDS as kind (kind)}
    <option value={kind}>{RELATION_LABELS[kind]}</option>
  {/each}
</select>
