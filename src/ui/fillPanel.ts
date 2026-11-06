/** The "Remove text" section of the editor: locked until the AI add-on is downloaded. */
import { formatBytes } from "../lib/format";
import { MODULE_ID, currentTarget, fillUi, fillsFor, moduleInstalled, removeFill, setDrawing, setLetters, setShowOriginal, slideAt } from "../fillOps";
import { store } from "../store";
import { $, h } from "./dom";
import type { Stage } from "./stage";

export function initFillPanel(stage: Stage): void {
  const body = $("#fill-body");

  const openSettings = () => $("#btn-settings").click();

  function locked(): HTMLElement[] {
    const m = store.modules.find((x) => x.id === MODULE_ID);
    return [
      h(
        "p",
        { class: "muted small fill-note" },
        `Remove captions and other text from photos and photo videos with AI, and fill in what was behind them. It's a free add-on${m ? ` (${formatBytes(m.bytes)})` : ""} that runs on your computer.`,
      ),
      h("div", {}, h("button", { class: "btn small", onclick: openSettings }, "Download it in Settings")),
    ];
  }

  /** "Photo 2 of 4" with buttons to go to the previous / next photo. */
  function slideBar(): HTMLElement | null {
    const s = store.video?.stills;
    if (s?.kind !== "slides") return null;
    const slide = slideAt(store.playhead);
    const go = (i: number) => {
      const seg = s.segments[Math.max(0, Math.min(s.segments.length - 1, i))];
      stage.pause();
      // A moment into the photo, so the frame shown is surely that photo.
      stage.seek(seg.start + Math.min(0.1, (seg.end - seg.start) / 2));
    };
    const index = slide?.index ?? s.segments.findIndex((seg) => seg.start > store.playhead) - 1;
    return h(
      "div",
      { class: "slide-bar" },
      h("button", { class: "btn small", title: "Previous photo", "aria-label": "Previous photo", disabled: !!slide && slide.index === 0, onclick: () => go((slide?.index ?? index + 1) - 1) }, "‹"),
      h("span", { class: "slide-name" }, slide ? `Photo ${slide.index + 1} of ${s.segments.length}` : "Between two photos"),
      h("button", { class: "btn small", title: "Next photo", "aria-label": "Next photo", disabled: !!slide && slide.index === s.segments.length - 1, onclick: () => go((slide?.index ?? index) + 1) }, "›"),
      h("span", { class: "muted small grow" }, "Text is removed from the photo shown."),
    );
  }

  function editor(): HTMLElement[] {
    const target = currentTarget();
    const fills = target ? fillsFor(target) : [];
    const draw = h(
      "button",
      {
        class: `btn ${fillUi.drawing ? "primary" : ""}`,
        "aria-pressed": String(fillUi.drawing),
        disabled: !target || !!fillUi.working,
        onclick: () => setDrawing(!fillUi.drawing),
      },
      fillUi.drawing ? "Drawing… (Esc to stop)" : "Draw a box over the text",
    );
    const letters = h("input", { type: "checkbox", checked: fillUi.letters, onchange: (e: Event) => setLetters((e.target as HTMLInputElement).checked) });
    const original = h("input", { type: "checkbox", checked: fillUi.showOriginal, onchange: (e: Event) => setShowOriginal((e.target as HTMLInputElement).checked) });
    return [
      slideBar(),
      h("div", { class: "text-row" }, draw, fillUi.working ? h("span", { class: "small muted" }, "Removing text…") : null),
      h(
        "label",
        { class: "check" },
        letters,
        h("span", {}, "Only the letters", h("br"), h("span", { class: "muted small" }, "Removes white or black text in the box and leaves the rest. Turn off to fill the whole box.")),
      ),
      fills.length > 0
        ? h(
            "div",
            { class: "fill-list" },
            ...fills.map((f, i) =>
              h(
                "div",
                { class: "fill-row" },
                h("span", { class: "grow" }, `Removed area ${i + 1}`),
                h("span", { class: "muted small" }, `${f.w}×${f.h} px`),
                h("button", { class: "icon-btn", title: "Put the text back", "aria-label": `Put area ${i + 1} back`, onclick: () => removeFill(f.id) }, "×"),
              ),
            ),
          )
        : null,
      fills.length > 0 ? h("label", { class: "check" }, original, h("span", {}, "Show the original")) : null,
      h(
        "p",
        { class: "muted small fill-note" },
        target
          ? "Drag a box over the text in the preview. The AI fills in what was behind it; Undo (⌘Z) takes it back."
          : "Go to one of the photos to remove text from it.",
      ),
    ].filter((x): x is HTMLElement => x !== null);
  }

  function render(): void {
    const v = store.video;
    if (!v) return;
    if (!moduleInstalled()) return body.replaceChildren(...locked());
    const s = v.stills;
    if (!s) return body.replaceChildren(h("p", { class: "muted small fill-note" }, "Checking whether the picture stands still…"));
    if (s.kind === "moving") {
      return body.replaceChildren(
        h(
          "p",
          { class: "muted small fill-note" },
          "This video moves, so text can't be removed from it. It works on photos, and on videos where the picture stands still: photo posts with music and slideshows.",
        ),
      );
    }
    body.replaceChildren(...editor());
  }

  // The slide shown only matters for slideshows; redrawing on every frame would be wasteful.
  let lastSlide: number | null | undefined;
  store.on("playhead", () => {
    if (store.video?.stills?.kind !== "slides") return;
    const now = slideAt(store.playhead)?.index ?? null;
    if (now === lastSlide) return;
    lastSlide = now;
    render();
  });
  store.on(["video", "modules", "stills", "fills", "busy"], render);
}
