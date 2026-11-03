<script lang="ts">
  import {
    RELATION_INPUT_KINDS,
    RELATION_LABELS,
    type RelationInputKind,
  } from "../lib/ipc";
  import Select from "./ui/Select.svelte";

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

  const options = RELATION_INPUT_KINDS.map((kind) => ({
    value: kind,
    label: RELATION_LABELS[kind],
  }));

  async function handleChange(next: string) {
    const kind = next as RelationInputKind;
    if (kind === value) return;
    pending = kind;
    try {
      await onChange(kind);
    } finally {
      pending = null;
    }
  }
</script>

<Select
  class="relation-type-select"
  ariaLabel={label}
  {disabled}
  value={current}
  {options}
  size="sm"
  onValueChange={handleChange}
/>
