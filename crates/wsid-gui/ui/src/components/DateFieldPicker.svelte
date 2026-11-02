<script lang="ts">
  import { DatePicker } from "bits-ui";
  import { parseDate, type DateValue } from "@internationalized/date";

  interface Props {
    label: string;
    value?: string;
    id?: string;
    invalid?: boolean;
    describedBy?: string;
  }

  let {
    label,
    value = $bindable(""),
    id,
    invalid = false,
    describedBy,
  }: Props = $props();

  function safeParse(raw: string): DateValue | undefined {
    const trimmed = raw.trim();
    if (!trimmed) return undefined;
    try {
      return parseDate(trimmed);
    } catch {
      return undefined;
    }
  }

  let pickerValue = $state<DateValue | undefined>(safeParse(value));

  $effect(() => {
    const parsed = safeParse(value);
    const currentStr = pickerValue ? pickerValue.toString() : "";
    const nextStr = parsed ? parsed.toString() : "";
    if (currentStr !== nextStr) {
      pickerValue = parsed;
    }
  });

  function handleValueChange(next: DateValue | undefined) {
    pickerValue = next;
    value = next ? next.toString() : "";
  }

  function clearDate() {
    pickerValue = undefined;
    value = "";
  }
</script>

<DatePicker.Root
  value={pickerValue}
  onValueChange={handleValueChange}
  granularity="day"
>
  <div class="date-picker-field">
    <DatePicker.Label class="field-label">{label}</DatePicker.Label>
    <div
      class="date-picker-control"
      data-invalid={invalid ? "true" : undefined}
    >
      <DatePicker.Input
        {id}
        aria-invalid={invalid ? "true" : undefined}
        aria-describedby={describedBy}
        class="date-picker-segments"
      >
        {#snippet children({ segments })}
          {#each segments as { part, value: segVal }}
            {#if part === "literal"}
              <DatePicker.Segment {part} class="date-segment-literal">
                {segVal}
              </DatePicker.Segment>
            {:else}
              <DatePicker.Segment {part} class="date-segment">
                {segVal}
              </DatePicker.Segment>
            {/if}
          {/each}
        {/snippet}
      </DatePicker.Input>

      {#if value}
        <button
          type="button"
          class="date-clear-btn"
          onclick={clearDate}
          aria-label={`Clear ${label}`}
          title={`Clear ${label}`}
        >
          ×
        </button>
      {/if}

      <DatePicker.Trigger
        class="date-trigger-btn"
        aria-label={`Open calendar for ${label}`}
        title="Open calendar"
      >
        Calendar
      </DatePicker.Trigger>
    </div>

    <DatePicker.Portal>
      <DatePicker.Content class="date-popover" sideOffset={6}>
        <DatePicker.Calendar class="date-calendar">
          {#snippet children({ months, weekdays })}
            <DatePicker.Header class="date-calendar-header">
              <DatePicker.PrevButton class="btn-sm" aria-label="Previous month">
                ‹
              </DatePicker.PrevButton>
              <DatePicker.Heading class="date-calendar-heading" />
              <DatePicker.NextButton class="btn-sm" aria-label="Next month">
                ›
              </DatePicker.NextButton>
            </DatePicker.Header>

            {#each months as month}
              <DatePicker.Grid class="date-calendar-grid">
                <DatePicker.GridHead>
                  <DatePicker.GridRow class="date-calendar-row">
                    {#each weekdays as day}
                      <DatePicker.HeadCell class="date-calendar-weekday">
                        {day.slice(0, 2)}
                      </DatePicker.HeadCell>
                    {/each}
                  </DatePicker.GridRow>
                </DatePicker.GridHead>
                <DatePicker.GridBody>
                  {#each month.weeks as weekDates}
                    <DatePicker.GridRow class="date-calendar-row">
                      {#each weekDates as date}
                        <DatePicker.Cell
                          {date}
                          month={month.value}
                          class="date-calendar-cell"
                        >
                          <DatePicker.Day class="date-calendar-day">
                            {date.day}
                          </DatePicker.Day>
                        </DatePicker.Cell>
                      {/each}
                    </DatePicker.GridRow>
                  {/each}
                </DatePicker.GridBody>
              </DatePicker.Grid>
            {/each}
          {/snippet}
        </DatePicker.Calendar>
      </DatePicker.Content>
    </DatePicker.Portal>
  </div>
</DatePicker.Root>
