/** Attachments found on a paste event's clipboard. */
export interface ClipboardAttachments {
  /** In-memory files (screenshots, copied images, files WebKit exposes). */
  files: File[];
  /** Local filesystem paths from copied files (`text/uri-list` and friends). */
  paths: string[];
}

export interface ClipboardLike {
  files?: ArrayLike<File> | null;
  items?: ArrayLike<DataTransferItem> | null;
  types?: ArrayLike<string> | null;
  getData?: (format: string) => string;
}

/**
 * Converts a `file://` URI into a local path, or `null` for any other string.
 * Handles percent-encoding, `localhost` hosts, and Windows drive letters.
 */
export function fileUriToPath(uri: string): string | null {
  const trimmed = uri.trim();
  if (!/^file:\/\//i.test(trimmed)) return null;
  let url: URL;
  try {
    url = new URL(trimmed);
  } catch {
    return null;
  }
  if (url.host && url.host.toLowerCase() !== "localhost") return null;
  let path: string;
  try {
    path = decodeURIComponent(url.pathname);
  } catch {
    return null;
  }
  if (/^\/[A-Za-z]:/.test(path)) path = path.slice(1);
  return path || null;
}

/**
 * Parses `text/uri-list` or GNOME's `x-special/gnome-copied-files` payload
 * (`copy\nfile:///a\nfile:///b`) into local paths. Comments, the `copy`/`cut`
 * verb line, and non-`file://` URIs are ignored.
 */
export function parseUriListToPaths(raw: string): string[] {
  const paths: string[] = [];
  for (const line of raw.split(/\r?\n/)) {
    if (!line || line.startsWith("#")) continue;
    const path = fileUriToPath(line);
    if (path) paths.push(path);
  }
  return paths;
}

function readData(data: ClipboardLike, format: string): string {
  try {
    return data.getData?.(format) ?? "";
  } catch {
    return "";
  }
}

/**
 * Extracts attachments from a clipboard, in order of preference:
 * `files`, then `items` that are files or images (`getAsFile()`), then
 * `file://` URIs from `text/uri-list` / `x-special/gnome-copied-files`.
 * WebKitGTK usually leaves `files` empty for screenshots and for files copied
 * in a file manager, hence the fallbacks.
 */
export function extractClipboardAttachments(
  data: ClipboardLike | null | undefined,
): ClipboardAttachments {
  const result: ClipboardAttachments = { files: [], paths: [] };
  if (!data) return result;

  if (data.files && data.files.length > 0) {
    result.files = Array.from(data.files);
    return result;
  }

  if (data.items) {
    for (const item of Array.from(data.items)) {
      if (item.kind !== "file" && !item.type?.startsWith("image/")) continue;
      let file: File | null = null;
      try {
        file = item.getAsFile();
      } catch {
        file = null;
      }
      if (file) result.files.push(file);
    }
    if (result.files.length > 0) return result;
  }

  for (const format of ["text/uri-list", "x-special/gnome-copied-files"]) {
    const paths = parseUriListToPaths(readData(data, format));
    if (paths.length > 0) {
      result.paths = paths;
      return result;
    }
  }
  return result;
}

/** Whether the clipboard carries anything a text field would paste. */
export function clipboardHasText(data: ClipboardLike | null | undefined) {
  if (!data?.types) return false;
  return Array.from(data.types).some(
    (t) => t === "text/plain" || t === "text/html" || t === "Text",
  );
}

export interface PasteHandlers {
  onFiles: (files: File[]) => void | Promise<void>;
  onPaths: (paths: string[]) => void | Promise<void>;
  /**
   * Native fallback for clipboards the webview cannot read (for example a
   * Wayland screenshot). Resolves to an image file or `null`.
   */
  readNativeImage?: () => Promise<File | null>;
}

/**
 * Shared `paste` handler. Only calls `preventDefault()` when an attachment was
 * found, so plain-text pastes keep working. Returns whether the event was
 * handled synchronously. When the webview exposes nothing and the clipboard
 * has no text, tries the native fallback asynchronously.
 */
export function handleClipboardPaste(
  event: Pick<ClipboardEvent, "clipboardData" | "preventDefault">,
  handlers: PasteHandlers,
): boolean {
  const data = event.clipboardData;
  const found = extractClipboardAttachments(data);
  if (found.files.length > 0) {
    event.preventDefault();
    void handlers.onFiles(found.files);
    return true;
  }
  if (found.paths.length > 0) {
    event.preventDefault();
    void handlers.onPaths(found.paths);
    return true;
  }
  if (handlers.readNativeImage && !clipboardHasText(data)) {
    const read = handlers.readNativeImage;
    void read()
      .then((file) => (file ? handlers.onFiles([file]) : undefined))
      .catch(() => {
        // No native clipboard image available; nothing to attach.
      });
  }
  return false;
}
