<script lang="ts">
  import { Dialog } from "bits-ui";

  interface Props {
    open?: boolean;
    title: string;
    description: string;
    confirmLabel?: string;
    cancelLabel?: string;
    danger?: boolean;
    onConfirm: () => void;
    onCancel: () => void;
  }

  let {
    open = $bindable(false),
    title,
    description,
    confirmLabel = "Confirm",
    cancelLabel = "Cancel",
    danger = true,
    onConfirm,
    onCancel,
  }: Props = $props();

  function handleOpenChange(nextOpen: boolean) {
    open = nextOpen;
    if (!nextOpen) {
      onCancel();
    }
  }
</script>

<Dialog.Root {open} onOpenChange={handleOpenChange}>
  <Dialog.Portal>
    <Dialog.Overlay class="confirm-modal-backdrop" />
    <Dialog.Content class="confirm-modal-dialog" role="alertdialog">
      <Dialog.Title class="confirm-modal-title">{title}</Dialog.Title>
      <Dialog.Description class="confirm-modal-desc">
        {description}
      </Dialog.Description>
      <div class="confirm-modal-actions">
        <button
          type="button"
          onclick={() => {
            open = false;
            onCancel();
          }}
        >
          {cancelLabel}
        </button>
        <button
          type="button"
          class={danger ? "btn-danger" : "btn-primary"}
          onclick={() => {
            open = false;
            onConfirm();
          }}
        >
          {confirmLabel}
        </button>
      </div>
    </Dialog.Content>
  </Dialog.Portal>
</Dialog.Root>
