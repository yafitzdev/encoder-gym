import type { Location } from "./actions.js";

export class NavigationHistory {
  private entries: { location: Location; scroll: number }[] = [{ location: { page: "models" }, scroll: 0 }];
  private index = 0;
  get current(): Location { return this.entries[this.index]!.location; }
  remember(location: Location, scroll: number): void {
    this.entries[this.index]!.scroll = scroll;
    if (JSON.stringify(location) === JSON.stringify(this.current)) return;
    this.entries.splice(this.index + 1);
    this.entries.push({ location, scroll: 0 });
    this.index++;
  }
  move(offset: -1 | 1, scroll: number): { location: Location; scroll: number } | undefined {
    if (!this.canNavigate(offset)) return undefined;
    this.entries[this.index]!.scroll = scroll;
    this.index += offset;
    return this.entries[this.index];
  }
  canNavigate(offset: -1 | 1): boolean { return this.index + offset >= 0 && this.index + offset < this.entries.length; }
}
