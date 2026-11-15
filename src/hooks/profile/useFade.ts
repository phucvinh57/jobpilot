import { useEffect, useState } from "react";

import { fade, FLASH_MS } from "@/lib/profile";

/** The opacity of a label that is fully visible when `token` changes, then fades out over
 *  `FLASH_MS` (see `fade`). `undefined`: nothing was shown yet, so the opacity is 0. */
export function useFade(token: number | undefined): number {
  const [opacity, setOpacity] = useState(0);
  useEffect(() => {
    if (token === undefined) {
      return;
    }
    const start = Date.now();
    setOpacity(1);
    const timer = setInterval(() => {
      const delta = (Date.now() - start) / FLASH_MS;
      setOpacity(fade(delta));
      if (delta >= 1) {
        clearInterval(timer);
      }
    }, 100);
    return () => clearInterval(timer);
  }, [token]);
  return opacity;
}
