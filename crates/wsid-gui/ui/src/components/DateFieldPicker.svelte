<script lang="ts">
  import { DatePicker, TimeField } from "bits-ui";
  import {
    Time,
    getLocalTimeZone,
    parseAbsoluteToLocal,
    parseDate,
    parseDateTime,
    toCalendarDate,
    toCalendarDateTime,
    toTime,
    toZoned,
    type DateValue,
  } from "@internationalized/date";

  interface Props {
    label: string;
    value?: string;
    id?: string;
    invalid?: boolean;
    describedBy?: string;
    onBlur?: () => void;
  }

  let {
    label,
    value = $bindable(""),
    id,
    invalid = false,
    describedBy,
    onBlur,
  }: Props = $props();

  const localTz = getLocalTimeZone();
  const shortTzName = (() => {
    try {
      const parts = new Intl.DateTimeFormat(undefined, {
        timeZone: localTz,
        timeZoneName: "short",
      }).formatToParts(new Date());
      return parts.find((p) => p.type === "timeZoneName")?.value ?? localTz;
    } catch {
      return localTz;
    }
  })();

  const DEFAULT_TIME = new Time(20, 0);

  function parseIsoValue(raw: string): {
    date: DateValue | undefined;
    time: Time | undefined;
  } {
    const trimmed = raw.trim();
    if (!trimmed) {
      return { date: undefined, time: undefined };
    }
    if (/^\d{4}-\d{2}-\d{2}$/.test(trimmed)) {
      try {
        return { date: parseDate(trimmed), time: new Time(20, 0) };
      } catch {
        return { date: undefined, time: undefined };
      }
    }
    try {
      const cleaned = trimmed.replace(/\[.*\]$/, "");
      if (/[Zz]|[+-]\d{2}:\d{2}$/.test(cleaned)) {
        const zoned = parseAbsoluteToLocal(cleaned);
        return {
          date: toCalendarDate(zoned),
          time: new Time(zoned.hour, zoned.minute),
        };
      }
      const dt = parseDateTime(cleaned);
      const t = toTime(dt);
      return {
        date: toCalendarDate(dt),
        time: new Time(t.hour, t.minute),
      };
    } catch {
      try {
        return {
          date: parseDate(trimmed.slice(0, 10)),
          time: new Time(20, 0),
        };
      } catch {
        return { date: undefined, time: undefined };
      }
    }
  }

  function formatCombinedIso(
    dateVal: DateValue | undefined,
    timeVal: Time | undefined,
  ): string {
    if (!dateVal) return "";
    const calDate = toCalendarDate(dateVal);
    const effectiveTime = timeVal ?? DEFAULT_TIME;
    const dt = toCalendarDateTime(calDate, effectiveTime);
    const zoned = toZoned(dt, localTz);
    return zoned.toString().replace(/\[.*\]$/, "");
  }

  const initial = parseIsoValue(value);
  let pickerDate = $state<DateValue | undefined>(initial.date);
  let pickerTime = $state<Time | undefined>(initial.time);

  $effect(() => {
    const currentFormatted = formatCombinedIso(pickerDate, pickerTime);
    if (value !== currentFormatted) {
      const parsed = parseIsoValue(value);
      pickerDate = parsed.date;
      pickerTime = parsed.time;
    }
  });

  function handleDateChange(nextDate: DateValue | undefined) {
    pickerDate = nextDate;
    if (nextDate && !pickerTime) {
      pickerTime = new Time(20, 0);
    } else if (!nextDate) {
      pickerTime = undefined;
    }
    value = formatCombinedIso(pickerDate, pickerTime);
  }

  function handleTimeChange(nextTime: Time | undefined) {
    pickerTime = nextTime;
    if (pickerDate) {
      value = formatCombinedIso(pickerDate, pickerTime);
    }
  }

  function clearDateTime() {
    pickerDate = undefined;
    pickerTime = undefined;
    value = "";
    onBlur?.();
  }

  function handleContainerFocusOut(e: FocusEvent) {
    const currentTarget = e.currentTarget as HTMLElement | null;
    const related = e.relatedTarget as Node | null;
    if (currentTarget && related && currentTarget.contains(related)) {
      return;
    }
    onBlur?.();
  }
</script>

<div class="date-picker-field" onfocusout={handleContainerFocusOut}>
  <DatePicker.Root
    value={pickerDate}
    onValueChange={handleDateChange}
    granularity="day"
  >
    <div class="date-picker-label-row">
      <DatePicker.Label class="field-label">{label}</DatePicker.Label>
      <span
        class="tz-badge"
        title={`Detected local timezone: ${localTz} (defaults to 8:00 PM when time is unset)`}
      >
        {shortTzName} ({localTz})
      </span>
    </div>

    <div class="date-picker-row" data-invalid={invalid ? "true" : undefined}>
      <DatePicker.Input
        {id}
        aria-invalid={invalid ? "true" : undefined}
        aria-describedby={describedBy}
        class="date-picker-input"
      >
        {#snippet children({ segments })}
          {#each segments as { part, value: segVal }, i (part + i)}
            <DatePicker.Segment {part} class="date-picker-segment">
              {segVal}
            </DatePicker.Segment>
          {/each}
        {/snippet}
      </DatePicker.Input>

      <TimeField.Root
        value={pickerTime}
        onValueChange={handleTimeChange}
        placeholder={DEFAULT_TIME}
        granularity="minute"
      >
        <TimeField.Input
          aria-label={`${label} time (${shortTzName})`}
          aria-invalid={invalid ? "true" : undefined}
          class="date-picker-input time-field-input"
        >
          {#snippet children({ segments })}
            {#each segments as { part, value: segVal }, i (part + i)}
              <TimeField.Segment {part} class="date-picker-segment">
                {segVal}
              </TimeField.Segment>
            {/each}
          {/snippet}
        </TimeField.Input>
      </TimeField.Root>

      <DatePicker.Trigger
        class="date-icon-btn"
        aria-label="Calendar"
        title="Calendar"
      >
        <svg
          aria-hidden="true"
          width="16"
          height="16"
          viewBox="0 0 24 24"
          fill="none"
          stroke="currentColor"
          stroke-width="2"
          stroke-linecap="round"
          stroke-linejoin="round"
        >
          <rect x="3" y="4" width="18" height="18" rx="2" ry="2" />
          <line x1="16" y1="2" x2="16" y2="6" />
          <line x1="8" y1="2" x2="8" y2="6" />
          <line x1="3" y1="10" x2="21" y2="10" />
        </svg>
      </DatePicker.Trigger>

      {#if value}
        <button
          type="button"
          class="date-clear-btn"
          onclick={clearDateTime}
          aria-label={`Clear ${label}`}
          title={`Clear ${label}`}
        >
          ×
        </button>
      {/if}
    </div>

    <DatePicker.Portal>
      <DatePicker.Content class="date-picker-popover" sideOffset={6}>
        <DatePicker.Calendar>
          {#snippet children({ months, weekdays })}
            <DatePicker.Header class="date-picker-header">
              <DatePicker.PrevButton
                class="date-picker-nav-btn"
                aria-label="Previous month"
              >
                ‹
              </DatePicker.PrevButton>
              <DatePicker.Heading class="date-picker-heading" />
              <DatePicker.NextButton
                class="date-picker-nav-btn"
                aria-label="Next month"
              >
                ›
              </DatePicker.NextButton>
            </DatePicker.Header>

            {#each months as month}
              <DatePicker.Grid class="date-picker-grid">
                <DatePicker.GridHead>
                  <DatePicker.GridRow>
                    {#each weekdays as day}
                      <DatePicker.HeadCell class="date-picker-head-cell">
                        {day.slice(0, 2)}
                      </DatePicker.HeadCell>
                    {/each}
                  </DatePicker.GridRow>
                </DatePicker.GridHead>
                <DatePicker.GridBody>
                  {#each month.weeks as weekDates}
                    <DatePicker.GridRow>
                      {#each weekDates as date}
                        <DatePicker.Cell
                          {date}
                          month={month.value}
                          class="date-picker-cell"
                        >
                          <DatePicker.Day class="date-picker-day">
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
  </DatePicker.Root>
</div>
