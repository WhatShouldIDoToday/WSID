<script lang="ts">
  import { onMount } from "svelte";
  import {
    type TaskAttachment,
    isImageMediaType,
    readAttachmentPreviewDataUrl,
    openAttachmentNative,
    editAttachmentAltDescription,
    deleteAttachment,
  } from "../lib/ipc";

  interface Props {
    attachments: TaskAttachment[];
    onChanged: () => void;
    announce: (msg: string) => void;
  }

  let { attachments, onChanged, announce }: Props = $props();

  let previewUrls = $state<Record<number, string>>({});
  let editingId = $state<number | null>(null);
  let editDescriptionValue = $state<string>("");
  let lightboxAttachment = $state<TaskAttachment | null>(null);

  async function loadPreviews(list: TaskAttachment[]) {
    for (const att of list) {
      if (isImageMediaType(att.media_type) && !previewUrls[att.id]) {
        try {
          const url = await readAttachmentPreviewDataUrl(att.id);
          previewUrls = { ...previewUrls, [att.id]: url };
        } catch {
          // Non-fatal if file is missing or unreadable
        }
      }
    }
  }

  $effect(() => {
    void loadPreviews(attachments);
  });

  onMount(() => {
    void loadPreviews(attachments);
  });

  function getAltText(att: TaskAttachment): string {
    return att.description && att.description.trim().length > 0
      ? att.description.trim()
      : att.file_name;
  }

  function startEditDescription(att: TaskAttachment) {
    editingId = att.id;
    editDescriptionValue = att.description;
  }

  async function saveDescription(att: TaskAttachment) {
    try {
      await editAttachmentAltDescription(att.id, editDescriptionValue);
      editingId = null;
      announce(`Updated alt description for ${att.file_name}`);
      onChanged();
    } catch (err) {
      announce(`Error updating attachment description: ${String(err)}`);
    }
  }

  async function handleRemove(att: TaskAttachment) {
    try {
      await deleteAttachment(att.id);
      announce(
        att.is_managed
          ? `Removed attachment ${att.file_name} and deleted managed copy`
          : `Unlinked external attachment ${att.file_name}`,
      );
      onChanged();
    } catch (err) {
      announce(`Error removing attachment: ${String(err)}`);
    }
  }

  async function handleOpenNative(att: TaskAttachment) {
    try {
      await openAttachmentNative({ attachmentId: att.id });
      announce(`Opened ${att.file_name} in default OS application`);
    } catch (err) {
      announce(`Unable to open file in OS viewer: ${String(err)}`);
    }
  }
</script>

{#if attachments.length > 0}
  <div class="attachment-grid" role="list" aria-label="Attached files">
    {#each attachments as att (att.id)}
      <figure class="attachment-card" role="listitem" style="margin: 0;">
        {#if isImageMediaType(att.media_type) && previewUrls[att.id]}
          <button
            type="button"
            class="attachment-thumb-btn"
            onclick={() => (lightboxAttachment = att)}
            aria-label={`Open full-size image preview for ${getAltText(att)}`}
          >
            <img src={previewUrls[att.id]} alt={getAltText(att)} />
          </button>
        {:else}
          <div
            style="padding: 0.75rem; background: var(--bg-surface); border-radius: 0.375rem; font-weight: 600;"
          >
            File: {att.file_name} ({att.media_type})
          </div>
        {/if}

        <figcaption
          style="display: flex; flex-direction: column; gap: 0.35rem;"
        >
          <div style="font-size: 0.85rem; font-weight: 600;">
            Alt / Description: <span style="font-weight: 400;"
              >{getAltText(att)}</span
            >
          </div>

          <div style="font-size: 0.78rem; color: var(--text-secondary);">
            Disk location ({att.is_managed
              ? "Managed app-data copy"
              : "External file"}):
            <button
              type="button"
              class="attachment-disk-path"
              onclick={() => handleOpenNative(att)}
              aria-label={`Open file on disk at ${att.file_path} in native application`}
            >
              {att.file_path}
            </button>
          </div>

          {#if editingId === att.id}
            <div
              style="display: flex; flex-direction: column; gap: 0.35rem; margin-top: 0.25rem;"
            >
              <label for={`att-desc-${att.id}`} class="sr-only">
                Alt text description for {att.file_name}
              </label>
              <input
                id={`att-desc-${att.id}`}
                type="text"
                bind:value={editDescriptionValue}
                placeholder={att.file_name}
              />
              <div class="actions-row">
                <button
                  type="button"
                  class="btn-sm btn-primary"
                  onclick={() => saveDescription(att)}
                >
                  Save Alt Text
                </button>
                <button
                  type="button"
                  class="btn-sm"
                  onclick={() => (editingId = null)}
                >
                  Cancel
                </button>
              </div>
            </div>
          {:else}
            <div class="actions-row">
              {#if isImageMediaType(att.media_type) && previewUrls[att.id]}
                <button
                  type="button"
                  class="btn-sm"
                  onclick={() => (lightboxAttachment = att)}
                >
                  Preview Image
                </button>
              {/if}
              <button
                type="button"
                class="btn-sm"
                onclick={() => handleOpenNative(att)}
              >
                Open in OS App
              </button>
              <button
                type="button"
                class="btn-sm"
                onclick={() => startEditDescription(att)}
              >
                Edit Alt Text
              </button>
              <button
                type="button"
                class="btn-sm btn-danger"
                onclick={() => handleRemove(att)}
              >
                Remove
              </button>
            </div>
          {/if}
        </figcaption>
      </figure>
    {/each}
  </div>
{/if}

{#if lightboxAttachment}
  <div
    class="lightbox-backdrop"
    role="presentation"
    onclick={(e) => {
      if (e.target === e.currentTarget) lightboxAttachment = null;
    }}
  >
    <div
      class="lightbox-dialog"
      role="dialog"
      aria-modal="true"
      aria-labelledby="lightbox-title"
      tabindex="-1"
      onkeydown={(e) => {
        if (e.key === "Escape") lightboxAttachment = null;
      }}
    >
      <div
        style="display: flex; justify-content: space-between; align-items: center; gap: 1rem;"
      >
        <h3 id="lightbox-title" style="margin: 0; font-size: 1.05rem;">
          {getAltText(lightboxAttachment)}
        </h3>
        <button
          type="button"
          onclick={() => (lightboxAttachment = null)}
          aria-label="Close image preview"
        >
          Close (Esc)
        </button>
      </div>

      {#if previewUrls[lightboxAttachment.id]}
        <img
          src={previewUrls[lightboxAttachment.id]}
          alt={getAltText(lightboxAttachment)}
        />
      {/if}

      <div class="meta-row" style="justify-content: space-between;">
        <span>Path: <code>{lightboxAttachment.file_path}</code></span>
        <button
          type="button"
          class="btn-sm btn-primary"
          onclick={() =>
            lightboxAttachment && handleOpenNative(lightboxAttachment)}
        >
          Open in Native OS Application
        </button>
      </div>
    </div>
  </div>
{/if}
