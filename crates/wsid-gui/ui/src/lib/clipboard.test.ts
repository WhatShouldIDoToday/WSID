import { describe, expect, test } from "bun:test";
import {
  clipboardHasText,
  extractClipboardAttachments,
  fileUriToPath,
  handleClipboardPaste,
  parseUriListToPaths,
  type ClipboardLike,
} from "./clipboard";

const png = (name = "shot.png") => new File(["x"], name, { type: "image/png" });

const item = (kind: string, type: string, file: File | null) =>
  ({ kind, type, getAsFile: () => file }) as unknown as DataTransferItem;

const clip = (opts: {
  files?: File[];
  items?: DataTransferItem[];
  data?: Record<string, string>;
}): ClipboardLike => ({
  files: opts.files ?? [],
  items: opts.items ?? [],
  types: Object.keys(opts.data ?? {}),
  getData: (f: string) => opts.data?.[f] ?? "",
});

describe("fileUriToPath", () => {
  test("decodes file URIs", () => {
    expect(fileUriToPath("file:///home/u/My%20Pic%C3%A9.png")).toBe(
      "/home/u/My Picé.png",
    );
    expect(fileUriToPath("file://localhost/tmp/a")).toBe("/tmp/a");
    expect(fileUriToPath("file:///C:/Users/a.txt")).toBe("C:/Users/a.txt");
  });
  test("rejects non-local URIs", () => {
    expect(fileUriToPath("https://example.com/a.png")).toBeNull();
    expect(fileUriToPath("file://server/share/a")).toBeNull();
    expect(fileUriToPath("not a uri")).toBeNull();
  });
});

describe("parseUriListToPaths", () => {
  test("handles uri-list and gnome-copied-files", () => {
    expect(
      parseUriListToPaths(
        "# c\r\nfile:///a%20b\r\nhttp://x/y\r\nfile:///c\r\n",
      ),
    ).toEqual(["/a b", "/c"]);
    expect(parseUriListToPaths("copy\nfile:///a\nfile:///b")).toEqual([
      "/a",
      "/b",
    ]);
  });
});

describe("extractClipboardAttachments", () => {
  test("prefers files", () => {
    const f = png();
    const r = extractClipboardAttachments(
      clip({ files: [f], items: [item("file", "image/png", png("o"))] }),
    );
    expect(r.files).toEqual([f]);
    expect(r.paths).toEqual([]);
  });
  test("falls back to file items", () => {
    const f = png();
    const r = extractClipboardAttachments(
      clip({
        items: [
          item("string", "text/plain", null),
          item("file", "image/png", f),
        ],
      }),
    );
    expect(r.files).toEqual([f]);
  });
  test("reads image-typed items even when kind is not file", () => {
    const f = png();
    const r = extractClipboardAttachments(
      clip({ items: [item("string", "image/png", f)] }),
    );
    expect(r.files).toEqual([f]);
  });
  test("falls back to uri-list paths", () => {
    const r = extractClipboardAttachments(
      clip({ data: { "text/uri-list": "file:///home/u/a.pdf\n" } }),
    );
    expect(r.files).toEqual([]);
    expect(r.paths).toEqual(["/home/u/a.pdf"]);
  });
  test("falls back to gnome-copied-files", () => {
    const r = extractClipboardAttachments(
      clip({ data: { "x-special/gnome-copied-files": "copy\nfile:///a" } }),
    );
    expect(r.paths).toEqual(["/a"]);
  });
  test("returns nothing for plain text or null", () => {
    expect(
      extractClipboardAttachments(clip({ data: { "text/plain": "hi" } })),
    ).toEqual({ files: [], paths: [] });
    expect(extractClipboardAttachments(null)).toEqual({ files: [], paths: [] });
  });
});

describe("handleClipboardPaste", () => {
  const makeEvent = (data: ClipboardLike) => {
    const state = { prevented: false };
    return {
      state,
      event: {
        clipboardData: data as unknown as DataTransfer,
        preventDefault: () => {
          state.prevented = true;
        },
      },
    };
  };

  test("does not intercept plain text", () => {
    const { event, state } = makeEvent(clip({ data: { "text/plain": "hi" } }));
    let called = false;
    const handled = handleClipboardPaste(event, {
      onFiles: () => void (called = true),
      onPaths: () => void (called = true),
      readNativeImage: async () => {
        called = true;
        return null;
      },
    });
    expect(handled).toBe(false);
    expect(state.prevented).toBe(false);
    expect(called).toBe(false);
  });

  test("intercepts files and paths", () => {
    const got: unknown[] = [];
    const a = makeEvent(clip({ files: [png()] }));
    expect(
      handleClipboardPaste(a.event, {
        onFiles: (f) => void got.push(f.length),
        onPaths: (p) => void got.push(p),
      }),
    ).toBe(true);
    expect(a.state.prevented).toBe(true);
    const b = makeEvent(clip({ data: { "text/uri-list": "file:///x" } }));
    handleClipboardPaste(b.event, {
      onFiles: () => {},
      onPaths: (p) => void got.push(p),
    });
    expect(b.state.prevented).toBe(true);
    expect(got).toEqual([1, ["/x"]]);
  });

  test("uses the native image fallback when the clipboard has no text", async () => {
    const f = png();
    const { event, state } = makeEvent(clip({ data: {} }));
    const got: File[][] = [];
    handleClipboardPaste(event, {
      onFiles: (files) => void got.push(files),
      onPaths: () => {},
      readNativeImage: async () => f,
    });
    await new Promise((r) => setTimeout(r, 0));
    expect(state.prevented).toBe(false);
    expect(got).toEqual([[f]]);
  });

  test("clipboardHasText detects text types", () => {
    expect(clipboardHasText(clip({ data: { "text/html": "<b>" } }))).toBe(true);
    expect(clipboardHasText(clip({ data: { "image/png": "" } }))).toBe(false);
  });
});
