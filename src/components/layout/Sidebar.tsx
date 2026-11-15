// Left column: the logo, navigation (Jobs, Trends), Settings and the health line.

import clsx from "clsx";
import { ArrowUpRight, ChartPie, Inbox, Settings, type LucideIcon } from "lucide-react";

import { Heading, HealthDot } from "@/components/ui/kit";
import { useNav, type Screen, type SettingsSection } from "@/stores/nav";
import { useShell } from "@/stores/shell";
import { jobsBadge, navScreen, spokenBadge, type NavBadge } from "@/stores/nav";

/** One entry of the navigation: icon, label, an optional count badge (shown from 1). */
function NavItem({
  id,
  icon: Icon,
  label,
  badge,
  current,
  onClick,
}: {
  id: string;
  icon: LucideIcon;
  label: string;
  badge?: NavBadge;
  current: boolean;
  onClick: () => void;
}) {
  const shown = badge && badge.count > 0 ? badge : undefined;
  return (
    <button
      id={id}
      type="button"
      aria-label={shown ? `${label}, ${spokenBadge(shown)}` : label}
      aria-current={current ? "page" : undefined}
      onClick={onClick}
      className={clsx(
        "flex h-9 w-full cursor-pointer items-center gap-2.5 border-l-2 px-3 text-left text-body transition-colors",
        current
          ? "border-accent bg-track font-medium text-ink"
          : "border-transparent text-muted hover:bg-track hover:text-ink",
      )}
    >
      <Icon aria-hidden className={clsx("size-4", current ? "text-accent" : "text-muted")} />
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {shown && (
        <span
          className={clsx(
            "shrink-0 px-1.5 text-tiny font-semibold tabular-nums",
            shown.kind === "new" && "bg-accent text-on-fill",
            shown.kind === "needsYou" && "bg-warn text-on-fill",
            shown.kind === "total" && (current ? "text-ink" : "text-subtle"),
          )}
        >
          {shown.count}
        </span>
      )}
    </button>
  );
}

/** The health line: a dot and a few words. When something is missing and Settings has a section
 *  that fixes it, pressing the line opens that section. */
function HealthLine() {
  const health = useShell((s) => s.status?.health);
  const openSettings = useNav((s) => s.openSettings);
  const text = health?.text ?? "Checking setup";
  const body = (
    <>
      <HealthDot state={health ? (health.ok ? "ok" : "warn") : "checking"} />
      <span className="min-w-0 flex-1 truncate">{text}</span>
    </>
  );
  const base = "flex items-center gap-2 border-t border-border px-3 pt-3 mt-2 text-left text-small text-subtle";
  const focus = health && !health.ok ? health.focus : null;
  if (!focus) {
    return (
      <p id="health" className={base}>
        {body}
      </p>
    );
  }
  return (
    <button
      id="health"
      type="button"
      aria-label={text}
      onClick={() => openSettings(focus as SettingsSection)}
      className={clsx(base, "cursor-pointer hover:text-ink")}
    >
      {body}
    </button>
  );
}

export function Sidebar() {
  const screen = useNav((s) => s.screen);
  const needsYou = useNav((s) => s.needsYou);
  const show = useNav((s) => s.show);
  const status = useShell((s) => s.status);
  const current = navScreen(screen);
  const entry = (target: Screen) => ({ current: current === target, onClick: () => show(target) });

  return (
    <aside className="flex h-full w-56 shrink-0 flex-col gap-0.5 border-r border-border bg-surface-alt px-2 py-4 text-ink">
      <div className="flex items-center gap-2.5 px-3 pt-1 pb-5">
        <span className="flex size-6 shrink-0 items-center justify-center bg-accent">
          <ArrowUpRight aria-hidden className="size-3.5 text-on-fill" />
        </span>
        <Heading>JobPilot</Heading>
      </div>
      <nav aria-label="Navigation" className="flex flex-col gap-0.5">
        <NavItem
          id="nav-jobs"
          icon={Inbox}
          label="Jobs"
          badge={jobsBadge(needsYou, status?.newLeads ?? 0)}
          {...entry("jobs")}
        />
        <NavItem id="nav-trends" icon={ChartPie} label="Trends" {...entry("trends")} />
      </nav>
      <div className="flex-1" />
      <NavItem id="open-settings" icon={Settings} label="Settings" {...entry("settings")} />
      <HealthLine />
    </aside>
  );
}
