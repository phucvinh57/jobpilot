// The one place that talks to the Tauri shell. Only the modules in `src/api/` import it (Biome
// enforces it, `biome.jsonc`): the rest of the app calls their typed functions and subscribes to
// events through `./events`, so everything above runs without a shell (tests, mocks).

import { invoke as tauriInvoke } from "@tauri-apps/api/core";
import { listen as tauriListen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open as openDialog, save as saveDialog } from "@tauri-apps/plugin-dialog";

import { toCommandError } from "@/lib/problem";

/** Call the shell command `command` (`<context>_<name>`). Rejects with a `CommandError`. */
export async function invoke<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await tauriInvoke<T>(command, args);
  } catch (thrown) {
    throw toCommandError(thrown);
  }
}

/** Subscribe to the shell event `event`. Returns the unsubscribe (safe to call at any time). */
export function listen<T>(event: string, handler: (payload: T) => void): () => void {
  let stopped = false;
  let unlisten: (() => void) | undefined;
  tauriListen<T>(event, (e) => {
    if (!stopped) {
      handler(e.payload);
    }
  })
    .then((stop) => {
      if (stopped) {
        stop();
      } else {
        unlisten = stop;
      }
    })
    .catch((e: unknown) => console.warn(`listening to ${event} failed`, e));
  return () => {
    stopped = true;
    unlisten?.();
  };
}

export interface FileFilter {
  name: string;
  extensions: string[];
}

/** The native file picker: the chosen path, or `null` when the user cancelled. */
export async function pickFile(title: string, filters: FileFilter[] = []): Promise<string | null> {
  const picked = await openDialog({ title, filters, multiple: false, directory: false });
  return typeof picked === "string" ? picked : null;
}

/** The native save dialog: the chosen path, or `null` when the user cancelled. */
export async function pickSavePath(
  title: string,
  defaultPath: string,
  filters: FileFilter[] = [],
): Promise<string | null> {
  const picked = await saveDialog({ title, defaultPath, filters });
  return typeof picked === "string" ? picked : null;
}

export type FileDrop = { type: "over" } | { type: "leave" } | { type: "drop"; paths: string[] };

/** Files dragged over or dropped on the window. Returns the unsubscribe. */
export function onFileDrop(handler: (drop: FileDrop) => void): () => void {
  let stopped = false;
  let unlisten: (() => void) | undefined;
  getCurrentWebview()
    .onDragDropEvent((event) => {
      if (stopped) {
        return;
      }
      const payload = event.payload;
      if (payload.type === "drop") {
        handler({ type: "drop", paths: payload.paths });
      } else if (payload.type === "leave") {
        handler({ type: "leave" });
      } else {
        handler({ type: "over" });
      }
    })
    .then((stop) => {
      if (stopped) {
        stop();
      } else {
        unlisten = stop;
      }
    })
    .catch((e: unknown) => console.warn("listening to file drops failed", e));
  return () => {
    stopped = true;
    unlisten?.();
  };
}

// ------------------------------------------------------------------ shared events

/** Which stored data changed (see `shared/events.rs`). */
export type Scope = "applications" | "leads" | "sources" | "trends" | "profile" | "integrations" | "health";

/** One step of a graph run (discovery, trends, prepare-CV, preference chat). Labels only. */
export interface GraphProgress {
  thread: string;
  graph: string;
  kind: "nodeStarted" | "nodeFinished" | "nodeFailed" | "detail" | "waiting" | "done" | "failed" | "cancelled";
  node: string | null;
  text: string | null;
}

type ScopeHandler = (scope: Scope) => void;
const scopeHandlers = new Set<ScopeHandler>();
let shellSubscribed = false;

function subscribeShell() {
  if (shellSubscribed) {
    return;
  }
  shellSubscribed = true;
  listen<{ scope: Scope }>("shared:data-changed", ({ scope }) => {
    for (const handler of [...scopeHandlers]) {
      handler(scope);
    }
  });
}

/** Stored data changed, in the shell (mail sync, a background run) or in this window
 *  ([`notifyChanged`]). Returns the unsubscribe. */
export function onDataChanged(handler: ScopeHandler): () => void {
  subscribeShell();
  scopeHandlers.add(handler);
  return () => scopeHandlers.delete(handler);
}

/** A command of this window changed `scopes`: tell every screen that shows them. */
export function notifyChanged(...scopes: Scope[]): void {
  for (const scope of scopes) {
    for (const handler of [...scopeHandlers]) {
      handler(scope);
    }
  }
}

/** Graph progress of every run. Returns the unsubscribe. */
export function onGraphProgress(handler: (progress: GraphProgress) => void): () => void {
  return listen<GraphProgress>("shared:graph-progress", handler);
}
