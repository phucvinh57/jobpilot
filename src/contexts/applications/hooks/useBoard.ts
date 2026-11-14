import { useQuery, type Query } from "@/shared";

import { readBoard } from "../api/applicationsApi";
import type { Board } from "../domain/types";

/** The application stages with their cards and the recent activity; read again whenever stored
 *  applications change. */
export function useBoard(): Query<Board> {
  return useQuery(readBoard, [], ["applications"]);
}
