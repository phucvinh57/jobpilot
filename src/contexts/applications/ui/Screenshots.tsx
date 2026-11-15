// Screenshots of a form run. The image bytes come from the shell and are shown through object URLs.

import type { ShotKind } from "../domain/types";
import { useShot } from "../hooks/useImages";

function Shot({
  id,
  kind,
  index,
  version,
  label,
}: {
  id: number;
  kind: ShotKind;
  index: number;
  version: unknown;
  label: string;
}) {
  const url = useShot(id, kind, index, version);
  if (url === undefined) {
    return null;
  }
  return <img src={url} alt={label} className="max-w-full border border-border" />;
}

/** `count` screenshots of one kind under a title (`count` 1 for a single one). */
export function Screenshots({
  id,
  kind,
  count,
  version,
  title,
}: {
  id: number;
  kind: ShotKind;
  count: number;
  version: unknown;
  title: string;
}) {
  return (
    <div className="flex flex-col gap-2">
      <p className="text-body font-semibold text-ink">{title}</p>
      {Array.from({ length: count }, (_, index) => (
        <Shot
          key={index}
          id={id}
          kind={kind}
          index={index}
          version={version}
          label={count === 1 ? title : `${title}, step ${index + 1}`}
        />
      ))}
    </div>
  );
}
