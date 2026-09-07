import { projects, recipesForProject, type Project, type Recipe, type RecipeStatus } from "./data.js";
import { h, type Child } from "./dom.js";

export interface ViewActions {
  openRecipe: (id: string) => void;
  openProject: (id: string) => void;
  copyText: (text: string) => void;
}

const statusLabel: Record<RecipeStatus, string> = {
  queued: "Queued",
  running: "Running",
  completed: "Completed",
  failed: "Failed",
};

/* ------------------------------ sidebar ------------------------------ */

export function renderProjectTree(
  activeProjectId: string | undefined,
  activeRecipeId: string | undefined,
  actions: ViewActions,
): HTMLElement {
  const rows = projects.map((project) => {
    const projectRecipes = recipesForProject(project.id);
    const recipeRows = projectRecipes.map((recipe) =>
      h("button",
        { class: "run-row" + (activeRecipeId === recipe.id ? " active" : ""), type: "button", onClick: () => actions.openRecipe(recipe.id) },
        h("span", { class: "run-dot " + recipe.status }),
        h("span", {}, recipe.name),
      ),
    );
    return h("div", { class: "project-tree-item" },
      h("button",
        { class: "project-row" + (activeProjectId === project.id ? " active" : ""), type: "button", onClick: () => actions.openProject(project.id) },
        folderIcon(),
        h("span", {}, project.name),
      ),
      ...recipeRows,
    );
  });
  if (rows.length === 0) return h("div", { class: "tree-empty" }, "No projects yet.");
  return h("div", { class: "project-tree" }, ...rows);
}

function folderIcon(): Node {
  return h("svg", { viewBox: "0 0 20 20", "aria-hidden": "true" },
    h("path", { d: "M3.5 5.5h5l1.7 2h6.3a1 1 0 0 1 1 1v6a1 1 0 0 1-1 1h-13a1 1 0 0 1-1-1v-8a1 1 0 0 1 1-1Z", fill: "none", stroke: "currentColor", "stroke-width": "1.4" }),
  );
}

/* ------------------------------ pages ------------------------------ */

function pageShell({ eyebrow, title, lede, children }: { eyebrow: string; title: string; lede?: string; children: Child[] }): HTMLElement {
  return h("div", { class: "page-pad" },
    eyebrow ? h("div", { class: "eyebrow" }, eyebrow) : null,
    h("h1", { class: "page-heading" }, title),
    lede ? h("p", { class: "page-lede" }, lede) : null,
    ...children,
  );
}

/** Project overview: evaluation results are shared and deferred for now. */
export function renderProjectPage(project: Project, actions: ViewActions): HTMLElement {
  const projectRecipes = recipesForProject(project.id);
  return pageShell({
    eyebrow: "Project",
    title: project.name,
    lede: project.description,
    children: [
      h("div", { class: "card" },
        h("div", { class: "card-body" },
          h("dl", { class: "facts" },
            fact("Task", project.task),
            fact("Folder", project.folder),
            fact("Evaluation suite", project.evaluationSuite.name),
          ),
        ),
      ),
      h("div", { class: "card section-gap" },
        h("div", { class: "card-header" }, h("h2", {}, "Recipes")),
        h("div", { class: "list-rows" }, projectRecipes.length
          ? projectRecipes.map((recipe) => recipeListRow(recipe, actions))
          : h("div", { class: "tree-empty" }, "No recipes yet.")),
      ),
      h("div", { class: "card section-gap" },
        h("div", { class: "card-header" }, h("h2", {}, "Evaluation results")),
        h("div", { class: "card-body muted" }, "Recipe evaluation results will be compared here. Deferred for now."),
      ),
    ],
  });
}

/** Recipe detail: A) dataset snapshots, B) instructions/configs, C) model snapshots. */
export function renderRecipePage(recipe: Recipe, actions: ViewActions): HTMLElement {
  return h("div", { class: "page-pad" },
    h("div", { class: "detail-nav" },
      h("button", { class: "quiet-button", type: "button", onClick: () => actions.openProject(recipe.projectId) }, "← Back to project"),
    ),
    h("div", { class: "badge-row" }, statusPill(recipe.status)),
    h("h1", { class: "page-heading" }, recipe.name),
    h("p", { class: "page-lede" }, recipe.description),
    h("div", { class: "bucket-grid" },
      bucket("A", "Dataset snapshots", recipe.datasetSnapshots.map((snap) =>
        h("div", { class: "bucket-item" }, h("strong", {}, snap.name), h("small", {}, snap.note)),
      )),
      bucket("B", "Instructions / configs", recipe.instructions.map((ins) =>
        h("div", { class: "bucket-item" }, h("strong", {}, ins.label), h("small", {}, ins.value)),
      )),
      bucket("C", "Model snapshots", recipe.modelSnapshots.map((ms) =>
        h("div", { class: "bucket-item" }, h("strong", {}, ms.name), h("small", {}, ms.checksum)),
      )),
    ),
    h("div", { class: "card section-gap" },
      h("div", { class: "card-header" },
        h("h2", {}, "Commands"),
        h("button", { class: "quiet-button", type: "button", onClick: () => actions.copyText(recipe.commands) }, "Copy"),
      ),
      h("div", { class: "card-body" }, h("pre", { class: "code" }, recipe.commands)),
    ),
  );
}

function bucket(letter: string, title: string, items: Child[]): HTMLElement {
  return h("section", { class: "card bucket" },
    h("div", { class: "card-header" },
      h("h2", {}, title),
      h("span", { class: "bucket-letter" }, letter),
    ),
    h("div", { class: "card-body" }, items.length ? items : h("div", { class: "tree-empty" }, "None.")),
  );
}

function recipeListRow(recipe: Recipe, actions: ViewActions): HTMLElement {
  return h("button", { class: "list-row", type: "button", onClick: () => actions.openRecipe(recipe.id) },
    h("span", { class: "row-main" }, h("strong", {}, recipe.name), h("small", {}, recipe.description)),
    h("span", { class: "row-cell" }, String(recipe.datasetSnapshots.length) + " snapshots"),
    h("span", { class: "row-cell" }, String(recipe.modelSnapshots.length) + " models"),
    statusPill(recipe.status),
  );
}

function statusPill(status: RecipeStatus): HTMLElement {
  return h("span", { class: "status-pill " + status }, statusLabel[status]);
}

function fact(label: string, value: string): HTMLElement {
  return h("div", { class: "fact" }, h("dt", {}, label), h("dd", {}, value));
}
