<script lang="ts">
  import { tokenizeCommentBody } from "../lib/ipc";

  interface Props {
    body: string;
    knownUsers: string[];
    /** Descriptions only create users; comments also spawn subtasks. */
    mode?: "comment" | "description";
    /** Called when a `#AB1234` reference is activated. */
    onSelectTask?: (id: string) => void;
    emptyText?: string;
    class?: string;
  }

  let {
    body,
    knownUsers,
    mode = "description",
    onSelectTask,
    emptyText = "No description provided.",
    class: className = "",
  }: Props = $props();

  let segments = $derived(tokenizeCommentBody(body, knownUsers, mode));

  function mentionClass(seg: (typeof segments)[number]): string {
    if (seg.type !== "mention" || !seg.mention) return "";
    return `comment-inline-mention mention-${seg.mention.kind}`;
  }
</script>

{#if body.trim()}
  <div class={`rich-text ${className}`}>
    {#each segments as seg}
      {#if seg.type === "text"}
        {seg.text}
      {:else if seg.type === "mention" && seg.mention}
        <span class={mentionClass(seg)} title={seg.mention.tooltip}>
          {seg.text}
        </span>
      {:else}
        {#if onSelectTask && seg.taskId}
          <button
            type="button"
            class="comment-inline-task-ref"
            onclick={() => onSelectTask(seg.taskId!)}
          >
            {seg.text}
          </button>
        {:else}
          <span class="comment-inline-task-ref">{seg.text}</span>
        {/if}
      {/if}
    {/each}
  </div>
{:else}
  <p class="field-hint">{emptyText}</p>
{/if}
