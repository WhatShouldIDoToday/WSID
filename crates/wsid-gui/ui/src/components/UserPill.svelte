<script lang="ts">
  import { DEFAULT_USER_KIND } from "../lib/ipc";

  interface Props {
    user: string;
    /** `human` renders a solid blue pill; any other kind renders a dashed agent pill. */
    kind?: string;
    /** Optional visible-to-SR role prefix, e.g. "Assignee". */
    role?: string;
  }

  let { user, kind = DEFAULT_USER_KIND, role }: Props = $props();

  let isAgent = $derived(kind !== DEFAULT_USER_KIND);
</script>

<span
  class="user-pill"
  class:is-agent={isAgent}
  title={role ? `${role}: @${user}${isAgent ? ` (${kind})` : ""}` : undefined}
>
  {#if role}<span class="sr-only">{role}: </span>{/if}@{user}{#if isAgent}
    <span class="user-pill-kind"> · {kind}</span>{/if}
</span>
