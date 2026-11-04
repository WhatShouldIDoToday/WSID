<script lang="ts">
  import { Select } from "bits-ui";

  export interface SelectOption {
    value: string;
    label: string;
    disabled?: boolean;
  }

  interface Props {
    /** Applied to the trigger button so an external `<label for>` can target it. */
    id?: string;
    /** Accessible name when no visible `<label for>` exists. */
    ariaLabel?: string;
    value: string;
    options: SelectOption[];
    placeholder?: string;
    disabled?: boolean;
    size?: "sm" | "md";
    /** Extra classes for the trigger (e.g. a view-specific width). */
    class?: string;
    title?: string;
    onValueChange: (value: string) => void;
  }

  let {
    id,
    ariaLabel,
    value,
    options,
    placeholder = "",
    disabled = false,
    size = "md",
    class: className = "",
    title,
    onValueChange,
  }: Props = $props();

  let selectedLabel = $derived(
    options.find((option) => option.value === value)?.label ?? placeholder,
  );
</script>

<Select.Root
  type="single"
  {value}
  items={options}
  {disabled}
  onValueChange={(next) => onValueChange(next)}
>
  <Select.Trigger
    {id}
    aria-label={ariaLabel}
    {title}
    class={`ui-select-trigger ui-select-${size} ${className}`}
    data-select-trigger
  >
    <span class="ui-select-value">{selectedLabel}</span>
    <span class="ui-select-chevron" aria-hidden="true">▾</span>
  </Select.Trigger>
  <Select.Portal>
    <Select.Content
      class="ui-select-content"
      sideOffset={4}
      data-select-content
    >
      <Select.Viewport class="ui-select-viewport">
        {#each options as option (option.value)}
          <Select.Item
            value={option.value}
            label={option.label}
            disabled={option.disabled}
            class="ui-select-item"
          >
            {#snippet children({ selected })}
              <span class="ui-select-check" aria-hidden="true"
                >{selected ? "✓" : ""}</span
              >
              <span>{option.label}</span>
            {/snippet}
          </Select.Item>
        {/each}
      </Select.Viewport>
    </Select.Content>
  </Select.Portal>
</Select.Root>
