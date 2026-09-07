export type ViewName = "project" | "recipe";

export interface AppLocation {
  view: ViewName;
  projectId?: string;
  recipeId?: string;
}

export interface NavigationOptions {
  replay: (location: AppLocation) => void | Promise<void>;
}

/** Owns workspace navigation history (back/forward), deduped like the desktop shell. */
export class NavigationHistory {
  readonly #options: NavigationOptions;
  readonly #entries: AppLocation[] = [];
  #index = -1;
  #replaying = false;
  #navigating = false;

  constructor(options: NavigationOptions) {
    this.#options = options;
    this.remember({ view: "project" });
  }

  remember(location: AppLocation): void {
    if (this.#replaying) return;
    const previous = this.#entries[this.#index];
    if (previous && sameLocation(previous, location)) return;
    this.#entries.splice(this.#index + 1);
    this.#entries.push(location);
    this.#index = this.#entries.length - 1;
  }

  async navigate(offset: -1 | 1): Promise<void> {
    if (this.#navigating) return;
    const nextIndex = this.#index + offset;
    const location = this.#entries[nextIndex];
    if (!location) return;
    this.#navigating = true;
    this.#index = nextIndex;
    this.#replaying = true;
    try {
      await this.#options.replay(location);
    } finally {
      this.#replaying = false;
      this.#navigating = false;
    }
  }

  canNavigate(offset: -1 | 1): boolean {
    const next = this.#index + offset;
    return next >= 0 && next < this.#entries.length;
  }
}

function sameLocation(left: AppLocation, right: AppLocation): boolean {
  return left.view === right.view && left.projectId === right.projectId && left.recipeId === right.recipeId;
}
