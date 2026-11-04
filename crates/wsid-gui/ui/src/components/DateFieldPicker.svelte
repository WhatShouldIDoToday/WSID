<script lang="ts">
  import { untrack } from "svelte";
  import { DatePicker, TimeField } from "bits-ui";
  import {
    CalendarDate,
    Time,
    getLocalTimeZone,
    parseDate,
    parseDateTime,
    toCalendarDate,
    toTime,
    today,
    type DateValue,
  } from "@internationalized/date";

  interface Props {
    label: string;
    value?: string;
    minDate?: string;
    id?: string;
    invalid?: boolean;
    errorMessage?: string | null;
    describedBy?: string;
    hasPartialInput?: boolean;
    onBlur?: () => void;
  }

  let {
    label,
    value = $bindable(""),
    minDate = "",
    id,
    invalid = false,
    errorMessage = null,
    describedBy,
    hasPartialInput = $bindable(false),
    onBlur,
  }: Props = $props();

  type TzMode = "local" | "utc" | "offset";

  const localTz = getLocalTimeZone();
  const todayCalendarDate = today(localTz);
  const localOffsetMinutes = -new Date().getTimezoneOffset();

  let effectiveMinDate = $derived.by((): DateValue => {
    if (minDate) {
      const parsedMin = parseIsoValue(
        minDate,
        "local",
        localOffsetMinutes,
      ).date;
      if (parsedMin && parsedMin.compare(todayCalendarDate) > 0) {
        return parsedMin;
      }
    }
    return todayCalendarDate;
  });
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

  let tzMode = $state<TzMode>("local");
  let offsetInput = $state(minutesToOffset(localOffsetMinutes));
  let offsetMinutes = $state(localOffsetMinutes);

  function minutesToOffset(mins: number): string {
    const sign = mins < 0 ? "-" : "+";
    const abs = Math.abs(mins);
    return `${sign}${String(Math.floor(abs / 60)).padStart(2, "0")}:${String(
      abs % 60,
    ).padStart(2, "0")}`;
  }

  function offsetToMinutes(raw: string): number | null {
    const m = raw.trim().match(/^([+-])(\d{1,2}):(\d{2})$/);
    if (!m) return null;
    const hours = Number(m[2]);
    const mins = Number(m[3]);
    if (hours > 23 || mins > 59) return null;
    const total = hours * 60 + mins;
    return m[1] === "-" ? -total : total;
  }

  /** Offset (in minutes east of UTC) that the wall clock is interpreted in. */
  function modeOffset(mode: TzMode, offset: number): number {
    if (mode === "utc") return 0;
    if (mode === "offset") return offset;
    return -new Date().getTimezoneOffset();
  }

  function wallClockToUtcMs(
    date: DateValue,
    time: Time,
    mode: TzMode,
    offset: number,
  ): number {
    const cal = toCalendarDate(date);
    if (mode === "local") {
      return new Date(
        cal.year,
        cal.month - 1,
        cal.day,
        time.hour,
        time.minute,
        0,
        0,
      ).getTime();
    }
    return (
      Date.UTC(cal.year, cal.month - 1, cal.day, time.hour, time.minute, 0, 0) -
      modeOffset(mode, offset) * 60_000
    );
  }

  function utcMsToWallClock(
    ms: number,
    mode: TzMode,
    offset: number,
  ): { date: DateValue; time: Time } {
    if (mode === "local") {
      const d = new Date(ms);
      return {
        date: new CalendarDate(d.getFullYear(), d.getMonth() + 1, d.getDate()),
        time: new Time(d.getHours(), d.getMinutes()),
      };
    }
    const d = new Date(ms + modeOffset(mode, offset) * 60_000);
    return {
      date: new CalendarDate(
        d.getUTCFullYear(),
        d.getUTCMonth() + 1,
        d.getUTCDate(),
      ),
      time: new Time(d.getUTCHours(), d.getUTCMinutes()),
    };
  }

  function formatUtcIso(ms: number): string {
    const d = new Date(ms);
    const pad = (n: number) => String(n).padStart(2, "0");
    return `${d.getUTCFullYear()}-${pad(d.getUTCMonth() + 1)}-${pad(
      d.getUTCDate(),
    )}T${pad(d.getUTCHours())}:${pad(d.getUTCMinutes())}:${pad(
      d.getUTCSeconds(),
    )}Z`;
  }

  function parseIsoValue(
    raw: string,
    mode: TzMode,
    offset: number,
  ): { date: DateValue | undefined; time: Time | undefined } {
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
    const cleaned = trimmed.replace(/\[.*\]$/, "");
    if (/[Zz]$|[+-]\d{2}:?\d{2}$/.test(cleaned)) {
      const ms = new Date(cleaned).getTime();
      if (!Number.isNaN(ms)) {
        return utcMsToWallClock(ms, mode, offset);
      }
    }
    try {
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

  /** Serializes the picked wall clock as a canonical UTC instant (`...Z`). */
  function formatCombinedIso(
    dateVal: DateValue | undefined,
    timeVal: Time | undefined,
    mode: TzMode,
    offset: number,
  ): string {
    if (!dateVal) return "";
    const effectiveTime = timeVal ?? DEFAULT_TIME;
    return formatUtcIso(wallClockToUtcMs(dateVal, effectiveTime, mode, offset));
  }

  /** Human-readable local rendering of the current value, for the hint under the row. */
  function localHint(raw: string): string {
    const ms = new Date(raw).getTime();
    if (Number.isNaN(ms)) return raw;
    const wall = utcMsToWallClock(ms, "local", localOffsetMinutes);
    const pad = (n: number) => String(n).padStart(2, "0");
    const cal = toCalendarDate(wall.date);
    return `${cal.year}-${pad(cal.month)}-${pad(cal.day)} ${pad(
      wall.time.hour,
    )}:${pad(wall.time.minute)} ${shortTzName}`;
  }

  const initial = parseIsoValue(value, "local", localOffsetMinutes);
  let pickerDate = $state<DateValue | undefined>(initial.date);
  let pickerTime = $state<Time | undefined>(initial.time);
  let internalResetKey = $state(0);

  $effect(() => {
    const externalValue = value;
    const mode = tzMode;
    const offset = offsetMinutes;
    const currentFormatted = untrack(() =>
      formatCombinedIso(pickerDate, pickerTime, mode, offset),
    );
    if (externalValue !== currentFormatted) {
      const parsed = parseIsoValue(externalValue, mode, offset);
      pickerDate = parsed.date;
      pickerTime = parsed.time;
      if (!externalValue) {
        hasPartialInput = false;
        internalResetKey += 1;
      }
    } else if (!externalValue) {
      untrack(() => {
        if (pickerDate !== undefined || pickerTime !== undefined) {
          pickerDate = undefined;
          pickerTime = undefined;
          hasPartialInput = false;
          internalResetKey += 1;
        }
      });
    }
  });

  function handleDateChange(nextDate: DateValue | undefined) {
    pickerDate = nextDate;
    if (nextDate && !pickerTime) {
      pickerTime = new Time(20, 0);
    } else if (!nextDate) {
      pickerTime = undefined;
    }
    value = formatCombinedIso(pickerDate, pickerTime, tzMode, offsetMinutes);
  }

  function handleTimeChange(nextTime: Time | undefined) {
    pickerTime = nextTime;
    if (nextTime) {
      hasPartialInput = true;
    }
    if (pickerDate) {
      value = formatCombinedIso(pickerDate, pickerTime, tzMode, offsetMinutes);
    }
  }

  function handleOffsetInput(e: Event) {
    offsetInput = (e.currentTarget as HTMLInputElement).value;
    const parsed = offsetToMinutes(offsetInput);
    if (parsed === null) return;
    offsetMinutes = parsed;
    value = formatCombinedIso(pickerDate, pickerTime, tzMode, offsetMinutes);
  }

  function handleTzModeChange(e: Event) {
    tzMode = (e.currentTarget as HTMLSelectElement).value as TzMode;
    if (tzMode === "offset" && offsetToMinutes(offsetInput) === null) {
      offsetInput = minutesToOffset(offsetMinutes);
    }
  }

  function clearDateTime() {
    pickerDate = undefined;
    pickerTime = undefined;
    hasPartialInput = false;
    value = "";
    internalResetKey += 1;
    onBlur?.();
  }

  function handleContainerKeyDown(e: KeyboardEvent) {
    if (
      e.key.length === 1 ||
      e.key === "ArrowUp" ||
      e.key === "ArrowDown" ||
      e.key === "Backspace" ||
      e.key === "Delete"
    ) {
      hasPartialInput = true;
    }
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

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="date-picker-field"
  onfocusout={handleContainerFocusOut}
  onkeydown={handleContainerKeyDown}
>
  {#key internalResetKey}
    <DatePicker.Root
      bind:value={pickerDate}
      onValueChange={handleDateChange}
      minValue={effectiveMinDate}
      granularity="day"
    >
      <div class="date-picker-label-row">
        <DatePicker.Label class="field-label">{label}</DatePicker.Label>
        <div class="tz-control">
          <select
            class="tz-select"
            value={tzMode}
            onchange={handleTzModeChange}
            aria-label={`${label} time zone`}
            title="Time zone used to interpret the date and time you enter. Stored as UTC."
          >
            <option value="local">Local · {shortTzName}</option>
            <option value="utc">UTC · Z</option>
            <option value="offset">Custom offset</option>
          </select>
          {#if tzMode === "offset"}
            <input
              class="tz-offset-input"
              aria-label={`${label} UTC offset (for example -05:00)`}
              value={offsetInput}
              oninput={handleOffsetInput}
              placeholder="+00:00"
              inputmode="text"
              spellcheck="false"
              size="7"
            />
          {/if}
        </div>
      </div>

      {#if tzMode !== "local" && value}
        <span class="tz-hint">= {localHint(value)}</span>
      {/if}

      <div
        class="date-picker-row"
        data-invalid={invalid || errorMessage ? "true" : undefined}
      >
        <DatePicker.Input
          {id}
          aria-invalid={invalid || errorMessage ? "true" : undefined}
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
          bind:value={pickerTime}
          onValueChange={handleTimeChange}
          placeholder={DEFAULT_TIME}
          granularity="minute"
        >
          <TimeField.Input
            aria-label={`${label} time (${shortTzName})`}
            aria-invalid={invalid || errorMessage ? "true" : undefined}
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

        {#if value || pickerDate || pickerTime || hasPartialInput}
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
      {#if errorMessage}
        <span class="field-error" role="alert">{errorMessage}</span>
      {/if}

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
  {/key}
</div>
