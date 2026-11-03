import { describe, expect, test } from "bun:test";
import vectors from "../../../../../test-fixtures/validation.json";
import {
  isPastDeadline,
  parseDurationInput,
  validateDeadlineOrder,
  validateTag,
  userKindMismatch,
  validateUserRef,
  validateUsername,
} from "./ipc";
import {
  handleDragDropPayload,
  hitTestPhysicalPoint,
  isPointInElement,
  toCssPoint,
} from "./filedrop";
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

  const fakeElement = (bounds: typeof rect) =>
    ({
      getBoundingClientRect: () => bounds,
      contains: () => true,
    }) as unknown as HTMLElement;

  test("converts physical coordinates using devicePixelRatio", () => {
    expect(hitTestPhysicalPoint({ x: 300, y: 500 }, rect, 2)).toBe(true);
    expect(hitTestPhysicalPoint({ x: 300, y: 500 }, rect, 1)).toBe(false);
  });

  test("does not re-scale macOS logical points on a Retina display", () => {
    // wry/WKWebView already reports logical points; halving them broke every drop.
    const logical = toCssPoint({ x: 250, y: 350 }, 2, { macLike: true });
    expect(logical).toEqual({ x: 250, y: 350 });
    expect(
      isPointInElement({ x: 250, y: 350 }, fakeElement(rect), 2, {
        macLike: true,
      }),
    ).toBe(true);
    expect(
      isPointInElement({ x: 50, y: 50 }, fakeElement(rect), 2, {
        macLike: true,
      }),
    ).toBe(false);

    // A macOS point outside the viewport can only be physical, so it is scaled.
    const scaled = toCssPoint({ x: 3000, y: 5000 }, 2, {
      macLike: true,
      viewport: { width: 1440, height: 900 },
    });
    expect(scaled).toEqual({ x: 1500, y: 2500 });

    // Non-macOS platforms keep the physical -> CSS conversion.
    expect(toCssPoint({ x: 300, y: 500 }, 2, { macLike: false })).toEqual({
      x: 150,
      y: 250,
    });
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

describe("typed user references", () => {
  const ok: [string, string][] = [
    ["bob", "bob"],
    ["@Bob", "bob"],
    ["claude:bob", "claude:bob"],
    ["@Claude:Bob", "claude:bob"],
    ["jira:rob_2", "jira:rob_2"],
  ];
  for (const [input, normalized] of ok) {
    test(`accepts ${JSON.stringify(input)}`, () => {
      const r = validateUserRef(input);
      expect(r.error).toBeNull();
      expect(r.normalized).toBe(normalized);
    });
  }

  const bad: [string, RegExp][] = [
    ["claude:bob:x", /single "kind:user"/],
    [":bob", /Kind cannot be empty/],
    ["bad kind:bob", /Invalid kind "bad kind"/],
    ["claude:bo b", /^Username: /],
    ["takenote:bob", /reserved/],
    ["claude:all", /Username: .*reserved/],
    ["all", /reserved/],
  ];
  for (const [input, pattern] of bad) {
    test(`rejects ${JSON.stringify(input)} naming the wrong part`, () => {
      const r = validateUserRef(input);
      expect(r.normalized).toBeNull();
      expect(r.error).toMatch(pattern);
    });
  }
});

describe("user kind mismatch warning", () => {
  const kinds = { bob: "claude", rob: "jira" };
  const known = ["bob", "rob", "davidb"];

  test("warns when the typed kind differs from the existing user", () => {
    expect(userKindMismatch("jira:bob", known, kinds)).toBe(
      '@bob is an existing claude user; "jira:" would be ignored.',
    );
  });

  test("no warning when the kind matches, the user is new, or no kind is typed", () => {
    expect(userKindMismatch("claude:bob", known, kinds)).toBeNull();
    expect(userKindMismatch("claude:newperson", known, kinds)).toBeNull();
    expect(userKindMismatch("bob", known, kinds)).toBeNull();
    expect(userKindMismatch("human:davidb", known, kinds)).toBeNull();
  });

  test("defaults unknown kinds to human", () => {
    expect(userKindMismatch("claude:davidb", known, kinds)).toMatch(
      /existing human user/,
    );
  });
});
