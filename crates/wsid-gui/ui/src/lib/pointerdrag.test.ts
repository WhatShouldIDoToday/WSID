import { describe, expect, test } from "bun:test";
import {
  DRAG_THRESHOLD_PX,
  advancePointerDrag,
  beginPointerDrag,
} from "./pointerdrag";

const element = {
  getBoundingClientRect: () => ({
    left: 100,
    top: 200,
    right: 340,
    bottom: 300,
    width: 240,
    height: 100,
  }),
} as unknown as Element;

const pointer = (clientX: number, clientY: number) =>
  ({ clientX, clientY }) as PointerEvent;

describe("pointer drag geometry", () => {
  test("records the grab offset and card width so the ghost sits under the pointer", () => {
    const drag = beginPointerDrag({ id: "AB1234" }, pointer(150, 240), element);
    expect(drag.offsetX).toBe(50);
    expect(drag.offsetY).toBe(40);
    expect(drag.width).toBe(240);
    expect(drag.active).toBe(false);
  });

  test("falls back to a usable width when the element is gone", () => {
    const drag = beginPointerDrag({ id: "AB1234" }, pointer(10, 20), null);
    expect(drag.width).toBe(220);
    expect(drag.offsetX).toBe(0);
    expect(drag.active).toBe(false);
  });

  test("only becomes active past the movement threshold", () => {
    const drag = beginPointerDrag({ id: "AB1234" }, pointer(150, 240), element);
    expect(advancePointerDrag(drag, pointer(152, 241))).toBe(false);
    expect(drag.active).toBe(false);

    const justOver = DRAG_THRESHOLD_PX + 1;
    expect(advancePointerDrag(drag, pointer(150 + justOver, 240))).toBe(true);
    expect(drag.active).toBe(true);
    expect(drag.x).toBe(150 + justOver);

    // Stays active once started, even if the pointer returns to the origin.
    expect(advancePointerDrag(drag, pointer(150, 240))).toBe(true);
  });
});
