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

export interface PointLike {
  x: number;
  y: number;
}

export interface CoordinateOptions {
  /** Viewport size in CSS pixels; defaults to the live `window` size. */
  viewport?: { width: number; height: number };
  /**
   * Whether the webview reports drag positions in logical (CSS) points, as
   * `wry`/WKWebView does on macOS. Defaults to sniffing the user agent.
   */
  macLike?: boolean;
}

function defaultsToMacLike(): boolean {
  return typeof navigator !== "undefined" && /Mac/i.test(navigator.userAgent);
}

/**
 * Normalizes a drag-drop position to CSS pixels.
 *
 * `wry` on macOS reports `draggingLocation()` in WKWebView logical points (CSS
 * pixels) even though Tauri types the position as `PhysicalPosition`. Dividing
 * by `devicePixelRatio` there halves every coordinate on Retina and no drop
 * ever hits its zone — the original bug. On other platforms the position really
 * is physical, so it is scaled. A macOS-reported point that falls outside the
 * viewport can only be physical, so it is scaled too.
 */
export function toCssPoint(
  position: PointLike,
  devicePixelRatio = 1,
  options: CoordinateOptions = {},
): PointLike {
  const dpr = devicePixelRatio > 0 ? devicePixelRatio : 1;
  const macLike = options.macLike ?? defaultsToMacLike();
  const viewport =
    options.viewport ??
    (typeof window !== "undefined"
      ? { width: window.innerWidth, height: window.innerHeight }
      : { width: Number.POSITIVE_INFINITY, height: Number.POSITIVE_INFINITY });

  const outsideViewport =
    position.x > viewport.width || position.y > viewport.height;

  if (!macLike || outsideViewport) {
    return { x: position.x / dpr, y: position.y / dpr };
  }
  return { x: position.x, y: position.y };
}

/** Hit-tests a CSS-pixel point against a `getBoundingClientRect()`-shaped rect. */
export function hitTestPoint(
  point: PointLike,
  rect: BoundingRectLike,
): boolean {
  if (rect.width <= 0 || rect.height <= 0) return false;
  return (
    point.x >= rect.left &&
    point.x <= rect.right &&
    point.y >= rect.top &&
    point.y <= rect.bottom
  );
}

/**
 * Converts a physical pixel coordinate into CSS pixels using
 * `devicePixelRatio` and checks whether it falls within `rect`.
 */
export function hitTestPhysicalPoint(
  position: PointLike,
  rect: BoundingRectLike,
  devicePixelRatio = 1,
): boolean {
  const dpr = devicePixelRatio > 0 ? devicePixelRatio : 1;
  return hitTestPoint({ x: position.x / dpr, y: position.y / dpr }, rect);
}

/**
 * Hit-tests a drag position against a DOM dropzone element, ensuring the
 * element is not occluded by a modal backdrop above it.
 */
export function isPointInElement(
  position: PointLike,
  el: HTMLElement | null,
  devicePixelRatio?: number,
  options: CoordinateOptions = {},
): boolean {
  if (!el) return false;
  const dpr =
    devicePixelRatio ??
    ((typeof window !== "undefined" && window.devicePixelRatio) || 1);
  const point = toCssPoint(position, dpr, options);
  const rect = el.getBoundingClientRect();
  if (!hitTestPoint(point, rect)) {
    return false;
  }
  if (
    typeof document !== "undefined" &&
    typeof document.elementFromPoint === "function"
  ) {
    const hit = document.elementFromPoint(point.x, point.y);
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
  options: CoordinateOptions = {},
): void {
  if (payload.type === "enter" || payload.type === "over") {
    const inside = isPointInElement(
      payload.position,
      el,
      devicePixelRatio,
      options,
    );
    callbacks.onDragChange?.(inside);
  } else if (payload.type === "leave") {
    callbacks.onDragChange?.(false);
  } else if (payload.type === "drop") {
    const inside = isPointInElement(
      payload.position,
      el,
      devicePixelRatio,
      options,
    );
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
