import { useAction, useQuery, type Action, type Query } from "@/hooks/useAsync";

import { readJd, readTimeline, resolveEmail } from "@/api/applications";
import type { Jd, Timeline } from "@/types/applications";

/** The posting, its requirements and the company brief. */
export function useJd(id: number): Query<Jd> {
  return useQuery(() => readJd(id), [id], ["applications"]);
}

export interface TimelineModel {
  query: Query<Timeline>;
  /** Accept or dismiss an email the classifier was unsure about. */
  resolve: Action<[emailId: string, accept: boolean], void>;
}

/** The events of an application and the emails that wait for the user. */
export function useTimeline(id: number): TimelineModel {
  const query = useQuery(() => readTimeline(id), [id], ["applications"]);
  const resolve = useAction((emailId: string, accept: boolean) => resolveEmail(emailId, accept));
  return { query, resolve };
}
