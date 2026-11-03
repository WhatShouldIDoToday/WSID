import { describe, expect, test } from "bun:test";
import vectors from "../../../../../test-fixtures/validation.json";
import {
  isPastDeadline,
  parseDurationInput,
  validateDeadlineOrder,
  validateTag,
  validateUsername,
} from "./ipc";
import { handleDragDropPayload, hitTestPhysicalPoint } from "./filedrop";
import { PhysicalPosition } from "@tauri-apps/api/dpi";

// Shared with Rust (`wsid-core` test `matches_shared_validation_vectors`) so both validators agree.
describe("shared validation vectors", () => {
  for (const c of vectors.usernames) {
    test(`username ${JSON.stringify(c.input)}`, () => {
      expect(validateUsername(c.input).normalized).toBe(c.normalized);
    });
  }

  for (const c of vectors.tags) {
    test(`tag ${JSON.stringify(c.input)}`, () => {
      expect(validateTag(c.input).normalized).toBe(c.normalized);
    });
  }

  for (const c of vectors.durations) {
    test(`duration ${JSON.stringify(c.input)}`, () => {
      expect(parseDurationInput(c.input).minutes).toBe(c.minutes);
    });
  }

  for (const c of vectors.deadline_order) {
    test(`deadline order ${c.sgdb} / ${c.due}`, () => {
      expect(validateDeadlineOrder(c.sgdb, c.due) === null).toBe(c.valid);
    });
  }

  for (const c of vectors.past_deadlines) {
    test(`past deadline ${c.value}`, () => {
      expect(isPastDeadline(c.value)).toBe(c.past);
    });
  }
});

describe("filedrop hit-testing and event routing", () => {
  const rect = {
    left: 100,
    top: 200,
    right: 300,
    bottom: 400,
    width: 200,
    height: 200,
  };

  test("converts physical coordinates using devicePixelRatio", () => {
    expect(hitTestPhysicalPoint({ x: 300, y: 500 }, rect, 2)).toBe(true);
    expect(hitTestPhysicalPoint({ x: 300, y: 500 }, rect, 1)).toBe(false);
  });

  test("routes over, leave, and drop events to dropzone callbacks", () => {
    const fakeEl = {
      getBoundingClientRect: () => rect,
      contains: () => true,
    } as unknown as HTMLElement;

    let dragging = false;
    let dropped: string[] = [];

    handleDragDropPayload(
      { type: "over", position: new PhysicalPosition(300, 500) },
      fakeEl,
      {
        onDragChange: (d) => (dragging = d),
        onDrop: (paths) => (dropped = paths),
      },
      2,
    );
    expect(dragging).toBe(true);

    handleDragDropPayload(
      { type: "over", position: new PhysicalPosition(50, 50) },
      fakeEl,
      {
        onDragChange: (d) => (dragging = d),
        onDrop: (paths) => (dropped = paths),
      },
      2,
    );
    expect(dragging).toBe(false);

    handleDragDropPayload(
      {
        type: "drop",
        paths: ["/Users/alice/Downloads/spec.pdf"],
        position: new PhysicalPosition(300, 500),
      },
      fakeEl,
      {
        onDragChange: (d) => (dragging = d),
        onDrop: (paths) => (dropped = paths),
      },
      2,
    );
    expect(dragging).toBe(false);
    expect(dropped).toEqual(["/Users/alice/Downloads/spec.pdf"]);
  });
});
