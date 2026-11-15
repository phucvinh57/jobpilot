// Toasts: short notes that something happened ("Saved.") or failed, shown at the bottom right
// and gone after a few seconds. Errors outside a form are reported this way; a form shows its own
// error under its fields (`FormMessage`).

import { create } from "zustand";

import type { CommandError } from "@/lib/problem";

export interface ErrorToastOptions {
  /** What failed ("Could not save"). */
  title?: string;
  /** Run the failed thing again (offered for a retryable problem). */
  onRetry?: () => void;
  /** Open the visible browser of a paused run (a `needsHuman` problem). */
  onOpenBrowser?: () => void;
  /** The toast went away (closed, timed out or acted on): forget the failure where it is kept. */
  onDismiss?: () => void;
}

export type Toast =
  | { id: number; kind: "ok" | "info"; title: string; body?: string }
  | ({ id: number; kind: "error"; title: string; error: CommandError } & Omit<ErrorToastOptions, "title">);

type NewToast = Toast extends infer T ? (T extends Toast ? Omit<T, "id"> : never) : never;

/** At most this many at once: the oldest goes first. */
export const MAX_TOASTS = 4;

interface ToastState {
  toasts: Toast[];
  push: (toast: NewToast) => number;
  dismiss: (id: number) => void;
  clear: () => void;
}

let next = 0;

/** What a toast says, to replace an identical one instead of stacking it. */
function sameAs(a: NewToast | Toast, b: NewToast | Toast): boolean {
  const text = (t: NewToast | Toast) => (t.kind === "error" ? t.error.message : (t.body ?? ""));
  return a.kind === b.kind && a.title === b.title && text(a) === text(b);
}

export const useToasts = create<ToastState>((set) => ({
  toasts: [],
  push: (toast) => {
    const id = ++next;
    set((state) => ({
      toasts: [...state.toasts.filter((t) => !sameAs(t, toast)), { ...toast, id } as Toast].slice(-MAX_TOASTS),
    }));
    return id;
  },
  dismiss: (id) => set((state) => ({ toasts: state.toasts.filter((t) => t.id !== id) })),
  clear: () => set({ toasts: [] }),
}));

export const toast = {
  /** Something the user asked for went through. */
  ok: (title: string, body?: string) => useToasts.getState().push({ kind: "ok", title, body }),
  /** A neutral note. */
  info: (title: string, body?: string) => useToasts.getState().push({ kind: "info", title, body }),
  /** A failed command, with its problem's advice and buttons. */
  error: (error: CommandError, options: ErrorToastOptions = {}) => {
    const { title = "Something went wrong", ...rest } = options;
    return useToasts.getState().push({ kind: "error", title, error, ...rest });
  },
};

/** How long a toast stays (the pointer on it pauses the clock); `null`: until it is closed. A
 *  failure that offers Retry stays, so it is not missed while the user looks elsewhere. */
export function toastDuration(item: Toast): number | null {
  if (item.kind !== "error") {
    return 4_000;
  }
  return item.onRetry ? null : 10_000;
}
