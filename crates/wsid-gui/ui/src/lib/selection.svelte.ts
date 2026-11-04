/** Reactive multi-select state with shift-range support, shared by list-like views. */
export class Selection {
  ids = $state<string[]>([]);
  private anchor: string | null = null;

  get count(): number {
    return this.ids.length;
  }

  has(id: string): boolean {
    return this.ids.includes(id);
  }

  toggle(id: string) {
    this.ids = this.has(id)
      ? this.ids.filter((x) => x !== id)
      : [...this.ids, id];
    this.anchor = id;
  }

  /** Selects the inclusive range between the last-clicked id and `id` within `ordered`. */
  range(id: string, ordered: string[]) {
    const from = this.anchor ? ordered.indexOf(this.anchor) : -1;
    const to = ordered.indexOf(id);
    if (from < 0 || to < 0) {
      this.toggle(id);
      return;
    }
    const [lo, hi] = from < to ? [from, to] : [to, from];
    this.ids = [...new Set([...this.ids, ...ordered.slice(lo, hi + 1)])];
  }

  /** Click handler helper: shift selects a range, anything else toggles. */
  click(e: { shiftKey: boolean }, id: string, ordered: string[]) {
    if (e.shiftKey) this.range(id, ordered);
    else this.toggle(id);
  }

  set(ids: string[]) {
    this.ids = [...new Set(ids)];
  }

  clear() {
    this.ids = [];
    this.anchor = null;
  }

  /** Drops ids that no longer exist in `valid`. */
  prune(valid: string[]) {
    const keep = new Set(valid);
    if (this.ids.some((id) => !keep.has(id))) {
      this.ids = this.ids.filter((id) => keep.has(id));
    }
  }
}
