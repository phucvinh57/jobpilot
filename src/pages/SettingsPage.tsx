// Settings: one scrolling page of cards, in this order: About you, Profile & preferences, Connections (AI
// providers, Gmail, Browser sessions), Job sources, Appearance. Each card belongs to its context;
// this file only composes them and scrolls to the one a call to action names.

import { useEffect, type ReactNode } from "react";

import { JobSourcesSection } from "@/components/discovery/JobSourcesSection";
import { GmailSection } from "@/components/integrations/GmailSection";
import { ProvidersSection } from "@/components/integrations/ProvidersSection";
import { SessionsSection } from "@/components/integrations/SessionsSection";
import { AboutYouSection } from "@/components/profile/AboutYouSection";
import { ProfileSection } from "@/components/profile/ProfileSection";
import { APPEARANCES, type Appearance } from "@/lib/theme";
import { Caption, Heading, Page, Section, Segmented } from "@/components/ui/kit";
import { ErrorBanner } from "@/components/ProblemHelp";
import { sectionId, useNav, type SettingsSection } from "@/stores/nav";
import { useAction } from "@/hooks/useAsync";
import { useShell } from "@/stores/shell";

function Slot({ section, children }: { section: SettingsSection; children: ReactNode }) {
  return (
    <div id={sectionId(section)} className="scroll-mt-4">
      {children}
    </div>
  );
}

/** Light, dark, or whatever the desktop uses. */
function AppearanceSection() {
  const appearance = useShell((s) => s.appearance);
  const setAppearance = useShell((s) => s.setAppearance);
  const save = useAction((value: Appearance) => setAppearance(value));
  return (
    <Section title="Appearance" hint="Light, dark, or follow your desktop.">
      <Segmented
        label="Appearance"
        choices={APPEARANCES}
        value={appearance}
        onChange={(value) => void save.run(value)}
      />
      {save.error && (
        <ErrorBanner error={save.error} title="The appearance was not saved" onDismiss={save.clearError} />
      )}
    </Section>
  );
}

export function SettingsPage() {
  const focus = useNav((s) => s.settingsFocus);
  const clearFocus = useNav((s) => s.clearSettingsFocus);
  const refresh = useShell((s) => s.refresh);

  // Opened from a call to action ("Open Settings > Browser sessions"): scroll to that section.
  useEffect(() => {
    if (focus) {
      document.getElementById(sectionId(focus))?.scrollIntoView({ block: "start" });
      clearFocus();
    }
  }, [focus, clearFocus]);

  // Leaving Settings: a provider, Chrome or typst may have been set up meanwhile.
  useEffect(() => refresh, [refresh]);

  return (
    <Page label="Settings" title="Settings">
      <Slot section="About you">
        <AboutYouSection />
      </Slot>
      <Slot section="Profile & preferences">
        <ProfileSection />
      </Slot>
      <div role="group" aria-label="Connections" className="flex flex-col gap-0.5">
        <Heading>Connections</Heading>
        <Caption>Keys and tokens are kept in your system keyring, never in files.</Caption>
      </div>
      <Slot section="AI providers">
        <ProvidersSection />
      </Slot>
      <Slot section="Gmail">
        <GmailSection />
      </Slot>
      <Slot section="Browser sessions">
        <SessionsSection />
      </Slot>
      <Slot section="Job sources">
        <JobSourcesSection />
      </Slot>
      <Slot section="Appearance">
        <AppearanceSection />
      </Slot>
    </Page>
  );
}
