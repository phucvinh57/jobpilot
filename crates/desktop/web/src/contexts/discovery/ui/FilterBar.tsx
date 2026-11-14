// Narrow the list: by source (when there are several), by minimum score, by location.

import { Caption, FilterChip, TextInput } from "@/shared";

import { MIN_SCORES, type QueueFilter } from "../domain/queue";
import { sourceName } from "../domain/text";
import type { Source } from "../domain/types";
import { setFilter } from "../hooks/todayStore";

interface Props {
  filter: QueueFilter;
  sources: readonly Source[];
  shown: number;
  total: number;
}

export function FilterBar({ filter, sources, shown, total }: Props) {
  return (
    <div id="queue-filters" role="group" aria-label="Filters" className="flex flex-wrap items-center gap-x-4 gap-y-2">
      {sources.length > 1 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <Caption>Source</Caption>
          <FilterChip id="flt-src-all" selected={filter.sourceId === null} onClick={() => setFilter({ sourceId: null })}>
            All
          </FilterChip>
          {sources.map((s) => (
            <FilterChip key={s.id} id={`flt-src-${s.id}`} selected={filter.sourceId === s.id} onClick={() => setFilter({ sourceId: s.id })}>
              {sourceName(s)}
            </FilterChip>
          ))}
        </div>
      )}
      <div className="flex flex-wrap items-center gap-1.5">
        <Caption>Min score</Caption>
        <FilterChip id="flt-score-any" selected={filter.minScore === null} onClick={() => setFilter({ minScore: null })}>
          Any
        </FilterChip>
        {MIN_SCORES.map((min) => (
          <FilterChip key={min} id={`flt-score-${min}`} selected={filter.minScore === min} onClick={() => setFilter({ minScore: min })}>
            {min}+
          </FilterChip>
        ))}
      </div>
      <div className="w-[200px]">
        <TextInput
          id="queue-location"
          aria-label="Location contains"
          placeholder="Location contains..."
          value={filter.location}
          onChange={(e) => setFilter({ location: e.target.value })}
        />
      </div>
      <Caption>
        {shown} of {total} shown
      </Caption>
    </div>
  );
}
