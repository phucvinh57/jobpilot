import { useQuery, type Query } from "@/hooks/useAsync";

import { readBoard } from "@/api/applications";
import type { Board } from "@/types/applications";

/** The application stages with their cards and the recent activity; read again whenever stored
 *  applications change. */
export function useBoard(): Query<Board> {
  return useQuery(readBoard, [], ["applications"]);
}
