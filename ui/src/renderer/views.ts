import {
  projects,
  recipeById,
  recipesForProject,
  type EvaluationResult,
  type Project,
  type Recipe,
  type RunStage,
} from "./data.js";
import { h, type Child } from "./dom.js";

export interface ViewActions {
  openRecipe: (id: string) => void;
  openProject: (id: string) => void;
  copyText: (text: string) => void;
}

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
        h("span", { class: "run-dot " + recipe.outcome }),
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

/** Project overview: the latest governed decision is the control-plane focus. */
export function renderProjectPage(project: Project, actions: ViewActions): HTMLElement {
  const projectRecipes = recipesForProject(project.id);
  const latest = recipeById(project.latestRecipeId) ?? projectRecipes[0];
  if (!latest) {
    return pageShell({
      eyebrow: "Control plane",
      title: project.name,
      lede: project.description,
      children: [h("div", { class: "empty-state" }, "No optimization runs have been registered yet.")],
    });
  }
  return pageShell({
    eyebrow: "Control plane",
    title: project.name,
    lede: project.description,
    children: [
      h("section", { class: "outcome-panel " + latest.outcome, "aria-label": "Latest optimization decision" },
        h("div", { class: "outcome-signal", "aria-hidden": "true" }, outcomeIcon()),
        h("div", { class: "outcome-copy" },
          h("div", { class: "overline" }, "Latest decision"),
          h("h2", {}, latest.outcomeLabel),
          h("p", {}, latest.outcomeSummary),
          h("div", { class: "signal-row" },
            signal("Run complete", "neutral"),
            signal(latest.sealedState === "unused" ? "Sealed evidence unused" : "Sealed evidence consumed", "protected"),
          ),
        ),
        h("button", { class: "primary-button", type: "button", onClick: () => actions.openRecipe(latest.id) }, "Open run"),
      ),
      h("section", { class: "stage-section", "aria-label": "Latest run progression" },
        h("div", { class: "section-title-row" },
          h("div", {}, h("span", { class: "overline" }, "Latest run"), h("strong", { class: "mono" }, shortId(latest.runId))),
          h("span", { class: "section-caption" }, "Every transition is persisted"),
        ),
        renderStageRail(latest.stages),
      ),
      h("div", { class: "control-grid" },
        h("section", { class: "control-surface evaluation-surface" },
          sectionHeader("Development gates", "Baseline vs candidate"),
          renderEvaluationTable(latest.evaluations.filter((result) => result.role === "development")),
        ),
        h("div", { class: "control-stack" },
          h("section", { class: "control-surface next-action" },
            h("span", { class: "overline" }, "Next safe action"),
            h("p", {}, latest.nextAction),
          ),
          h("section", { class: "control-surface authority-panel" },
            sectionHeader("Authority & budgets", "Finite by contract"),
            ...latest.budgets.map((budget) => budgetRow(budget.label, budget.used, budget.limit, budget.unit)),
          ),
        ),
      ),
      h("section", { class: "control-surface run-history" },
        sectionHeader("Optimization history", projectRecipes.length + " immutable runs"),
        h("div", { class: "list-rows" }, ...projectRecipes.map((recipe) => recipeListRow(recipe, actions))),
      ),
      h("section", { class: "project-footprint" },
        h("div", { class: "footprint-heading" },
          h("span", { class: "overline" }, "Project authority"),
          h("span", { class: "integrity-mark" }, "Verified local workspace"),
        ),
        h("dl", { class: "facts" },
          fact("Task", project.task),
          fact("Baseline", project.baselineModel),
          fact("Revision", shortId(project.revision)),
          fact("Benchmark generation", shortId(project.benchmarkGeneration)),
        ),
      ),
    ],
  });
}

function outcomeIcon(): Node {
  return h("svg", { viewBox: "0 0 24 24" },
    h("path", { d: "M12 3 4.8 6v5.4c0 4.6 2.9 7.9 7.2 9.6 4.3-1.7 7.2-5 7.2-9.6V6L12 3Z" }),
    h("path", { d: "M8.5 12h7" }),
  );
}

function signal(label: string, tone: string): HTMLElement {
  return h("span", { class: "signal " + tone }, label);
}

function sectionHeader(title: string, meta: string): HTMLElement {
  return h("div", { class: "section-title-row" }, h("h2", {}, title), h("span", { class: "section-caption" }, meta));
}

function renderStageRail(stages: RunStage[]): HTMLElement {
  return h("ol", { class: "stage-rail" }, ...stages.map((stage, index) =>
    h("li", { class: "stage " + stage.state },
      h("div", { class: "stage-track" },
        h("span", { class: "stage-node", "aria-label": stage.state }, stage.state === "completed" ? "✓" : stage.state === "failed" ? "×" : stage.state === "protected" ? "◆" : String(index + 1)),
      ),
      h("strong", {}, stage.label),
      h("small", {}, stage.detail),
    ),
  ));
}

function renderEvaluationTable(results: EvaluationResult[]): HTMLElement {
  return h("div", { class: "table-scroll" },
    h("table", { class: "evaluation-table" },
      h("thead", {}, h("tr", {},
        h("th", { scope: "col" }, "Suite / metric"),
        h("th", { scope: "col" }, "Baseline"),
        h("th", { scope: "col" }, "Candidate"),
        h("th", { scope: "col" }, "Delta"),
        h("th", { scope: "col" }, "Gate"),
      )),
      h("tbody", {}, ...results.map((result) =>
        h("tr", { class: "evidence-role-" + result.role },
          h("td", {}, h("strong", {}, result.suite), h("small", {}, result.metric + " · " + result.role)),
          h("td", { class: "numeric" }, result.baseline),
          h("td", { class: "numeric" }, result.candidate),
          h("td", { class: "numeric delta " + result.gate }, result.delta),
          h("td", {}, h("span", { class: "gate " + result.gate }, result.gate === "passed" ? "Passed" : result.gate === "failed" ? "Failed" : "Not run")),
        ),
      )),
    ),
  );
}

function budgetRow(label: string, used: number, limit: number, unit: string): HTMLElement {
  const percentage = limit === 0 ? 0 : Math.min(100, (used / limit) * 100);
  return h("div", { class: "budget-row" },
    h("div", { class: "budget-copy" }, h("span", {}, label), h("strong", {}, used + " / " + limit + " ", h("small", {}, unit))),
    h("div", { class: "budget-track", role: "meter", "aria-valuemin": "0", "aria-valuemax": String(limit), "aria-valuenow": String(used) },
      h("span", { style: "width:" + percentage + "%" }),
    ),
  );
}

function shortId(value: string): string {
  return value.length > 18 ? value.slice(0, 8) + "…" + value.slice(-6) : value;
}

/** Recipe detail: one immutable optimization run and its decision evidence. */
export function renderRecipePage(recipe: Recipe, actions: ViewActions): HTMLElement {
  const project = projects.find((item) => item.id === recipe.projectId);
  return h("div", { class: "page-pad" },
    h("div", { class: "detail-nav" },
      h("button", { class: "quiet-button breadcrumb-back", type: "button", onClick: () => actions.openProject(recipe.projectId) }, "← Project overview"),
    ),
    h("header", { class: "run-heading" },
      h("div", { class: "overline" }, "Optimization run · immutable"),
      h("div", { class: "run-title-row" }, h("h1", { class: "page-heading" }, recipe.name), outcomePill(recipe)),
      h("p", { class: "page-lede" }, recipe.description),
      h("div", { class: "run-identity mono" }, h("span", {}, "RUN"), recipe.runId),
    ),
    h("section", { class: "run-decision " + recipe.outcome },
      h("div", { class: "decision-icon", "aria-hidden": "true" }, outcomeIcon()),
      h("div", { class: "decision-copy" },
        h("span", { class: "overline" }, "Deterministic decision"),
        h("h2", {}, recipe.outcomeLabel),
        h("p", {}, recipe.outcomeSummary),
      ),
      h("div", { class: "baseline-state" },
        h("span", { class: "overline" }, "Production baseline"),
        h("strong", {}, recipe.outcome === "promote-candidate" ? "Changed" : "Unchanged"),
      ),
    ),
    h("section", { class: "stage-section run-stage-section", "aria-label": "Run progression" },
      sectionHeader("Run progression", "Reserved and append-only"),
      renderStageRail(recipe.stages),
    ),
    h("div", { class: "run-grid" },
      h("section", { class: "control-surface evaluation-surface" },
        sectionHeader("Evaluation evidence", recipe.evaluations.length + " decisive metrics"),
        renderEvaluationTable(recipe.evaluations),
      ),
      h("div", { class: "control-stack" },
        h("section", { class: "control-surface next-action" },
          h("span", { class: "overline" }, "Next safe action"),
          h("p", {}, recipe.nextAction),
        ),
        h("section", { class: "control-surface authority-panel" },
          sectionHeader("Budget ledger", "Used / authorized"),
          ...recipe.budgets.map((budget) => budgetRow(budget.label, budget.used, budget.limit, budget.unit)),
        ),
      ),
    ),
    h("section", { class: "artifact-section" },
      h("div", { class: "artifact-section-heading" },
        h("div", {}, h("span", { class: "overline" }, "Immutable run graph"), h("h2", {}, "Inputs, policy and outputs")),
        h("span", { class: "section-caption" }, "Every artifact retains its source identity"),
      ),
      h("div", { class: "artifact-grid" },
        artifactPanel("Training authority", "Input", recipe.datasetSnapshots.map((snapshot) =>
          artifactItem(snapshot.name, snapshot.note, snapshot.id),
        )),
        artifactPanel("Training policy", "Config", recipe.instructions.map((instruction) =>
          artifactItem(instruction.label, instruction.value),
        )),
        artifactPanel("Candidate outputs", "Model", recipe.modelSnapshots.map((model) =>
          artifactItem(model.name, model.checksum, model.id),
        )),
      ),
    ),
    h("div", { class: "evidence-grid" },
      h("section", { class: "control-surface provenance-panel" },
        sectionHeader("Provenance", "Row-free evidence"),
        h("dl", { class: "provenance-list" }, ...recipe.provenance.map((entry) =>
          h("div", {}, h("dt", {}, entry.label), h("dd", { class: "mono" }, entry.value)),
        )),
      ),
      h("section", { class: "control-surface context-panel" },
        sectionHeader("Run context", "Bound at start"),
        h("dl", { class: "provenance-list" },
          h("div", {}, h("dt", {}, "Project"), h("dd", {}, project?.name ?? recipe.projectId)),
          h("div", {}, h("dt", {}, "Hypothesis"), h("dd", {}, recipe.hypothesis)),
          h("div", {}, h("dt", {}, "Sealed evidence"), h("dd", { class: "sealed-value " + recipe.sealedState }, recipe.sealedState)),
        ),
      ),
    ),
    h("section", { class: "control-surface commands-panel" },
      sectionHeader("Operator commands", "Read-only inspection and verification"),
      h("div", { class: "command-toolbar" },
        h("span", {}, "Use the CLI for authoritative operations."),
        h("button", { class: "quiet-button copy-button", type: "button", onClick: () => actions.copyText(recipe.commands) }, "Copy commands"),
      ),
      h("pre", { class: "code" }, recipe.commands),
    ),
  );
}

function artifactPanel(title: string, role: string, items: Child[]): HTMLElement {
  return h("section", { class: "artifact-panel" },
    h("div", { class: "artifact-panel-header" }, h("h3", {}, title), h("span", {}, role)),
    h("div", { class: "artifact-items" }, items.length ? items : h("div", { class: "tree-empty" }, "None.")),
  );
}

function artifactItem(title: string, detail: string, identity?: string): HTMLElement {
  return h("div", { class: "artifact-item" },
    h("strong", {}, title),
    h("span", {}, detail),
    identity ? h("code", {}, shortId(identity)) : null,
  );
}

function recipeListRow(recipe: Recipe, actions: ViewActions): HTMLElement {
  return h("button", { class: "list-row", type: "button", onClick: () => actions.openRecipe(recipe.id) },
    h("span", { class: "row-main" }, h("strong", {}, recipe.name), h("small", {}, recipe.description)),
    h("span", { class: "row-cell mono" }, shortId(recipe.runId)),
    h("span", { class: "row-cell sealed-label " + recipe.sealedState }, recipe.sealedState === "unused" ? "Sealed unused" : "Sealed used"),
    outcomePill(recipe),
  );
}

function outcomePill(recipe: Recipe): HTMLElement {
  return h("span", { class: "outcome-pill " + recipe.outcome }, recipe.outcomeLabel);
}

function fact(label: string, value: string): HTMLElement {
  return h("div", { class: "fact" }, h("dt", {}, label), h("dd", {}, value));
}
