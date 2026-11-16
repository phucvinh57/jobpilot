// The way out to the app, for the modules that only emit events (no import cycle with the connection).

export type Sink = (message: unknown) => boolean;

let sink: Sink | null = null;

/** The connection sets this while the app has welcomed the extension, and clears it on close. */
export function setSink(next: Sink | null): void {
  sink = next;
}

/** Whether the app is connected and authenticated. */
export const linked = (): boolean => sink !== null;

/** Send an event to the app; dropped (false) when it is not connected. */
export function emit(event: string, params: Record<string, unknown>): boolean {
  return sink?.({ type: "event", event, params }) ?? false;
}
