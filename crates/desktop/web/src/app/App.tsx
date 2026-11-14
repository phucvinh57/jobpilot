// The composition root of the web app: sidebar on the left, the current screen on the right.
// It is the only file that knows every context.

import clsx from "clsx";
import { useEffect, useState } from "react";

import { ApplicationScreen, PipelineScreen } from "@/contexts/applications";
import { TodayScreen } from "@/contexts/discovery";
import { TrendsScreen } from "@/contexts/trends";
import { useNav, type Screen } from "@/shared";
import { onFileDrop } from "@/shared/bridge";
import { startShell } from "@/shared/shell/shell";

import { SettingsScreen } from "./SettingsScreen";
import { SetupBanner } from "./SetupBanner";
import { Sidebar } from "./Sidebar";

function CurrentScreen({ screen }: { screen: Screen }) {
  switch (screen) {
    case "today":
      return <TodayScreen />;
    case "pipeline":
      return <PipelineScreen />;
    case "application":
      return <ApplicationScreen />;
    case "trends":
      return <TrendsScreen />;
    case "settings":
      return <SettingsScreen />;
  }
}

export function App() {
  const screen = useNav((s) => s.screen);
  const dropFile = useNav((s) => s.dropFile);
  const [dragging, setDragging] = useState(false);

  useEffect(() => startShell(), []);

  // Drop a .md/.txt/.html/.pdf file anywhere on the window to add it to Today as a Lead
  // (Today checks the file type and reports a refusal).
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
