<script lang="ts">
  interface Props {
    /** Pointer position in viewport (CSS) pixels. */
    x: number;
    y: number;
    /** Where inside the source card the pointer grabbed it. */
    offsetX: number;
    offsetY: number;
    width: number;
    taskId: string;
    title: string;
    subtitle?: string;
  }

  let { x, y, offsetX, offsetY, width, taskId, title, subtitle }: Props =
    $props();
</script>

<!--
  Floating stand-in for the card being dragged. Tauri's file-drop interceptor
  swallows native HTML5 drags inside the webview, so the browser's own drag
  image never appears; this follows the pointer instead. `pointer-events: none`
  keeps `document.elementFromPoint` reporting the drop target underneath.
-->
<div
  class="drag-ghost"
  style={`left:${x - offsetX}px; top:${y - offsetY}px; width:${width}px;`}
  aria-hidden="true"
>
  <span class="task-id-badge">{taskId}</span>
  <strong>{title}</strong>
  {#if subtitle}
    <span class="drag-ghost-meta">{subtitle}</span>
  {/if}
</div>
