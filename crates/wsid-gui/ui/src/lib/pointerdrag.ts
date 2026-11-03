/**
 * Shared pointer-drag bookkeeping for the Kanban board and the Eisenhower matrix.
 *
 * Both views move cards between containers with the mouse. Native HTML5 drag is
 * unavailable inside the Tauri webview (the file-drop interceptor claims every
 * drag), so each view follows the pointer itself and renders a floating ghost
 * card. This module owns the geometry; the views own the drop targets.
 */
export interface PointerDragState<T> {
  item: T;
  /** Pointer position where the drag started, used for the movement threshold. */
  startX: number;
  startY: number;
  /** Current pointer position, in viewport (CSS) pixels. */
  x: number;
  y: number;
  /** Where inside the source card the pointer grabbed it. */
  offsetX: number;
  offsetY: number;
  /** Source card width, so the ghost matches the card it replaces. */
  width: number;
  active: boolean;
}

/** Movement (px) before a press becomes a drag, so clicks and taps still work. */
export const DRAG_THRESHOLD_PX = 6;

export function beginPointerDrag<T>(
  item: T,
  event: PointerEvent,
  element: Element | null,
): PointerDragState<T> {
  const rect = element?.getBoundingClientRect();
  return {
    item,
    startX: event.clientX,
    startY: event.clientY,
    x: event.clientX,
    y: event.clientY,
    offsetX: rect ? event.clientX - rect.left : 0,
    offsetY: rect ? event.clientY - rect.top : 0,
    width: rect ? rect.width : 220,
    active: false,
  };
}

/** Updates the pointer position; returns true once the threshold has been crossed. */
export function advancePointerDrag<T>(
  drag: PointerDragState<T>,
  event: PointerEvent,
): boolean {
  drag.x = event.clientX;
  drag.y = event.clientY;
  if (!drag.active) {
    const dx = event.clientX - drag.startX;
    const dy = event.clientY - drag.startY;
    if (Math.hypot(dx, dy) > DRAG_THRESHOLD_PX) {
      drag.active = true;
    }
  }
  return drag.active;
}
