import { Button, Spinner, TextArea } from "@/components/ui/kit";

import { saveShortcut, type NotesStatus } from "@/lib/profile";
import { Row } from "./Row";

/** "Anything else": the user's own words. Saving them lets the model read them once in the
 *  background; anything but a clean read is a muted line under the box, never a blocker. */
export function NotesField({
  text,
  onChange,
  onSave,
  canSave,
  status,
}: {
  text: string;
  onChange: (text: string) => void;
  onSave: () => void;
  canSave: boolean;
  status: NotesStatus;
}) {
  const shortcut = saveShortcut(typeof navigator === "undefined" ? "" : navigator.platform);
  return (
    <Row label="Anything else" hint="Seniority, salary, visa, company type">
      <div className="flex flex-col gap-2">
        <TextArea
          id="profile-notes"
          aria-label="Anything else"
          rows={3}
          placeholder="Seniority, salary, visa, the kind of company: say it in your own words."
          value={text}
          onChange={(e) => onChange(e.target.value)}
          onKeyDown={(e) => {
            // Enter adds a line; Ctrl/Cmd+Enter saves.
            if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) {
              e.preventDefault();
              onSave();
            }
          }}
        />
        <div className="flex items-center gap-2">
          <Button id="profile-notes-save" disabled={!canSave} onClick={onSave}>
            Save
          </Button>
          <span className="text-small text-muted">or {shortcut}</span>
        </div>
        {status.kind === "reading" && (
          <div
            id="profile-notes-reading"
            role="status"
            aria-label="Reading your notes"
            className="flex items-center gap-2 text-small text-muted"
          >
            <Spinner className="size-3.5" />
            Reading your notes to fill in the rest...
          </div>
        )}
        {status.kind === "failed" && (
          <p
            id="profile-notes-unread"
            role="status"
            aria-label="Notes saved, not read"
            className="selectable text-small text-muted"
          >
            {status.line}
          </p>
        )}
      </div>
    </Row>
  );
}
