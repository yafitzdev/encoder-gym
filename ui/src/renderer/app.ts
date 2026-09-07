import { activeProjectId, projectById, recipeById } from "./data.js";
import { NavigationHistory, type AppLocation, type ViewName } from "./state.js";
import { renderProjectPage, renderProjectTree, renderRecipePage, type ViewActions } from "./views.js";

const SIDEBAR_WIDTH_KEY = "encoder-gym:sidebar-width";
const SIDEBAR_COLLAPSED_KEY = "encoder-gym:sidebar-collapsed";

const viewNames: ViewName[] = ["project", "recipe"];

interface ShellElements {
  appShell: HTMLElement;
  projectsNav: HTMLElement;
  pageTitle: HTMLElement;
  pageSubtitle: HTMLElement;
  sections: Record<ViewName, HTMLElement>;
  backButton: HTMLButtonElement;
  forwardButton: HTMLButtonElement;
  sidebarMenu: HTMLButtonElement;
  resizer: HTMLElement;
  workspaceState: HTMLElement;
}

export function mount(): void {
  const elements = collectElements();
  const actions: ViewActions = {
    openRecipe: (id) => navigate({ view: "recipe", recipeId: id, projectId: recipeById(id)?.projectId ?? activeProjectId }),
    openProject: (id) => navigate({ view: "project", projectId: id }),
    copyText: (text) => { void window.encoderGym.copyText(text); },
  };
  const history = new NavigationHistory({ replay: (location) => render(location) });

  function navigate(location: AppLocation): void {
    history.remember(location);
    render(location);
  }

  function render(location: AppLocation): void {
    const { projectId = activeProjectId, recipeId } = location;
    const project = projectById(projectId);
    const recipe = recipeId ? recipeById(recipeId) : undefined;

    elements.projectsNav.replaceChildren(renderProjectTree(projectId, recipeId, actions));

    if (location.view === "recipe") {
      elements.pageTitle.textContent = project?.name ?? "Project";
      elements.pageSubtitle.textContent = recipe ? "Runs / " + recipe.name : "Run";
      elements.pageSubtitle.hidden = false;
    } else {
      elements.pageTitle.textContent = project?.name ?? "Project";
      elements.pageSubtitle.textContent = "";
      elements.pageSubtitle.hidden = true;
    }

    for (const view of viewNames) {
      const section = elements.sections[view];
      const active = location.view === view;
      section.hidden = !active;
      if (active) {
        section.replaceChildren(
          view === "project"
            ? (project ? renderProjectPage(project, actions) : emptyText("Project " + projectId + " is not registered yet."))
            : (recipe ? renderRecipePage(recipe, actions) : emptyText("Recipe not found.")),
        );
      }
    }

    elements.backButton.disabled = !history.canNavigate(-1);
    elements.forwardButton.disabled = !history.canNavigate(1);
  }

  function emptyText(text: string): HTMLElement {
    const el = document.createElement("div");
    el.className = "page-pad";
    el.textContent = text;
    return el;
  }

  /* ------------------------- wiring ------------------------- */

  elements.backButton.addEventListener("click", () => { void history.navigate(-1); });
  elements.forwardButton.addEventListener("click", () => { void history.navigate(1); });

  const collapsedInitial = readStoredBoolean(SIDEBAR_COLLAPSED_KEY, false);
  setSidebarCollapsed(elements.appShell, collapsedInitial);
  elements.sidebarMenu.addEventListener("click", () => {
    const collapsed = elements.appShell.classList.contains("sidebar-collapsed");
    setSidebarCollapsed(elements.appShell, !collapsed);
  });
  wireResizer(elements.resizer, elements.appShell);

  for (const button of Array.from(document.querySelectorAll<HTMLButtonElement>("[data-window-action]"))) {
    button.addEventListener("click", () => {
      const action = button.dataset.windowAction;
      if (action === "minimize" || action === "maximize" || action === "close") void window.encoderGym.windowAction(action);
    });
  }

  elements.workspaceState.classList.add("online");
  elements.workspaceState.textContent = "local · evidence preview";

  navigate({ view: "project", projectId: activeProjectId });
}

function setSidebarCollapsed(appShell: HTMLElement, collapsed: boolean): void {
  appShell.classList.toggle("sidebar-collapsed", collapsed);
  try { window.localStorage.setItem(SIDEBAR_COLLAPSED_KEY, collapsed ? "1" : "0"); } catch { /* ignore */ }
}

function readStoredBoolean(key: string, fallback: boolean): boolean {
  try {
    const value = window.localStorage.getItem(key);
    if (value === null) return fallback;
    return value === "1";
  } catch {
    return fallback;
  }
}

function readStoredSidebarWidth(): number | undefined {
  try {
    const value = window.localStorage.getItem(SIDEBAR_WIDTH_KEY);
    if (value === null) return undefined;
    const parsed = Number(value);
    return Number.isFinite(parsed) && parsed >= 180 && parsed <= 420 ? parsed : undefined;
  } catch {
    return undefined;
  }
}

function wireResizer(resizer: HTMLElement, appShell: HTMLElement): void {
  const stored = readStoredSidebarWidth();
  if (stored) appShell.style.setProperty("--sidebar-width", stored + "px");

  let startX = 0;
  let startWidth = 272;
  let dragging = false;

  resizer.addEventListener("mousedown", (event) => {
    dragging = true;
    startX = (event as MouseEvent).clientX;
    const configuredWidth = Number.parseFloat(window.getComputedStyle(appShell).getPropertyValue("--sidebar-width"));
    startWidth = Number.isFinite(configuredWidth) ? configuredWidth : 272;
    resizer.classList.add("dragging");
    document.body.style.cursor = "col-resize";
    document.body.style.userSelect = "none";
  });
  window.addEventListener("mousemove", (event) => {
    if (!dragging) return;
    const width = Math.min(420, Math.max(180, startWidth + (event.clientX - startX)));
    appShell.style.setProperty("--sidebar-width", width + "px");
    try { window.localStorage.setItem(SIDEBAR_WIDTH_KEY, String(width)); } catch { /* ignore */ }
  });
  window.addEventListener("mouseup", () => {
    if (!dragging) return;
    dragging = false;
    resizer.classList.remove("dragging");
    document.body.style.cursor = "";
    document.body.style.userSelect = "";
  });
}

function collectElements(): ShellElements {
  const required = (id: string): HTMLElement => {
    const el = document.getElementById(id);
    if (!el) throw new Error("Renderer is missing #" + id);
    return el;
  };
  return {
    appShell: required("app-shell"),
    projectsNav: required("projects"),
    pageTitle: required("page-title"),
    pageSubtitle: required("page-subtitle"),
    sections: {
      project: required("page-project"),
      recipe: required("page-recipe"),
    },
    backButton: required("navigate-back") as HTMLButtonElement,
    forwardButton: required("navigate-forward") as HTMLButtonElement,
    sidebarMenu: required("sidebar-menu") as HTMLButtonElement,
    resizer: required("sidebar-resizer"),
    workspaceState: required("workspace-state"),
  };
}

if (typeof window !== "undefined") {
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", mount);
  else mount();
}
