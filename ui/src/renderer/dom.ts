export type Child = string | number | boolean | Node | null | undefined | Child[];

const SVG_TAGS = new Set(["svg", "path", "g", "rect", "circle", "line", "polygon", "polyline", "defs", "clipPath", "use", "text", "ellipse"]);

export function h(tag: string, attrs: Record<string, unknown> = {}, ...children: Child[]): HTMLElement {
  const node = SVG_TAGS.has(tag)
    ? document.createElementNS("http://www.w3.org/2000/svg", tag)
    : document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === null || value === undefined || value === false) continue;
    if (key === "class") node.setAttribute("class", String(value));
    else if (key === "dataset") {
      const record = value as Record<string, string>;
      for (const [name, entry] of Object.entries(record)) node.setAttribute("data-" + name, entry);
    } else if (key.startsWith("on") && typeof value === "function") {
      node.addEventListener(key.slice(2).toLowerCase(), value as EventListener);
    } else if (key === "value") {
      if (node instanceof HTMLInputElement || node instanceof HTMLTextAreaElement || node instanceof HTMLSelectElement) node.value = String(value);
      else node.setAttribute(key, String(value));
    } else if (key === "checked" && node instanceof HTMLInputElement) node.checked = Boolean(value);
    else if (value === true) node.setAttribute(key, "");
    else node.setAttribute(key, String(value));
  }
  appendChildren(node, children);
  // Select values must be applied after their option children exist.
  if (node instanceof HTMLSelectElement && attrs.value !== undefined) node.value = String(attrs.value);
  return node as HTMLElement;
}

function appendChildren(node: { append(...nodes: Array<Node | string>): void }, children: Child[]): void {
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    if (Array.isArray(child)) {
      appendChildren(node, child);
      continue;
    }
    node.append(child instanceof Node ? child : document.createTextNode(String(child)));
  }
}

/**
 * Move keyed live nodes from the current tree into its replacement.
 *
 * The renderer intentionally rebuilds ordinary presentation state. Animated
 * status nodes are different: replacing them restarts their CSS animation and
 * creates visible jitter during frequent progress updates.
 */
export function preserveKeyedNodes(current: ParentNode, replacement: ParentNode): void {
  const existing = new Map<string, HTMLElement>();
  for (const node of current.querySelectorAll<HTMLElement>("[data-preserve-key]")) {
    const key = node.dataset.preserveKey;
    if (key) existing.set(key, node);
  }
  for (const next of replacement.querySelectorAll<HTMLElement>("[data-preserve-key]")) {
    const key = next.dataset.preserveKey;
    const previous = key ? existing.get(key) : undefined;
    if (previous && previous.tagName === next.tagName && previous.className === next.className) {
      next.replaceWith(previous);
    }
  }
}
