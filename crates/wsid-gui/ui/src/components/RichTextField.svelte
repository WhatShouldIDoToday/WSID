<script lang="ts">
  import {
    tokenizeCommentBody,
    searchTasksData,
    USERNAME_REGEX,
  } from "../lib/ipc";

  interface Props {
    value?: string;
    knownUsers: string[];
    /** Account kind per user (`human` unless set); drives typed-mention hints. */
    userKinds?: Record<string, string>;
    /** `comment` also offers `@takenote:` and notes that mentions spawn tasks. */
    mode?: "comment" | "description";
    id?: string;
    rows?: number;
    placeholder?: string;
    describedBy?: string;
    onPasteFiles?: (files: FileList) => void;
  }

  let {
    value = $bindable(""),
    knownUsers,
    userKinds = {},
    mode = "description",
    id,
    rows = 4,
    placeholder = "",
    describedBy,
    onPasteFiles,
  }: Props = $props();

  type Trigger = "mention" | "takenote" | "task";

  interface Suggestion {
    kind: Trigger;
    /** Text shown in the list. */
    label: string;
    /** Text inserted in place of the `@…` / `#…` token. */
    insert: string;
    detail?: string;
  }

  let textarea = $state<HTMLTextAreaElement | null>(null);
  let trigger = $state<Trigger | null>(null);
  let query = $state("");
  let tokenStart = $state(0);
  let activeIndex = $state(0);
  let taskHits = $state<Suggestion[]>([]);
  let searchTimer: ReturnType<typeof setTimeout> | null = null;

  let userSuggestions = $derived.by((): Suggestion[] => {
    if (trigger !== "mention" && trigger !== "takenote") return [];
    const q = query.toLowerCase();
    const matches: Suggestion[] = knownUsers
      .filter((u) => u.toLowerCase().startsWith(q))
      .slice(0, 8)
      .map((u) => ({
        kind: (trigger === "takenote" ? "takenote" : "mention") as Trigger,
        label: `@${u}`,
        insert: trigger === "takenote" ? `@takenote:${u}` : `@${u}`,
      }));
    const valid = USERNAME_REGEX.test(q);
    const alreadyKnown = knownUsers.some((u) => u.toLowerCase() === q);
    if (q && valid && !alreadyKnown) {
      matches.push({
        kind: "mention",
        label: `@${q}`,
        insert: trigger === "takenote" ? `@takenote:${q}` : `@${q}`,
        detail: "new user will be created",
      });
    }
    return matches;
  });

  let suggestions = $derived(trigger === "task" ? taskHits : userSuggestions);

  function close() {
    trigger = null;
    query = "";
    activeIndex = 0;
    taskHits = [];
  }

  function syncTrigger() {
    if (!textarea) return;
    const caret = textarea.selectionStart ?? value.length;
    const before = value.slice(0, caret);

    const mention = before.match(/@((?:takenote:)?)([a-zA-Z0-9_-]*)$/);
    if (mention) {
      trigger = mention[1] ? "takenote" : "mention";
      query = mention[2] ?? "";
      tokenStart = caret - mention[0].length;
      activeIndex = 0;
      return;
    }

    const task = before.match(/#([a-zA-Z0-9]*)$/);
    if (task) {
      trigger = "task";
      query = task[1] ?? "";
      tokenStart = caret - task[0].length;
      activeIndex = 0;
      scheduleTaskSearch(query);
      return;
    }

    close();
  }

  function scheduleTaskSearch(q: string) {
    if (searchTimer) clearTimeout(searchTimer);
    searchTimer = setTimeout(async () => {
      if (!q.trim()) {
        taskHits = [];
        return;
      }
      try {
        const hits = await searchTasksData({ query: q, limit: 8 });
        taskHits = hits.map((hit) => ({
          kind: "task" as Trigger,
          label: `#${hit.task.id} — ${hit.task.title}`,
          insert: `#${hit.task.id}`,
          detail: hit.task.project,
        }));
      } catch {
        taskHits = [];
      }
    }, 180);
  }

  function accept(suggestion: Suggestion) {
    if (!textarea) return;
    const caret = textarea.selectionStart ?? value.length;
    const next =
      value.slice(0, tokenStart) + suggestion.insert + value.slice(caret);
    value = next;
    const pos = tokenStart + suggestion.insert.length;
    close();
    requestAnimationFrame(() => {
      textarea?.focus();
      textarea?.setSelectionRange(pos, pos);
    });
  }

  function handleKeydown(e: KeyboardEvent) {
    if (trigger && suggestions.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        activeIndex = (activeIndex + 1) % suggestions.length;
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        activeIndex =
          (activeIndex - 1 + suggestions.length) % suggestions.length;
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        accept(suggestions[Math.min(activeIndex, suggestions.length - 1)]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        close();
        return;
      }
    }
    if (e.key === "Escape" && trigger) {
      e.preventDefault();
      close();
    }
  }

  let previewSegments = $derived(
    value.trim()
      ? tokenizeCommentBody(value, knownUsers, mode, userKinds).filter(
          (s) => s.type !== "text",
        )
      : [],
  );
</script>

<div class="rich-text-field">
  <textarea
    {id}
    {rows}
    {placeholder}
    aria-describedby={describedBy}
    role="combobox"
    aria-autocomplete="list"
    aria-controls={id ? `${id}-suggestions` : undefined}
    aria-expanded={Boolean(trigger && suggestions.length > 0)}
    bind:this={textarea}
    bind:value
    oninput={syncTrigger}
    onclick={syncTrigger}
    onkeydown={handleKeydown}
    onblur={() => setTimeout(close, 120)}
    onpaste={(e) => {
      if (e.clipboardData?.files?.length && onPasteFiles) {
        onPasteFiles(e.clipboardData.files);
      }
    }}></textarea>

  {#if trigger && suggestions.length > 0}
    <ul
      class="mention-menu"
      role="listbox"
      aria-label="Suggestions"
      id={id ? `${id}-suggestions` : undefined}
    >
      {#each suggestions as suggestion, idx (suggestion.insert + suggestion.label)}
        <li
          role="option"
          aria-selected={idx === activeIndex}
          class:active={idx === activeIndex}
          onmousedown={(e) => {
            e.preventDefault();
            accept(suggestion);
          }}
        >
          <span>{suggestion.label}</span>
          {#if suggestion.detail}
            <span class="mention-menu-detail">{suggestion.detail}</span>
          {/if}
        </li>
      {/each}
    </ul>
  {/if}

  {#if previewSegments.length > 0}
    <div class="comment-mention-preview" aria-live="polite">
      {#each previewSegments as seg}
        {#if seg.type === "mention" && seg.mention}
          <span
            class={`comment-inline-mention mention-${seg.mention.kind}`}
            title={seg.mention.tooltip}
          >
            {seg.text}
          </span>
        {:else}
          <span class="comment-inline-task-ref">{seg.text}</span>
        {/if}
      {/each}
      <span class="field-hint">
        {mode === "comment"
          ? "Posting creates missing users and spawns tasks for @mentions."
          : "Saving creates missing users and links existing #task references."}
      </span>
    </div>
  {/if}
</div>
