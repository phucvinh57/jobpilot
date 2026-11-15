import { useEffect, useRef } from "react";

import { onGraphProgress, type GraphProgress } from "@/api/bridge";

/** Call `handler` for every step of every graph run (discovery, trends, prepare-CV, chat). */
export function useGraphProgress(handler: (progress: GraphProgress) => void): void {
  const ref = useRef(handler);
  ref.current = handler;
  useEffect(() => onGraphProgress((p) => ref.current(p)), []);
}
