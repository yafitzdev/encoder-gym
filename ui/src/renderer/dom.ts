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
