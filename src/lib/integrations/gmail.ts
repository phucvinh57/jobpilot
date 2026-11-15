// What the Gmail card says about the connection and the last sync.

import type { SyncReport } from "@/types/integrations";

function pad(n: number): string {
  return String(n).padStart(2, "0");
}

/** `2026-11-14 09:05` in the user's time zone. */
export function formatSyncTime(at: Date): string {
  return `${at.getFullYear()}-${pad(at.getMonth() + 1)}-${pad(at.getDate())} ${pad(at.getHours())}:${pad(at.getMinutes())}`;
}

export function lastSyncLine(lastSync: string | null): string {
  return lastSync === null ? "Not synced yet" : `Last sync: ${formatSyncTime(new Date(lastSync))}`;
}

/** What the last sync did, and whether it went well (an empty line before the first sync). When it
 *  did not, `text` is the problem; the card tags it "Sync problem". */
export function syncResult(report: SyncReport | null): { text: string; ok: boolean } {
  if (report === null) {
    return { text: "", ok: true };
  }
  if (report.error !== null) {
    return { text: report.error, ok: false };
  }
  let text = `fetched ${report.fetched}, updated ${report.updated}`;
  if (report.needsReview > 0) {
    const verb = report.needsReview === 1 ? "needs" : "need";
    text += `, ${report.needsReview} ${verb} review`;
  }
  return { text, ok: true };
}

export function pollLine(minutes: number): string {
  return `New mail is checked every ${minutes} min while JobPilot is open.`;
}

/** The label of the client secret field: a saved secret stays unless a new one is typed. */
export function secretLabel(hasClientSecret: boolean): string {
  return hasClientSecret ? "Client secret (leave empty to keep the saved one)" : "Client secret";
}

export function disconnectLabel(confirming: boolean): string {
  return confirming ? "Click again to disconnect" : "Disconnect";
}
