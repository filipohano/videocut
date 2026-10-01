/** Routes `job-progress` events from the backend to whoever started the job. */
import { onProgress, type JobProgress } from "./api";

type Listener = (p: JobProgress) => void;
const listeners = new Map<string, Set<Listener>>();

export async function startProgressRouter(): Promise<void> {
  await onProgress((p) => listeners.get(p.job)?.forEach((fn) => fn(p)));
}

export function subscribe(job: JobProgress["job"], fn: Listener): () => void {
  if (!listeners.has(job)) listeners.set(job, new Set());
  listeners.get(job)!.add(fn);
  return () => listeners.get(job)?.delete(fn);
}

/** Bind a `.bar` + label to a job for the duration of a promise. */
export async function withProgress<T>(
  job: JobProgress["job"],
  bar: HTMLElement,
  label: HTMLElement,
  work: () => Promise<T>,
  defaultMessage = "",
): Promise<T> {
  const fill = bar.querySelector<HTMLElement>(".bar-fill")!;
  const render = (p: { fraction: number | null; message: string | null }) => {
    bar.classList.toggle("indeterminate", p.fraction === null);
    fill.style.width = p.fraction === null ? "" : `${Math.round(p.fraction * 100)}%`;
    label.textContent =
      p.fraction === null ? (p.message ?? defaultMessage) : `${p.message ?? defaultMessage} ${Math.round(p.fraction * 100)}%`.trim();
  };
  render({ fraction: null, message: defaultMessage });
  const off = subscribe(job, render);
  try {
    return await work();
  } finally {
    off();
  }
}
