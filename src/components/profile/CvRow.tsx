import { FileText } from "lucide-react";

import { Button } from "@/components/ui/kit";

import type { Cv } from "@/types/profile";
import { Row } from "./Row";

/** The CV attached for suggestions. It is context only: it is never copied into a CV. */
export function CvRow({
  cv,
  busy,
  onUpload,
  onRemove,
}: {
  cv: Cv | null;
  busy: boolean;
  onUpload: () => void;
  onRemove: () => void;
}) {
  return (
    <Row label="Your CV" hint="Used only to suggest preferences, never copied into a CV">
      {cv ? (
        <div className="flex flex-wrap items-center gap-2">
          <p id="profile-cv-name" aria-label={`CV: ${cv.name}`} className="selectable max-w-[320px] truncate text-body">
            {cv.name}
          </p>
          <span className="text-small text-muted">
            {cv.chars} characters read{cv.truncated ? ", cut to the limit" : ""}
          </span>
          <Button id="profile-cv-replace" icon={FileText} busy={busy} onClick={onUpload}>
            Replace
          </Button>
          <Button id="profile-cv-remove" variant="ghost" disabled={busy} onClick={onRemove}>
            Remove
          </Button>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <Button id="profile-cv-upload" icon={FileText} busy={busy} onClick={onUpload}>
            Upload CV
          </Button>
          <span className="text-small text-muted">A .pdf, .md or .txt file.</span>
        </div>
      )}
    </Row>
  );
}
