export type Child = string | number | boolean | Node | null | undefined | Child[];

const SVG_TAGS = new Set(["svg", "path", "g", "rect", "circle", "line", "polygon", "polyline", "defs", "clipPath", "use", "text", "ellipse"]);
const handlers = new WeakMap<Element, Map<string, EventListener>>();

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
      const name = key.slice(2).toLowerCase();
      node.addEventListener(name, value as EventListener);
      if (!handlers.has(node)) handlers.set(node, new Map());
      handlers.get(node)!.set(name, value as EventListener);
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

/** Reconnecting even the SAME element restarts CSS animations in Chromium.
 * Anchor each spinner to the document clock after insertion, before paint.
 * Its phase then stays continuous through file updates and page navigation.
 */
export function replaceView(current: HTMLElement, replacement: Node): void {
  const next = replacement instanceof DocumentFragment ? replacement.firstElementChild : replacement;
  const previous = current.firstElementChild;
  // Live overview updates must not disconnect pressed buttons or a scrollbar
  // being dragged. Other pages retain their existing form/dialog lifecycle.
  if (previous?.hasAttribute("data-live-view") && next instanceof Element
    && previous.getAttribute("data-live-view") === next.getAttribute("data-live-view")) {
    reconcile(previous, next);
  } else {
    preserveKeyedNodes(current, replacement as ParentNode);
    current.replaceChildren(replacement);
  }
  for (const spinner of current.querySelectorAll<HTMLElement>(".spinner")) {
    for (const animation of spinner.getAnimations()) animation.startTime = 0;
  }
}

function nodeKey(node: Node): string | undefined {
  if (!(node instanceof Element)) return undefined;
  return node.id || node.getAttribute("data-key") || node.getAttribute("data-run-id") || node.getAttribute("data-preserve-key") || undefined;
}

function compatible(previous: Node, next: Node): boolean {
  if (previous.nodeType !== next.nodeType || previous.nodeName !== next.nodeName || nodeKey(previous) !== nodeKey(next)) return false;
  if (nodeKey(previous)) return true;
  return !(previous instanceof Element && next instanceof Element)
    || previous.getAttribute("class")?.split(" ")[0] === next.getAttribute("class")?.split(" ")[0];
}

/** Patch the live overview in place, including fresh event closures. */
function reconcile(previous: Element, next: Element): void {
  const selectedValue = next instanceof HTMLSelectElement ? next.value : undefined;
  for (const attr of Array.from(previous.attributes)) if (!next.hasAttribute(attr.name)) previous.removeAttribute(attr.name);
  for (const attr of Array.from(next.attributes)) if (previous.getAttribute(attr.name) !== attr.value) previous.setAttribute(attr.name, attr.value);
  for (const [name, listener] of handlers.get(previous) ?? []) previous.removeEventListener(name, listener);
  const listeners = handlers.get(next) ?? new Map<string, EventListener>();
  for (const [name, listener] of listeners) previous.addEventListener(name, listener);
  handlers.set(previous, listeners);

  const oldChildren = Array.from(previous.childNodes), used = new Set<Node>();
  const keyed = new Map(oldChildren.filter(node => nodeKey(node)).map(node => [nodeKey(node)!, node]));
  let cursor = previous.firstChild;
  for (const child of Array.from(next.childNodes)) {
    const key = nodeKey(child);
    const match = key ? keyed.get(key) : oldChildren.find(node => !used.has(node) && !nodeKey(node) && compatible(node, child));
    const live = match && compatible(match, child) ? match : child;
    used.add(live);
    if (live !== cursor) previous.insertBefore(live, cursor);
    if (live !== child) {
      if (live instanceof Element && child instanceof Element) reconcile(live, child);
      else if (live.nodeValue !== child.nodeValue) live.nodeValue = child.nodeValue;
    }
    cursor = live.nextSibling;
  }
  for (const child of oldChildren) if (!used.has(child)) child.remove();
  if (previous instanceof HTMLSelectElement && selectedValue !== undefined && previous.value !== selectedValue) previous.value = selectedValue;
  if (previous instanceof HTMLInputElement && next instanceof HTMLInputElement) {
    if (previous.value !== next.value) previous.value = next.value;
    previous.checked = next.checked;
  }
}
