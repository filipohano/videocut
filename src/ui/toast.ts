import { h } from "./dom";

interface ToastOptions {
  kind?: "info" | "success" | "error";
  action?: { label: string; onClick: () => void };
  /** ms; 0 keeps it until dismissed. */
  timeout?: number;
}

export function toast(message: string, opts: ToastOptions = {}): void {
  const host = document.getElementById("toasts")!;
  const kind = opts.kind ?? "info";
  const el = h(
    "div",
    { class: `toast ${kind}`, role: kind === "error" ? "alert" : "status" },
    h("span", { class: "toast-msg" }, message),
    opts.action &&
      h(
        "button",
        {
          class: "btn small",
          onclick: () => {
            opts.action!.onClick();
            el.remove();
          },
        },
        opts.action.label,
      ),
    h("button", { class: "icon-btn", "aria-label": "Dismiss", onclick: () => el.remove() }, "×"),
  );
  host.append(el);
  const timeout = opts.timeout ?? (kind === "error" ? 9000 : 5000);
  if (timeout > 0) setTimeout(() => el.remove(), timeout);
}
