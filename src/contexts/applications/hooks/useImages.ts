import { useCallback, useEffect, useRef, useState } from "react";

import { readShot } from "../api/applicationsApi";
import type { ShotKind } from "../domain/types";

/** Object URLs for a list of images: replacing the list frees the old URLs, and so does leaving. */
export function usePageUrls(): { urls: string[]; replace: (blobs: Blob[]) => void } {
  const [urls, setUrls] = useState<string[]>([]);
  const current = useRef<string[]>([]);

  const replace = useCallback((blobs: Blob[]) => {
    const next = blobs.map((blob) => URL.createObjectURL(blob));
    const old = current.current;
    current.current = next;
    setUrls(next);
    for (const url of old) {
      URL.revokeObjectURL(url);
    }
  }, []);

  useEffect(
    () => () => {
      for (const url of current.current) {
        URL.revokeObjectURL(url);
      }
      current.current = [];
    },
    [],
  );

  return { urls, replace };
}

/** One screenshot of a form run as an object URL (`undefined` while it loads or when it is gone).
 *  `version` makes it read again (a new review, a new run). */
export function useShot(id: number, kind: ShotKind, index: number, version: unknown): string | undefined {
  const [url, setUrl] = useState<string>();
  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` is the trigger to read again.
  useEffect(() => {
    let stopped = false;
    let made: string | undefined;
    readShot(id, kind, index).then(
      (blob) => {
        if (stopped) {
          return;
        }
        made = URL.createObjectURL(blob);
        setUrl(made);
      },
      () => {
        if (!stopped) {
          setUrl(undefined);
        }
      },
    );
    return () => {
      stopped = true;
      if (made !== undefined) {
        URL.revokeObjectURL(made);
      }
    };
  }, [id, kind, index, version]);
  return url;
}
