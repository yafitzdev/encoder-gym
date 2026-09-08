import type { Location, Page } from "./actions.js";

interface HistoryEntry { location: Location; scroll: number; focusId?: string }

export class NavigationHistory {
  private entries: HistoryEntry[] = [{ location: { page: "models" }, scroll: 0 }];
  private index = 0;
  get current(): Location { return this.entries[this.index]!.location; }
  get bookmark(): HistoryEntry { return this.entries[this.index]!; }
  capture(scroll: number, focusId?: string): void {
    this.bookmark.scroll = scroll;
    if (focusId) this.bookmark.focusId = focusId;
  }
  remember(location: Location, scroll: number, focusId?: string): void {
    this.capture(scroll, focusId);
    if (JSON.stringify(location) === JSON.stringify(this.current)) return;
    this.entries.splice(this.index + 1);
    this.entries.push({ location, scroll: 0 });
    this.index++;
  }
  returnTo(page: Page, scroll: number): HistoryEntry | undefined {
    const target = this.entries.slice(0, this.index).findLastIndex(entry => entry.location.page === page);
    if (target < 0) return undefined;
    this.capture(scroll); this.index = target;
    return this.bookmark;
  }
  move(offset: -1 | 1, scroll: number): HistoryEntry | undefined {
    if (!this.canNavigate(offset)) return undefined;
    this.capture(scroll);
    this.index += offset;
    return this.entries[this.index];
  }
  canNavigate(offset: -1 | 1): boolean { return this.index + offset >= 0 && this.index + offset < this.entries.length; }
}
