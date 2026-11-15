// Narrow the list: open leads or dismissed jobs first; then, for the open leads, by source (when there
// are several), by minimum score, by location; for the dismissed jobs, by company or role.

import { Caption, FilterChip, TextInput } from "@/components/ui/kit";

import { MIN_SCORES, type QueueFilter } from "@/lib/discovery/queue";
import { sourceName } from "@/lib/discovery/text";
import type { Source } from "@/types/discovery";
import { setFilter, setShow } from "@/stores/today";

interface Props {
  filter: QueueFilter;
  sources: readonly Source[];
  /** Rows shown after the filters. */
  shown: number;
  /** Rows of the list picked (open leads, or dismissed jobs) before the filters. */
  total: number;
  /** How many jobs are dismissed. */
  dismissed: number;
}

/** "Show: Open | Dismissed N". */
function ShowChips({ filter, dismissed }: Pick<Props, "filter" | "dismissed">) {
  return (
    <div className="flex flex-wrap items-center gap-1.5">
      <Caption>Show</Caption>
      <FilterChip id="flt-show-open" selected={filter.show === "open"} onClick={() => setShow("open")}>
        Open
      </FilterChip>
      <FilterChip id="flt-show-dismissed" selected={filter.show === "dismissed"} onClick={() => setShow("dismissed")}>
        Dismissed {dismissed}
      </FilterChip>
    </div>
  );
}

export function FilterBar({ filter, sources, shown, total, dismissed }: Props) {
  if (filter.show === "dismissed") {
    return (
      <div id="queue-filters" role="group" aria-label="Filters" className="flex flex-wrap items-center gap-x-4 gap-y-2">
        <ShowChips filter={filter} dismissed={dismissed} />
        <div className="w-[240px]">
          <TextInput
            id="dismissed-search"
            aria-label="Company or role contains"
            placeholder="Company or role..."
            value={filter.search}
            onChange={(e) => setFilter({ search: e.target.value })}
          />
        </div>
        <Caption>
          {shown} of {total} shown
        </Caption>
      </div>
    );
  }
  return (
    <div id="queue-filters" role="group" aria-label="Filters" className="flex flex-wrap items-center gap-x-4 gap-y-2">
      <ShowChips filter={filter} dismissed={dismissed} />
      {sources.length > 1 && (
        <div className="flex flex-wrap items-center gap-1.5">
          <Caption>Source</Caption>
          <FilterChip
            id="flt-src-all"
            selected={filter.sourceId === null}
            onClick={() => setFilter({ sourceId: null })}
          >
            All
          </FilterChip>
          {sources.map((s) => (
            <FilterChip
              key={s.id}
              id={`flt-src-${s.id}`}
              selected={filter.sourceId === s.id}
              onClick={() => setFilter({ sourceId: s.id })}
            >
              {sourceName(s)}
            </FilterChip>
          ))}
        </div>
      )}
      <div className="flex flex-wrap items-center gap-1.5">
        <Caption>Min score</Caption>
        <FilterChip
          id="flt-score-any"
          selected={filter.minScore === null}
          onClick={() => setFilter({ minScore: null })}
        >
          Any
        </FilterChip>
        {MIN_SCORES.map((min) => (
          <FilterChip
            key={min}
            id={`flt-score-${min}`}
            selected={filter.minScore === min}
            onClick={() => setFilter({ minScore: min })}
          >
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
