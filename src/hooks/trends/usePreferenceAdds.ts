// "Add to preferences": put a topic of the digest into the saved skills or domains at once.
// Nothing is cancelled and the screen does not change; the button shows a spinner while the save
// runs (more clicks on it do nothing), then reads "Added".

import { useCallback, useRef, useState } from "react";

import { toCommandError } from "@/lib/problem";

import * as api from "@/api/trends";
import { addFailure } from "@/lib/trends/format";
import { addButton, canAdd, topicKey, type AddButton } from "@/lib/trends/preferences";
import type { PrefField, Presence } from "@/types/trends";

const NONE: ReadonlyMap<string, Presence> = new Map();

export interface PreferenceAdds {
  /** What the button of `topic` shows now; `presence` is where the last read says it stands. */
  stateOf: (field: PrefField, topic: string, presence: Presence) => AddButton;
  add: (field: PrefField, topic: string, presence: Presence) => Promise<void>;
}

/**
 * `snapshot` identifies the data the presences came from: what a save answered holds until the
 * next read replaces it, so the button never flickers back to "Add" between the two.
 */
export function usePreferenceAdds(snapshot: unknown, say: (text: string) => void): PreferenceAdds {
  const [pending, setPending] = useState<ReadonlySet<string>>(new Set());
  const [saved, setSaved] = useState<{ snapshot: unknown; presence: ReadonlyMap<string, Presence> }>();
  const latest = useRef(snapshot);
  latest.current = snapshot;
  // Synchronous twin of `pending`: a second click before the next render still finds the first save.
  const inFlight = useRef(new Set<string>());

  const known = saved !== undefined && saved.snapshot === snapshot ? saved.presence : NONE;

  const stateOf = useCallback(
    (field: PrefField, topic: string, presence: Presence): AddButton => {
      const key = topicKey(field, topic);
      return addButton(known.get(key) ?? presence, pending.has(key));
    },
    [known, pending],
  );

  const add = useCallback(
    async (field: PrefField, topic: string, presence: Presence): Promise<void> => {
      const key = topicKey(field, topic);
      if (inFlight.current.has(key) || !canAdd(stateOf(field, topic, presence))) {
        return;
      }
      inFlight.current.add(key);
      setPending((before) => new Set(before).add(key));
      try {
        const added = await api.addToPreferences(field, topic);
        setSaved((before) => {
          const base = before !== undefined && before.snapshot === latest.current ? before.presence : NONE;
          return { snapshot: latest.current, presence: new Map(base).set(key, added.presence) };
        });
        if (added.message) {
          say(added.message);
        }
      } catch (thrown) {
        say(addFailure(topic, toCommandError(thrown).message));
      } finally {
        inFlight.current.delete(key);
        setPending((before) => {
          const after = new Set(before);
          after.delete(key);
          return after;
        });
      }
    },
    [stateOf, say],
  );

  return { stateOf, add };
}
