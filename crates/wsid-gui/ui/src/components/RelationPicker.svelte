<script lang="ts">
  import { Combobox } from "bits-ui";
  import {
    type RelationInputKind,
    type SearchHit,
    STATUS_LABELS,
    searchTasksData,
  } from "../lib/ipc";

  export interface SelectedRelationEntry {
    relation: RelationInputKind;
    target_id: string;
    target_title?: string;
  }

  interface Props {
    excludeTaskId?: string;
    buttonLabel?: string;
    searchQuery?: string;
    onAdd: (entry: SelectedRelationEntry) => void | Promise<void>;
  }

  let {
    excludeTaskId = "",
    buttonLabel = "Add Relation",
    searchQuery = $bindable(""),
    onAdd,
  }: Props = $props();

  let selectedTaskId = $state("");
  let open = $state(false);
  let results = $state<SearchHit[]>([]);
  let relationKind = $state<RelationInputKind>("related");

  const RELATION_OPTIONS: { value: RelationInputKind; label: string }[] = [
    { value: "related", label: "Related to" },
    { value: "blocks", label: "Blocks" },
    { value: "blocked_by", label: "Is blocked by" },
    { value: "sub_task_of", label: "Sub-task of" },
    { value: "parent_task_of", label: "Parent task of" },
    { value: "parallel", label: "Parallel with" },
  ];

  async function runLookup(q: string) {
    try {
      const hits = await searchTasksData({
        query: q.trim(),
        limit: 12,
      });
      const excluded = excludeTaskId.trim().toUpperCase();
      results = excluded
        ? hits.filter((h) => h.task.id.toUpperCase() !== excluded)
        : hits;
    } catch {
      results = [];
    }
  }

  function handleSearchInput(e: Event) {
    const val = (e.currentTarget as HTMLInputElement).value;
    searchQuery = val;
    const cleaned = val.trim().replace(/^#/, "").toUpperCase();
    if (/^[A-Z]{2}\d{4}$/.test(cleaned)) {
      selectedTaskId = cleaned;
    } else if (
      selectedTaskId &&
      !val.toUpperCase().startsWith(selectedTaskId)
    ) {
      selectedTaskId = "";
    }
    open = true;
    void runLookup(val);
  }

  function handleValueChange(val: string) {
    if (!val) return;
    selectedTaskId = val;
    const matched = results.find((r) => r.task.id === val);
    if (matched) {
      searchQuery = `${matched.task.id} — ${matched.task.title}`;
    } else {
      searchQuery = val;
    }
    open = false;
  }

  function resolveTarget(): { id: string; title?: string } | null {
    if (selectedTaskId) {
      const matched = results.find((r) => r.task.id === selectedTaskId);
      return { id: selectedTaskId, title: matched?.task.title };
    }
    const raw = searchQuery.trim().replace(/^#/, "");
    if (!raw) return null;
    const prefixMatch = raw.match(/^([A-Za-z]{2}\d{4})\b/);
    if (prefixMatch) {
      const id = prefixMatch[1].toUpperCase();
      const matched = results.find((r) => r.task.id === id);
      return { id, title: matched?.task.title };
    }
    if (results.length > 0) {
      return { id: results[0].task.id, title: results[0].task.title };
    }
    return { id: raw.toUpperCase() };
  }

  async function submitRelation() {
    const resolved = resolveTarget();
    if (!resolved) return;
    await onAdd({
      relation: relationKind,
      target_id: resolved.id,
      target_title: resolved.title,
    });
    searchQuery = "";
    selectedTaskId = "";
    open = false;
  }

  function handleSelectKeyDown(e: KeyboardEvent) {
    if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
      e.preventDefault();
      void submitRelation();
    }
  }
</script>

<div class="relation-picker-row">
  <div class="relation-combobox-wrapper">
    <Combobox.Root
      type="single"
      value={selectedTaskId}
      onValueChange={handleValueChange}
      bind:open
      inputValue={searchQuery}
    >
      <div class="relation-input-group">
        <Combobox.Input
          class="relation-combobox-input"
          placeholder="Search task by title or ID (e.g. AB1234)..."
          aria-label="Search target task by title or ID"
          oninput={handleSearchInput}
          onfocus={() => {
            open = true;
            void runLookup(searchQuery);
          }}
        />
        <Combobox.Trigger
          class="relation-combobox-trigger"
          aria-label="Toggle task suggestions"
        >
          ▾
        </Combobox.Trigger>
      </div>

      <Combobox.Portal>
        <Combobox.Content class="relation-combobox-content" sideOffset={4}>
          <Combobox.Viewport>
            {#if results.length === 0}
              <div class="relation-combobox-empty">No matching tasks found</div>
            {:else}
              {#each results as hit (hit.task.id)}
                <Combobox.Item
                  value={hit.task.id}
                  label={`${hit.task.id} — ${hit.task.title}`}
                  class="relation-combobox-item"
                >
                  <span class="task-id">{hit.task.id}</span>
                  <span class="relation-item-title">{hit.task.title}</span>
                  <span class="relation-item-meta">
                    @{hit.task.assignee} · {STATUS_LABELS[hit.task.status]}
                  </span>
                </Combobox.Item>
              {/each}
            {/if}
          </Combobox.Viewport>
        </Combobox.Content>
      </Combobox.Portal>
    </Combobox.Root>
  </div>

  <select
    class="relation-kind-select"
    aria-label="Relation type"
    bind:value={relationKind}
    onkeydown={handleSelectKeyDown}
  >
    {#each RELATION_OPTIONS as opt}
      <option value={opt.value}>{opt.label}</option>
    {/each}
  </select>

  <button
    type="button"
    class="btn-sm"
    onclick={submitRelation}
    disabled={!searchQuery.trim() && !selectedTaskId}
  >
    {buttonLabel}
  </button>
</div>
