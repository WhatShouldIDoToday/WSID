<script lang="ts">
  import { onMount } from "svelte";
  import {
    type SearchHit,
    type TaskStatus,
    STATUS_LABELS,
    STATUS_ORDER,
    searchTasksData,
    formatDurationMinutes,
    formatDeadlineDisplay,
  } from "../lib/ipc";
  import { Selection } from "../lib/selection.svelte";
  import BulkActions from "./BulkActions.svelte";

  interface Props {
    knownTags: string[];
    knownUsers: string[];
    refreshTrigger: number;
    onSelectTask: (id: string) => void;
    onDataChanged: () => void;
    announce: (msg: string) => void;
  }

  let {
    knownTags,
    knownUsers,
    refreshTrigger,
    onSelectTask,
    onDataChanged,
    announce,
  }: Props = $props();

  const selection = new Selection();

  type SortCol =
    | "score"
    | "id"
    | "title"
    | "status"
    | "assignee"
    | "importance"
    | "difficulty"
    | "due_date";

  let query = $state<string>("");
  let statusFilter = $state<string>("");
  let tagFilter = $state<string>("");
  let assigneeFilter = $state<string>("");
  let hits = $state<SearchHit[]>([]);
  let sortCol = $state<SortCol>("score");
  let sortAsc = $state<boolean>(false);

  async function executeSearch() {
    try {
      const results = await searchTasksData({
        query,
        status: (statusFilter || null) as TaskStatus | null,
        tag: tagFilter || null,
        assignee: assigneeFilter || null,
      });
      hits = results;
      selection.prune(results.map((h) => h.task.id));
    } catch (err) {
      announce(`Search error: ${String(err)}`);
    }
  }

  $effect(() => {
    void refreshTrigger;
    void query;
    void statusFilter;
    void tagFilter;
    void assigneeFilter;
    void executeSearch();
  });

  onMount(() => {
    void executeSearch();
  });

  function toggleSort(col: SortCol) {
    if (sortCol === col) {
      sortAsc = !sortAsc;
    } else {
      sortCol = col;
      sortAsc = col === "id" || col === "title" || col === "due_date";
    }
  }

  function ariaSortFor(col: SortCol): "ascending" | "descending" | "none" {
    if (sortCol !== col) return "none";
    return sortAsc ? "ascending" : "descending";
  }

  let sortedHits = $derived.by(() => {
    const copy = [...hits];
    copy.sort((a, b) => {
      let cmp = 0;
      switch (sortCol) {
        case "score":
          cmp = a.score - b.score;
          break;
        case "id":
          cmp = a.task.id.localeCompare(b.task.id);
          break;
        case "title":
          cmp = a.task.title.localeCompare(b.task.title);
          break;
        case "status":
          cmp = a.task.status.localeCompare(b.task.status);
          break;
        case "assignee":
          cmp = a.task.assignee.localeCompare(b.task.assignee);
          break;
        case "importance":
          cmp = a.task.importance - b.task.importance;
          break;
        case "difficulty":
          cmp = a.task.difficulty - b.task.difficulty;
          break;
        case "due_date": {
          const da = a.task.sgdb_date ?? a.task.due_date ?? "9999-99-99";
          const db = b.task.sgdb_date ?? b.task.due_date ?? "9999-99-99";
          cmp = da.localeCompare(db);
          break;
        }
      }
      return sortAsc ? cmp : -cmp;
    });
    return copy;
  });

  let orderedIds = $derived(sortedHits.map((h) => h.task.id));
  let allSelected = $derived(
    orderedIds.length > 0 && orderedIds.every((id) => selection.has(id)),
  );
  let someSelected = $derived(selection.count > 0 && !allSelected);

  function toggleAll() {
    if (allSelected) selection.clear();
    else selection.set(orderedIds);
  }
</script>

<section aria-labelledby="database-view-heading">
  <div style="margin-bottom: 1rem;">
    <h2 id="database-view-heading" style="margin: 0; font-size: 1.35rem;">
      Database Search
    </h2>
  </div>

  <div
    class="card"
    style="margin-bottom: 1rem; display: grid; grid-template-columns: repeat(auto-fit, minmax(200px, 1fr)); gap: 0.85rem; align-items: end;"
    role="search"
    aria-label="Filter and search tasks"
  >
    <div class="form-field">
      <label for="db-search-input">Fuzzy Search Query</label>
      <input
        id="db-search-input"
        type="search"
        bind:value={query}
        placeholder="Search by ID (AB1234), title, tag, or typo..."
      />
    </div>

    <div class="form-field">
      <label for="db-status-filter">Status</label>
      <select id="db-status-filter" bind:value={statusFilter}>
        <option value="">All Statuses</option>
        {#each STATUS_ORDER as st}
          <option value={st}>{STATUS_LABELS[st]}</option>
        {/each}
      </select>
    </div>

    <div class="form-field">
      <label for="db-tag-filter">Tag</label>
      <select id="db-tag-filter" bind:value={tagFilter}>
        <option value="">All Tags</option>
        {#each knownTags as t}
          <option value={t}>#{t}</option>
        {/each}
      </select>
    </div>

    <div class="form-field">
      <label for="db-assignee-filter">Assignee</label>
      <select id="db-assignee-filter" bind:value={assigneeFilter}>
        <option value="">All Assignees</option>
        {#each knownUsers as u}
          <option value={u}>@{u}</option>
        {/each}
      </select>
    </div>
  </div>

  <div
    style="margin-bottom: 0.5rem; font-size: 0.85rem; color: var(--text-secondary);"
  >
    Showing <strong>{sortedHits.length}</strong> matching task(s).
  </div>

  {#if selection.count > 0}
    <BulkActions
      ids={selection.ids}
      {knownTags}
      {knownUsers}
      {announce}
      onClear={() => selection.clear()}
      onDone={(outcome) => {
        if (outcome.failures.length === 0) selection.clear();
        onDataChanged();
        void executeSearch();
      }}
    />
  {/if}

  <div class="table-container">
    <table class="db-table" aria-label="Tasks Database Table">
      <thead>
        <tr>
          <th scope="col">
            <input
              type="checkbox"
              class="select-check"
              aria-label="Select all shown tasks"
              checked={allSelected}
              indeterminate={someSelected}
              disabled={orderedIds.length === 0}
              onchange={toggleAll}
            />
          </th>
          <th scope="col" aria-sort={ariaSortFor("id")}>
            <button
              type="button"
              class="sort-btn"
              onclick={() => toggleSort("id")}
            >
              ID {sortCol === "id" ? (sortAsc ? "▲" : "▼") : ""}
            </button>
          </th>
          <th scope="col" aria-sort={ariaSortFor("title")}>
            <button
              type="button"
              class="sort-btn"
              onclick={() => toggleSort("title")}
            >
              Title {sortCol === "title" ? (sortAsc ? "▲" : "▼") : ""}
            </button>
          </th>
          <th scope="col" aria-sort={ariaSortFor("status")}>
            <button
              type="button"
              class="sort-btn"
              onclick={() => toggleSort("status")}
            >
              Status {sortCol === "status" ? (sortAsc ? "▲" : "▼") : ""}
            </button>
          </th>
          <th scope="col" aria-sort={ariaSortFor("assignee")}>
            <button
              type="button"
              class="sort-btn"
              onclick={() => toggleSort("assignee")}
            >
              Assignee {sortCol === "assignee" ? (sortAsc ? "▲" : "▼") : ""}
            </button>
          </th>
          <th scope="col" aria-sort={ariaSortFor("importance")}>
            <button
              type="button"
              class="sort-btn"
              onclick={() => toggleSort("importance")}
            >
              Imp {sortCol === "importance" ? (sortAsc ? "▲" : "▼") : ""}
            </button>
          </th>
          <th scope="col" aria-sort={ariaSortFor("difficulty")}>
            <button
              type="button"
              class="sort-btn"
              onclick={() => toggleSort("difficulty")}
            >
              Diff {sortCol === "difficulty" ? (sortAsc ? "▲" : "▼") : ""}
            </button>
          </th>
          <th scope="col">Estimate</th>
          <th scope="col" aria-sort={ariaSortFor("due_date")}>
            <button
              type="button"
              class="sort-btn"
              onclick={() => toggleSort("due_date")}
            >
              SGDB / Due {sortCol === "due_date" ? (sortAsc ? "▲" : "▼") : ""}
            </button>
          </th>
          <th scope="col">Tags</th>
          <th scope="col">Actions</th>
        </tr>
      </thead>
      <tbody>
        {#if sortedHits.length === 0}
          <tr>
            <td
              colspan="11"
              style="text-align: center; padding: 1.5rem; color: var(--text-muted);"
            >
              No tasks match the current search and filters.
            </td>
          </tr>
        {:else}
          {#each sortedHits as hit (hit.task.id)}
            <tr class:is-selected={selection.has(hit.task.id)}>
              <td>
                <input
                  type="checkbox"
                  class="select-check"
                  aria-label={`Select ${hit.task.id}`}
                  checked={selection.has(hit.task.id)}
                  onclick={(e) => selection.click(e, hit.task.id, orderedIds)}
                />
              </td>
              <td>
                <span class="task-id-badge">{hit.task.id}</span>
              </td>
              <td>
                <strong>{hit.task.title}</strong>
                {#if hit.task.urgent_override}
                  <span
                    class="badge badge-urgent"
                    style="margin-left: 0.35rem;"
                  >
                    Urgent
                  </span>
                {/if}
                {#if hit.task.attachments.length > 0}
                  <span class="badge" style="margin-left: 0.35rem;">
                    {hit.task.attachments.length} file(s)
                  </span>
                {/if}
              </td>
              <td>
                <span class="badge">{STATUS_LABELS[hit.task.status]}</span>
              </td>
              <td>@{hit.task.assignee}</td>
              <td>{hit.task.importance}/5</td>
              <td>{hit.task.difficulty}/5</td>
              <td>{formatDurationMinutes(hit.task.time_estimate_minutes)}</td>
              <td>
                {#if hit.task.sgdb_date}
                  SGDB: {formatDeadlineDisplay(hit.task.sgdb_date)}
                {/if}
                {#if hit.task.due_date}
                  {hit.task.sgdb_date ? " · " : ""}Due: {formatDeadlineDisplay(
                    hit.task.due_date,
                  )}
                {/if}
                {#if !hit.task.sgdb_date && !hit.task.due_date}
                  —
                {/if}
              </td>
              <td>
                {#each hit.task.tags as tag}
                  <span class="badge badge-tag" style="margin-right: 0.25rem;">
                    #{tag}
                  </span>
                {/each}
              </td>
              <td>
                <button
                  type="button"
                  class="btn-sm"
                  onclick={() => onSelectTask(hit.task.id)}
                >
                  Details
                </button>
              </td>
            </tr>
          {/each}
        {/if}
      </tbody>
    </table>
  </div>
</section>
