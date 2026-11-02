<script lang="ts">
  import { Slider } from "bits-ui";

  interface Props {
    label: string;
    value?: number;
    min?: number;
    max?: number;
    step?: number;
    id?: string;
    hint?: string;
  }

  let {
    label,
    value = $bindable(3),
    min = 1,
    max = 5,
    step = 1,
    id,
    hint,
  }: Props = $props();
</script>

<div class="form-field slider-field">
  <div class="slider-header">
    <span class="field-label" {id}>{label}</span>
    <span class="slider-value-badge" aria-live="polite">{value} / {max}</span>
  </div>
  <Slider.Root type="single" {min} {max} {step} bind:value class="slider-root">
    {#snippet children({ tickItems })}
      <span class="slider-track">
        <Slider.Range class="slider-range" />
      </span>
      {#each tickItems as { index, value: tickValue } (index)}
        <Slider.Tick {index} class="slider-tick" />
        <Slider.TickLabel {index} position="bottom" class="slider-tick-label">
          {tickValue}
        </Slider.TickLabel>
      {/each}
      <Slider.Thumb
        index={0}
        class="slider-thumb"
        aria-label={label}
        aria-labelledby={id}
      />
    {/snippet}
  </Slider.Root>
  {#if hint}
    <span class="field-hint">{hint}</span>
  {/if}
</div>
