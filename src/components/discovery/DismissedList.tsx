// The Leads stage's "Show: Dismissed" list: every dismissed job, newest first. Leads the user
// dismissed, postings the preferences filtered out and jobs dismissed from CVs, each with a plain
// Restore. A CV row opens its application in the drawer; a lead row can open its posting.

import { ExternalLink, RotateCcw } from "lucide-react";
import type { ReactNode } from "react";

import { Button, DataTable, Tag, type DataColumn } from "@/components/ui/kit";
import { openLink } from "@/api/shell";
import { useNav } from "@/stores/nav";

import { dismissedWhen, isWebLink } from "@/lib/discovery/text";
import type { DismissedJob } from "@/types/discovery";
import { dismissedKey, restoreJob } from "@/stores/today";

interface Props {
  shown: readonly DismissedJob[];
  restoring: readonly string[];
  /** The application the drawer shows, if any. */
  openAppId: number | undefined;
  /** What the table says when no dismissed job passes the search. */
  empty?: ReactNode;
}

/** A dismissed job's row as assistive technology names it (tests find rows by it). */
export function dismissedRowLabel(job: DismissedJob): string {
  return `${job.company}: ${job.role}, dismissed ${job.kind === "lead" ? "lead" : "CV"}`;
}

export function DismissedList({ shown, restoring, openAppId, empty }: Props) {
  const openAppDrawer = useNav((s) => s.openAppDrawer);
  const now = new Date();

  const columns: DataColumn<DismissedJob>[] = [
    {
      key: "role",
      header: "Role",
      // Takes the rest of the width; its text is cut with an ellipsis, the whole of it in the title.
      className: "w-full max-w-0",
      cell: (job) => (
        <span id={`dismissed-${dismissedKey(job)}`} className="flex min-w-0 flex-col gap-0.5">
          <span title={job.role} className="truncate font-semibold text-ink">
            {job.role}
          </span>
          <span title={job.company} className="truncate text-small text-muted">
            {job.company}
          </span>
        </span>
      ),
    },
    {
      key: "was",
      header: "Was",
      className: "w-16",
      cell: (job) => <Tag tone={job.kind === "lead" ? "neutral" : "accent"}>{job.kind === "lead" ? "Lead" : "CV"}</Tag>,
    },
    {
      key: "reason",
      header: "Reason",
      className: "max-w-[260px]",
      cell: (job) => (
        <span title={job.reason ?? undefined} className="block truncate text-small text-muted">
          {job.reason ?? "No reason given"}
        </span>
      ),
    },
    {
      key: "when",
      header: "When",
      className: "w-24 whitespace-nowrap text-small text-muted",
      cell: (job) => dismissedWhen(job.dismissedAt, now),
    },
    {
      key: "actions",
      header: <span className="sr-only">Actions</span>,
      className: "w-px whitespace-nowrap",
      cell: (job) => {
        const key = dismissedKey(job);
        const title = `${job.company}: ${job.role}`;
        return (
          <span className="flex items-center justify-end gap-1.5">
            {job.kind === "lead" && job.url !== null && isWebLink(job.url) && (
              <Button
                id={`dismissed-posting-${job.id}`}
                size="compact"
                variant="ghost"
                icon={ExternalLink}
                aria-label={`Open posting: ${title}`}
                onClick={() => job.url !== null && openLink(job.url)}
              >
                Open posting
              </Button>
            )}
            <Button
              id={`restore-${key}`}
              size="compact"
              icon={RotateCcw}
              aria-label={`Restore ${title}`}
              busy={restoring.includes(key)}
              onClick={() => void restoreJob(job)}
            >
              Restore
            </Button>
          </span>
        );
      },
    },
  ];

  return (
    <DataTable
      id="dismissed-list"
      label="Dismissed jobs"
      columns={columns}
      rows={shown}
      rowKey={dismissedKey}
      rowLabel={dismissedRowLabel}
      onOpen={(job) => {
        if (job.kind === "application") {
          openAppDrawer(job.id);
        }
      }}
      openKey={openAppId === undefined ? null : dismissedKey({ kind: "application", id: openAppId })}
      empty={empty}
    />
  );
}
