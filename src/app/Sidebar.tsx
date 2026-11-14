// Left column: the logo, navigation (Today, Pipeline, Trends), Settings and the health line.

import clsx from "clsx";
import { ArrowUpRight, ChartPie, Inbox, LayoutDashboard, Settings, type LucideIcon } from "lucide-react";

import { Heading, HealthDot, useNav, useShell, type Screen, type SettingsSection } from "@/shared";
import { navScreen, spokenBadge, todayBadge, type NavBadge } from "@/shared/nav/nav";

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
        "flex min-h-10 w-full cursor-pointer items-center gap-2.5 rounded-button px-3 text-left text-body transition-colors",
        current ? "bg-accent-soft font-semibold text-accent-dark" : "text-ink hover:bg-track",
      )}
    >
      <Icon aria-hidden className={clsx("size-4", current ? "text-accent" : "text-muted")} />
      <span className="min-w-0 flex-1 truncate">{label}</span>
      {shown && (
        <span
          className={clsx(
            "shrink-0 rounded-full px-1.5 text-tiny font-semibold",
            shown.kind === "new" && "bg-accent text-on-fill",
            shown.kind === "needsYou" && "bg-warn text-on-fill",
            shown.kind === "total" && (current ? "text-accent-dark" : "text-subtle"),
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
  const base = "flex items-center gap-2 px-2.5 pt-3 text-left text-small text-subtle";
  const focus = health && !health.ok ? health.focus : null;
  if (!focus) {
    return (
      <p id="health" className={base}>
        {body}
      </p>
    );
  }
  return (
    <button id="health" type="button" aria-label={text} onClick={() => openSettings(focus as SettingsSection)} className={clsx(base, "cursor-pointer hover:text-ink")}>
      {body}
    </button>
  );
}

export function Sidebar() {
  const screen = useNav((s) => s.screen);
  const detailFrom = useNav((s) => s.detailFrom);
  const needsYou = useNav((s) => s.needsYou);
  const show = useNav((s) => s.show);
  const status = useShell((s) => s.status);
  const current = navScreen(screen, detailFrom);
  const entry = (target: Screen) => ({ current: current === target, onClick: () => show(target) });

  return (
    <aside className="flex h-full w-56 shrink-0 flex-col gap-1 border-r border-border bg-surface-alt px-3 py-5 text-ink">
      <div className="flex items-center gap-2.5 px-2.5 pt-1 pb-5">
        <span className="flex size-[26px] shrink-0 items-center justify-center rounded-full border-2 border-accent">
          <ArrowUpRight aria-hidden className="size-3 text-accent" />
        </span>
        <Heading>JobPilot</Heading>
      </div>
      <nav aria-label="Navigation" className="flex flex-col gap-1">
        <NavItem id="nav-today" icon={Inbox} label="Today" badge={todayBadge(needsYou, status?.newLeads ?? 0)} {...entry("today")} />
        <NavItem
          id="nav-pipeline"
          icon={LayoutDashboard}
          label="Pipeline"
          badge={{ kind: "total", count: status?.applications ?? 0 }}
          {...entry("pipeline")}
        />
        <NavItem id="nav-trends" icon={ChartPie} label="Trends" {...entry("trends")} />
      </nav>
      <div className="flex-1" />
      <NavItem id="open-settings" icon={Settings} label="Settings" {...entry("settings")} />
      <HealthLine />
    </aside>
  );
}
