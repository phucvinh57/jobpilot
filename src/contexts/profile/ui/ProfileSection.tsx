import { Undo2 } from "lucide-react";

import { Button, Caption, ErrorBanner, Section, Segmented } from "@/shared";

import { JOB_TYPES } from "../domain/profile";
import { useFade } from "../hooks/useFade";
import { useProfile } from "../hooks/useProfile";
import { NotesField } from "./NotesField";
import { Row } from "./Row";
import { TagField } from "./TagField";

const TITLE = "Profile & preferences";
const HINT = "What to look for. Every change is saved at once and used by the next search.";

/** Settings > Profile & preferences: the job type, tag fields for locations, domains and skills,
 *  and free-text notes (what to look for; the facts about the user are in Settings > About you). There is no chat and no primary button: every change
 *  is saved at once, and Undo puts back the last one. */
export function ProfileSection() {
  const card = useProfile();
  const { profile } = card;
  const opacity = useFade(card.flash?.n);

  const actions = (
    <div className="flex items-center gap-2">
      {card.flash && opacity > 0 && (
        <span
          id="profile-saved"
          role="status"
          aria-label={card.flash.label}
          style={{ opacity }}
          className="text-small text-ok"
        >
          {card.flash.label}
        </span>
      )}
      <Button
        id="profile-undo"
        variant="ghost"
        size="compact"
        icon={Undo2}
        disabled={!card.canUndo}
        onClick={card.undo}
      >
        Undo
      </Button>
    </div>
  );

  if (!profile) {
    return (
      <Section id="settings-profile" title={TITLE} hint={HINT} actions={actions}>
        {card.loadError ? (
          <ErrorBanner
            id="profile-error"
            error={card.loadError}
            title="Could not read your preferences"
            onRetry={card.retry}
          />
        ) : (
          <Caption>Loading...</Caption>
        )}
      </Section>
    );
  }

  return (
    <Section id="settings-profile" title={TITLE} hint={HINT} actions={actions}>
      <div className="flex flex-col gap-4">
        <Row label="Job type">
          <div id="profile-job-type">
            <Segmented label="Job type" choices={JOB_TYPES} value={profile.jobType} onChange={card.setJobType} />
          </div>
        </Row>
        {profile.fields.map((field) => (
          <TagField
            key={field.key}
            field={field}
            onAdd={(text) => card.addItem(field.key, text)}
            onRemove={(item) => card.removeItem(field.key, item)}
          />
        ))}
        <NotesField
          text={card.notesText}
          onChange={card.setNotesText}
          onSave={card.saveNotes}
          canSave={card.canSaveNotes}
          status={card.notesStatus}
        />
        {card.error && (
          <ErrorBanner id="profile-error" error={card.error} title="Not saved" onDismiss={card.clearError} />
        )}
      </div>
    </Section>
  );
}
