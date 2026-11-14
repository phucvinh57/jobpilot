import { useQuery, type Query } from "@/shared";

import { readBoard } from "../api/applicationsApi";
import type { Board } from "../domain/types";

/** The board and the recent activity; read again whenever stored applications change. */
export function usePipeline(): Query<Board> {
  return useQuery(readBoard, [], ["applications"]);
}
