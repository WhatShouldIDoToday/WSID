import { describe, expect, test } from "bun:test";
import {
  describeUserNotice,
  editDistance,
  parseCommentMentions,
  plainTextPreview,
  suggestKinds,
  withUserNotices,
} from "./ipc";

describe("user kinds", () => {
  test("edit distance counts single-character edits", () => {
    expect(editDistance("claud", "claude")).toBe(1);
    expect(editDistance("", "abc")).toBe(3);
    expect(editDistance("same", "same")).toBe(0);
  });

  test("suggests close known kinds nearest first, never the exact kind", () => {
    expect(suggestKinds("claud", ["human", "claude", "jira"])).toEqual([
      "claude",
    ]);
    expect(suggestKinds("claude", ["human", "claude"])).toEqual([]);
    expect(suggestKinds("xyz", ["human", "claude"])).toEqual([]);
  });

  test("typed mention exposes kind, mismatch, and suggestions", () => {
    const users = ["bob", "carl"];
    const kinds = { bob: "claude", carl: "human" };

    const [bob] = parseCommentMentions("@human:bob", users, kinds);
    expect(bob.username).toBe("bob");
    expect(bob.typedKind).toBe("human");
    expect(bob.kindMismatch).toBe("claude");
    expect(bob.tooltip).toContain("is a claude user");

    const [carl] = parseCommentMentions("@claud:carl", users, kinds);
    expect(carl.typedKind).toBe("claud");
    expect(carl.kindSuggestions).toEqual(["claude"]);
    expect(carl.tooltip).toContain('Did you mean "claude"?');
  });

  test("takenote directive combines with a typed kind", () => {
    const [mention] = parseCommentMentions(
      "@takenote:claude:dana",
      ["dana"],
      {},
    );
    expect(mention.kind).toBe("takenote");
    expect(mention.username).toBe("dana");
    expect(mention.typedKind).toBe("claude");
  });

  test("describes notices for the live region and alerts", () => {
    expect(
      describeUserNotice({
        type: "created",
        user: "bob",
        kind: "claude",
        new_kind: false,
      }),
    ).toBe("Created @bob as claude.");
    expect(
      describeUserNotice({
        type: "created",
        user: "bob",
        kind: "claud",
        new_kind: true,
      }),
    ).toBe('Created @bob with new user kind "claud".');
    expect(
      describeUserNotice({
        type: "kind_mismatch",
        user: "bob",
        requested: "human",
        actual: "claude",
      }),
    ).toBe('@bob is a claude user; "human:" was ignored.');
    expect(withUserNotices("Saved", [])).toBe("Saved");
  });
});

describe("plain text preview", () => {
  test("strips markdown and keeps mentions and task references as plain text", () => {
    expect(
      plainTextPreview(
        "## Plan\n\n- **Ship** the [docs](https://example.test) for @bob, see #AB1234",
      ),
    ).toBe("Plan Ship the docs for @bob, see #AB1234");
  });

  test("truncates with an ellipsis and never exceeds the limit", () => {
    const preview = plainTextPreview("word ".repeat(100), 20);
    expect(preview.endsWith("…")).toBe(true);
    expect(preview.length).toBeLessThanOrEqual(20);
    expect(plainTextPreview("", 20)).toBe("");
  });
});
