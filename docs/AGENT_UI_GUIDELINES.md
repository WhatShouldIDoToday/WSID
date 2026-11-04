# Agent UI Guidelines

When generating or refactoring user interfaces for WSID (both Tauri GUI and
Ratatui TUI), adhere strictly to the following principles to eliminate visual
clutter, protect user focus, and maintain high accessibility standards.

---

## 1. Administrative Metadata & Progressive Disclosure

- **Rule:** **Do not expose system internals in the primary header.** File paths
  (`/Users/.../wsid.db`), active user tags (`@davidb`), database connection
  buttons, and settings belong in secondary surfaces.
- **Actionable Implementation:**
  - **Tauri GUI:** Remove path selectors, DB dropdowns, and user input bars from
    the top bar. Move administrative features into the native OS Menu Bar
    (`File` / `Help`), a dedicated Settings view (`Cmd+,` / `Ctrl+,`), and a
    subtle bottom-right active user status indicator (`@username`). The header
    bar should only contain essential branding and view navigation.
  - **Ratatui TUI:** Clean up the top status line. Display only the application
    name and active view tab. Hide full filesystem paths completely; if
    necessary, display only a shortened DB moniker (e.g., `[wsid.db]`) or
    relegate it to the `?` keybindings/status modal and bottom status bar.

---

## 2. Border & Panel Hygiene (Eliminate "Box-in-Box" Syndrome)

- **Rule:** **Borders consume valuable cognitive bandwidth.** Do not enclose
  every section, header, and sub-pane in heavy outlines.
- **Actionable Implementation:**
  - **Ratatui TUI:**
    - Avoid wrapping every `Layout` block in
      `Block::default().borders(Borders::ALL)`.
    - Use padding, empty line spacing, or single horizontal/vertical dividers
      (`Borders::BOTTOM` or `Borders::LEFT`) to delimit major structural areas.
    - Collapse empty secondary panes dynamically: if "Situational Awareness" has
      0 items, yield its screen space to the primary Focus Queue rather than
      rendering an empty box.
  - **Tauri GUI:**
    - Replace heavy structural borders with subtle surface background colors,
      soft shadows, or whitespace separation (while preserving explicit `2px`
      borders when High Contrast or Forced Colors mode is active).
    - Eliminate nested card containers inside empty states.
    - Use full-height horizontal scrolling (`overflow-x: auto`) for multi-column
      Kanban boards rather than wrapping columns vertically.

---

## 3. Keybindings & Instruction Decoupling

- **Rule:** **Do not embed inline keybinding instructions into panel titles or
  subheaders.** (e.g., Avoid: `Focus Queue (Top 0, [p] cycle preset...)`).
- **Actionable Implementation:**
  - Keep panel titles strictly nominal (e.g., `Focus Queue`).
  - Relegate keyboard shortcuts to a dedicated, unobtrusive footer keymap bar at
    the bottom of the screen or display them in a standard help overlay (`?`).
  - In the GUI, rely on standard hover tooltips (`title`) and
    `aria-keyshortcuts` on navigation tabs and interactive buttons.

---

## 4. Zero-Redundancy Copy & Minimalist Empty States

- **Rule:** **Never state the obvious or repeat the user/preset state across
  multiple UI labels.**
- **Actionable Implementation:**
  - Eliminate repetitive subtext like *"Showing top actionable unblocked tasks
    for @davidb under preset Balanced."* The user selected the preset and active
    user handle—they do not need it restated on every screen.
  - Keep empty states clean, centered, and concise:
    - **Avoid:** *"No Actionable Unblocked Tasks — @davidb currently has no
      unblocked To Do or In Progress tasks."*
    - **Use:** *"All caught up. No pending tasks in focus."*

---

## 5. Accessible-by-Default Architecture (Visual Reduction != Accessibility Reduction)

- **Rule:** **Removing visual elements must never reduce information
  availability for screen readers or low-vision users.**
- **Actionable Implementation:**
  - **Tauri GUI:**
    - Use visually hidden elements (`class="sr-only"`) to preserve structural
      context for screen readers (e.g., announcing the current user and database
      state) without taking up visual pixel real estate.
    - Ensure all controls use proper semantic tags (`<nav>`, `<main>`,
      `<button>`, `<dialog>`) with clear `aria-label`, `aria-current`,
      `aria-keyshortcuts`, and `aria-live` region updates.
    - Support system color schemes (`prefers-color-scheme: light` / `dark`),
      system contrast preferences (`prefers-contrast: more`), OS forced colors
      (`forced-colors: active` using CSS System Colors and explicit `outline`
      focus rings), colorblind-safe palettes (Okabe-Ito), and relative `rem`/`em`
      typography without hardcoded root `px` overrides.
  - **Ratatui TUI:**
    - Maintain the dedicated `--accessible` plain text rendering mode (`a` key
      toggle) for terminal screen readers, bypassing multi-column layouts and
      box-drawing characters in favor of a sequential, linear text stream.

---

## Visual Contrast Checklist for Agents

| UI Element | Cluttered Pattern (Avoid) | Clean & Accessible Pattern (Use) |
| --- | --- | --- |
| **TUI Borders** | Double-nested `Borders::ALL` around every pane | Subtle background style differences, `Borders::NONE`, or single dividers |
| **Headers** | `WSID \| User: @davidb \| Preset: balanced \| DB: /long/path/...` | `WSID  Focus  Kanban  Eisenhower  Database` |
| **Panel Titles** | `Focus Queue (Top 0, [p] cycle preset, [space] complete)` | `Focus Queue` (Keymap moved to footer or `?` overlay) |
| **Empty States** | Card box with title + subheader + status text explaining zero state | Centered single-line text in dimmed visual color |
| **GUI Controls** | Multi-button bar with File/DB/User dropdowns in app header | Native OS Menus, Settings (`Cmd+,`), and subtle bottom-corner `@user` pill |
