// Page-level rules of the Jobs page: the stage bar's items and the room the drawer takes.

import type { DrawerTarget, Stage } from "@/shared";

export interface StageItem {
  key: Stage;
  label: string;
  count: number;
}

/** The stage bar: Leads (counting the new ones), then the application stages with their cards. */
export function stageItems(newLeads: number, appStages: readonly StageItem[]): StageItem[] {
  return [{ key: "leads", label: "Leads", count: newLeads }, ...appStages];
}

/** Right padding that keeps the list visible next to the open drawer, on screens wide enough for
 *  both (the lead drawer is 480 px wide, the application drawer 760 px; the sidebar takes 224 px). */
export function drawerPad(drawer: DrawerTarget | undefined): string {
  switch (drawer?.kind) {
    case undefined:
      return "";
    case "lead":
      return "xl:pr-[480px]";
    case "app":
      return "2xl:pr-[760px]";
  }
}
