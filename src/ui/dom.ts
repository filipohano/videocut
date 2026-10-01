/** Tiny DOM helpers — the UI is plain TypeScript, no framework. */

type Child = Node | string | null | undefined | false;
type Attrs = Record<string, unknown>;

export function $<T extends HTMLElement = HTMLElement>(selector: string, root: ParentNode = document): T {
  const el = root.querySelector<T>(selector);
  if (!el) throw new Error(`Missing element: ${selector}`);
  return el;
}

export function h<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  attrs: Attrs = {},
  ...children: Child[]
): HTMLElementTagNameMap[K] {
  const el = document.createElement(tag);
  for (const [key, value] of Object.entries(attrs)) {
    if (value === undefined || value === null || value === false) continue;
    if (key === "class") el.className = String(value);
    else if (key === "style" && typeof value === "object") Object.assign(el.style, value);
    else if (key.startsWith("on") && typeof value === "function")
      el.addEventListener(key.slice(2).toLowerCase(), value as EventListener);
    else if (key in el && key !== "list") (el as unknown as Attrs)[key] = value;
    else el.setAttribute(key, value === true ? "" : String(value));
  }
  for (const child of children) {
    if (child === null || child === undefined || child === false) continue;
    el.append(child instanceof Node ? child : document.createTextNode(child));
  }
  return el;
}

export function show(el: HTMLElement, visible: boolean): void {
  el.classList.toggle("hidden", !visible);
}

/** Run `fn` at most once per animation frame. */
export function raf<A extends unknown[]>(fn: (...args: A) => void): (...args: A) => void {
  let queued: A | null = null;
  return (...args: A) => {
    const idle = queued === null;
    queued = args;
    if (idle)
      requestAnimationFrame(() => {
        const a = queued!;
        queued = null;
        fn(...a);
      });
  };
}

export interface Debounced<A extends unknown[]> {
  (...args: A): void;
  cancel(): void;
}

export function debounce<A extends unknown[]>(fn: (...args: A) => void, ms: number): Debounced<A> {
  let t: ReturnType<typeof setTimeout> | undefined;
  const wrapped = ((...args: A) => {
    clearTimeout(t);
    t = setTimeout(() => fn(...args), ms);
  }) as Debounced<A>;
  wrapped.cancel = () => clearTimeout(t);
  return wrapped;
}
