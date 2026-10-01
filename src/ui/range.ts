/**
 * Range inputs that don't fight the user.
 *
 * The model (store) is the source of truth, but while someone is dragging a
 * slider we must not write the model's (possibly clamped / rounded) value back
 * into it — that is what made sliders jump. Safari/WebKit doesn't focus a range
 * input on mouse down, so `document.activeElement` can't be used to tell; we
 * track the pointer ourselves.
 */
export interface BoundRange {
  /** Reflect a model value into the slider (ignored while it is being dragged). */
  set(value: number): void;
  readonly dragging: boolean;
}

export function bindRange(
  input: HTMLInputElement,
  onInput: (value: number) => void,
  onSettle?: () => void,
): BoundRange {
  let dragging = false;
  const end = () => {
    if (!dragging) return;
    dragging = false;
    window.removeEventListener("pointerup", end);
    window.removeEventListener("pointercancel", end);
    onSettle?.();
  };
  input.addEventListener("pointerdown", () => {
    dragging = true;
    window.addEventListener("pointerup", end);
    window.addEventListener("pointercancel", end);
  });
  input.addEventListener("input", () => onInput(Number(input.value)));
  return {
    set(value: number) {
      if (!dragging && input.value !== String(value)) input.value = String(value);
    },
    get dragging() {
      return dragging;
    },
  };
}
