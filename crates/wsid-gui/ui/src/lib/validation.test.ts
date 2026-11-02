import { describe, expect, test } from "bun:test";
import vectors from "../../../../../test-fixtures/validation.json";
import {
  isPastDeadline,
  parseDurationInput,
  validateDeadlineOrder,
  validateTag,
  validateUsername,
} from "./ipc";

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
