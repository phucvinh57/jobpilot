// The root component: sidebar on the left, the current page on the right.

import clsx from "clsx";
import { useEffect, useState } from "react";

import { onFileDrop } from "@/api/events";
import { SetupBanner } from "@/components/layout/SetupBanner";
import { Sidebar } from "@/components/layout/Sidebar";
import { ApplicationPage } from "@/pages/ApplicationPage";
import { JobsPage } from "@/pages/JobsPage";
import { ProfilePage } from "@/pages/ProfilePage";
import { SettingsPage } from "@/pages/SettingsPage";
import { TrendsPage } from "@/pages/TrendsPage";
import { useNav, type Screen } from "@/stores/nav";
import { startShell } from "@/stores/shell";

function CurrentScreen({ screen }: { screen: Screen }) {
  switch (screen) {
    case "jobs":
      return <JobsPage />;
    case "application":
      return <ApplicationPage />;
    case "trends":
      return <TrendsPage />;
    case "profile":
      return <ProfilePage />;
    case "settings":
      return <SettingsPage />;
  }
}

export function App() {
  const screen = useNav((s) => s.screen);
  const dropFile = useNav((s) => s.dropFile);
  const [dragging, setDragging] = useState(false);

  useEffect(() => startShell(), []);

  // Drop a .md/.txt/.html/.pdf file anywhere on the window to add it as a Lead: navigation shows
  // the Leads stage, whose lifecycle reads the file (it checks the type and reports a refusal).
  useEffect(
    () =>
      onFileDrop((drop) => {
        setDragging(drop.type === "over");
        if (drop.type === "drop" && drop.paths[0] !== undefined) {
          dropFile(drop.paths[0]);
        }
      }),
    [dropFile],
  );

  // The webview's own menu (Reload, Back) has no place in a desktop app; text fields keep theirs.
  useEffect(() => {
    const block = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null;
      if (!target?.closest("input, textarea, .cm-editor, .selectable")) {
        e.preventDefault();
      }
    };
    window.addEventListener("contextmenu", block);
    return () => window.removeEventListener("contextmenu", block);
  }, []);

  return (
    <div className={clsx("flex h-full text-ink", dragging ? "bg-accent-soft" : "bg-bg")}>
      <Sidebar />
      <div className="flex h-full min-w-0 flex-1 flex-col">
        <SetupBanner />
        <div className="min-h-0 flex-1" key={screen}>
          <CurrentScreen screen={screen} />
        </div>
      </div>
    </div>
  );
}
