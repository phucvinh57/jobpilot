// The tabs the extension opened, and the JobPilot windows: the only things it ever acts on. Kept in
// `chrome.storage.session` so a service-worker restart does not forget them (cleared with the browser).

export type WindowKind = "normal" | "private";

type State = {
  owned: number[];
  guarded: number[];
  /** Owned tabs that are gone (recent ones): closing one again is not an error. */
  closed: number[];
  windows: Partial<Record<WindowKind, number>>;
};

const KEYS = ["owned", "guarded", "closed", "windows"] as const;
const CLOSED_KEPT = 200;

let state: State | null = null;
let queue: Promise<unknown> = Promise.resolve();

async function load(): Promise<State> {
  if (state) return state;
  const stored = await chrome.storage.session.get([...KEYS]);
  const numbers = (value: unknown) => (Array.isArray(value) ? value.filter((n) => typeof n === "number") : []);
  const windows = typeof stored.windows === "object" && stored.windows !== null ? stored.windows : {};
  state = {
    owned: numbers(stored.owned),
    guarded: numbers(stored.guarded),
    closed: numbers(stored.closed),
    windows: windows as State["windows"],
  };
  return state;
}

/** Run one change after the previous ones and persist it. */
function change<T>(apply: (s: State) => T): Promise<T> {
  const next = queue.then(async () => {
    const s = await load();
    const result = apply(s);
    await chrome.storage.session.set({ ...s });
    return result;
  });
  queue = next.catch(() => undefined);
  return next;
}

export const isOwned = async (tab: number): Promise<boolean> => (await load()).owned.includes(tab);

export const ownedTabs = async (): Promise<number[]> => [...(await load()).owned];

export const wasClosed = async (tab: number): Promise<boolean> => (await load()).closed.includes(tab);

export const guardedTabs = async (): Promise<number[]> => [...(await load()).guarded];

export const isGuarded = async (tab: number): Promise<boolean> => (await load()).guarded.includes(tab);

export const addOwned = (tab: number): Promise<void> =>
  change((s) => {
    if (!s.owned.includes(tab)) s.owned.push(tab);
    s.closed = s.closed.filter((t) => t !== tab);
  });

export const markGuarded = (tab: number): Promise<void> =>
  change((s) => {
    if (!s.guarded.includes(tab)) s.guarded.push(tab);
  });

/** Forget an owned tab that is gone. Returns whether it was owned (and whether it was guarded). */
export const removeOwned = (tab: number): Promise<{ owned: boolean; guarded: boolean }> =>
  change((s) => {
    const owned = s.owned.includes(tab);
    const guarded = s.guarded.includes(tab);
    s.owned = s.owned.filter((t) => t !== tab);
    s.guarded = s.guarded.filter((t) => t !== tab);
    if (owned) s.closed = [...s.closed.filter((t) => t !== tab), tab].slice(-CLOSED_KEPT);
    return { owned, guarded };
  });

export const jobPilotWindow = async (kind: WindowKind): Promise<number | undefined> => (await load()).windows[kind];

export const setJobPilotWindow = (kind: WindowKind, windowId: number | undefined): Promise<void> =>
  change((s) => {
    s.windows = { ...s.windows, [kind]: windowId };
  });

/** A window closed: forget it if it was a JobPilot window. */
export const forgetWindow = (windowId: number): Promise<void> =>
  change((s) => {
    for (const kind of ["normal", "private"] as const) {
      if (s.windows[kind] === windowId) s.windows = { ...s.windows, [kind]: undefined };
    }
  });

/** Tests only: drop the in-memory copy. */
export function resetOwnedCache(): void {
  state = null;
  queue = Promise.resolve();
}
