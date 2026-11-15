// Profile: one scrolling page of two cards, About you (the documents the profile is read from and
// what was understood) then Preferences (what to look for). Both belong to the profile context;
// this file only composes them and scrolls to the one a call to action names.

import { useEffect, type ReactNode } from "react";

import { AboutYouSection } from "@/components/profile/AboutYouSection";
import { ProfileSection } from "@/components/profile/ProfileSection";
import { Page } from "@/components/ui/kit";
import { sectionId, useNav, type ProfileSection as Section } from "@/stores/nav";
import { useShell } from "@/stores/shell";

function Slot({ section, children }: { section: Section; children: ReactNode }) {
  return (
    <div id={sectionId(section)} className="scroll-mt-4">
      {children}
    </div>
  );
}

export function ProfilePage() {
  const focus = useNav((s) => s.profileFocus);
  const clearFocus = useNav((s) => s.clearProfileFocus);
  const refresh = useShell((s) => s.refresh);

  // Opened from a call to action ("Open Profile > About you"): scroll to that section.
  useEffect(() => {
    if (focus) {
      document.getElementById(sectionId(focus))?.scrollIntoView({ block: "start" });
      clearFocus();
    }
  }, [focus, clearFocus]);

  // Leaving Profile: a profile may have been read, or the preferences saved, meanwhile.
  useEffect(() => refresh, [refresh]);

  return (
    <div id="profile" className="h-full">
      <Page
        label="Profile"
        title="Profile"
        subtitle="Your documents, what JobPilot read from them, and the jobs you are looking for."
      >
        <Slot section="About you">
          <AboutYouSection />
        </Slot>
        <Slot section="Preferences">
          <ProfileSection />
        </Slot>
      </Page>
    </div>
  );
}
