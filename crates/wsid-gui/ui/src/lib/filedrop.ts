import { getCurrentWebview, type DragDropEvent } from "@tauri-apps/api/webview";

export interface BoundingRectLike {
  left: number;
  top: number;
  right: number;
  bottom: number;
  width: number;
  height: number;
}

export interface DropzoneCallbacks {
  onDragChange?: (dragging: boolean) => void;
  onDrop: (paths: string[]) => void;
}

/**
 * Converts a physical pixel coordinate from Tauri's `onDragDropEvent` into CSS
 * pixels using `devicePixelRatio` and checks whether it falls within `rect`.
 */
export function hitTestPhysicalPoint(
  position: { x: number; y: number },
  rect: BoundingRectLike,
  devicePixelRatio = 1,
): boolean {
  if (rect.width <= 0 || rect.height <= 0) return false;
  const dpr = devicePixelRatio > 0 ? devicePixelRatio : 1;
  const x = position.x / dpr;
  const y = position.y / dpr;
  return x >= rect.left && x <= rect.right && y >= rect.top && y <= rect.bottom;
}

/**
 * Hit-tests a physical pixel coordinate against a DOM dropzone element,
 * ensuring the element is not occluded by a modal backdrop above it.
 */
export function isPointInElement(
  position: { x: number; y: number },
  el: HTMLElement | null,
  devicePixelRatio?: number,
): boolean {
  if (!el) return false;
  const dpr =
    devicePixelRatio ??
    ((typeof window !== "undefined" && window.devicePixelRatio) || 1);
  const rect = el.getBoundingClientRect();
  if (!hitTestPhysicalPoint(position, rect, dpr)) {
    return false;
  }
  if (
    typeof document !== "undefined" &&
    typeof document.elementFromPoint === "function"
  ) {
    const cssX = position.x / (dpr > 0 ? dpr : 1);
    const cssY = position.y / (dpr > 0 ? dpr : 1);
    const hit = document.elementFromPoint(cssX, cssY);
    if (hit && !el.contains(hit)) {
      return false;
    }
  }
  return true;
}

/**
 * Dispatches a single Tauri `DragDropEvent` payload against a target dropzone.
 */
export function handleDragDropPayload(
  payload: DragDropEvent,
  el: HTMLElement | null,
  callbacks: DropzoneCallbacks,
  devicePixelRatio?: number,
): void {
  if (payload.type === "enter" || payload.type === "over") {
    const inside = isPointInElement(payload.position, el, devicePixelRatio);
    callbacks.onDragChange?.(inside);
  } else if (payload.type === "leave") {
    callbacks.onDragChange?.(false);
  } else if (payload.type === "drop") {
    const inside = isPointInElement(payload.position, el, devicePixelRatio);
    callbacks.onDragChange?.(false);
    if (inside && payload.paths.length > 0) {
      callbacks.onDrop(payload.paths);
    }
  }
}

/**
 * Subscribes a dropzone element to Tauri's native `onDragDropEvent` stream.
 * Returns an unsubscribe function suitable for Svelte `$effect` cleanup.
 */
export function subscribeDropzone(
  getElement: () => HTMLElement | null,
  callbacks: DropzoneCallbacks,
): () => void {
  let cancelled = false;
  let unlisten: (() => void) | null = null;

  try {
    getCurrentWebview()
      .onDragDropEvent((event) => {
        if (cancelled) return;
        handleDragDropPayload(event.payload, getElement(), callbacks);
      })
      .then((fn) => {
        if (cancelled) {
          fn();
        } else {
          unlisten = fn;
        }
      })
      .catch(() => {
        // Non-Tauri environment (e.g. unit tests or standalone browser preview).
      });
  } catch {
    // Non-Tauri environment.
  }

  return () => {
    cancelled = true;
    callbacks.onDragChange?.(false);
    if (unlisten) {
      unlisten();
      unlisten = null;
    }
  };
}
