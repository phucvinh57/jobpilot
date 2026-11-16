// The debugger session of an owned tab: attach and set it up, and send with one re-attach when Chrome
// says the debugger is gone (the user pressed Cancel on the "started debugging" bar, or a restart).

import { attach, command, isNotAttached } from "./cdp";
import { intercept } from "./guard";
import { isGuarded } from "./owned";
import type { Params } from "./protocol";

/** Attach and enable what every JobPilot tab needs. Automation is never hidden. */
export async function prepare(tab: number): Promise<void> {
  await attach(tab);
  await command(tab, "Emulation.setAutomationOverride", { enabled: true });
  await command(tab, "Emulation.setFocusEmulationEnabled", { enabled: true });
  await command(tab, "Page.enable");
  await command(tab, "Runtime.enable");
}

/** Send to an owned tab; if the debugger was detached, attach again once and retry. */
export async function send(tab: number, method: string, params: Params = {}, session?: string): Promise<Params> {
  try {
    return await command(tab, method, params, session);
  } catch (error) {
    if (!isNotAttached(error)) throw error;
    console.info("jobpilot: re-attaching the debugger", tab);
    await prepare(tab);
    if (await isGuarded(tab)) await intercept(tab);
    return await command(tab, method, params, session);
  }
}
