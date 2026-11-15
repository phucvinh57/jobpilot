// The app is square: no view rounds a corner. The only exception is a small status dot
// (`size-1.5` / `size-2` with `rounded-full`, like HealthDot).

import { readdirSync, readFileSync, statSync } from "node:fs";
import { join, relative } from "node:path";

import { describe, expect, it } from "vitest";

const SRC = join(__dirname, "..", "..");

function tsxFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    if (statSync(path).isDirectory()) {
      return tsxFiles(path);
    }
    return path.endsWith(".tsx") && !path.endsWith(".test.tsx") ? [path] : [];
  });
}

/** A radius class (any variant prefix), except `rounded-none`. */
const ROUNDED =
  /(?<![\w-])(?:[\w-[\]=&>*:]+:)?rounded(?:-(?:t|r|b|l|tl|tr|br|bl|s|e|ss|se|es|ee))?(?:-(?!none\b)[\w[\]().%/-]+)?(?![\w-])/g;

/** A class string that draws a small dot may be round. */
function isDot(line: string): boolean {
  return /\brounded-full\b/.test(line) && /\bsize-(?:1|1\.5|2)\b/.test(line);
}

describe("square design", () => {
  it("rounds no corner outside the status dots", () => {
    const offenders: string[] = [];
    for (const file of tsxFiles(SRC)) {
      readFileSync(file, "utf8")
        .split("\n")
        .forEach((line, i) => {
          if (/^\s*(\/\/|\*|\/\*)/.test(line)) {
            return;
          }
          const hits = (line.match(ROUNDED) ?? []).filter((c) => !c.endsWith("rounded-none"));
          if (hits.length > 0 && !isDot(line)) {
            offenders.push(`${relative(SRC, file)}:${i + 1}: ${hits.join(" ")}`);
          }
        });
    }
    expect(offenders).toEqual([]);
  });

  it("finds a rounded class when there is one", () => {
    expect("p-2 rounded-md".match(ROUNDED)).toEqual(["rounded-md"]);
    expect("hover:rounded-card".match(ROUNDED)).toEqual(["hover:rounded-card"]);
    expect("rounded".match(ROUNDED)).toEqual(["rounded"]);
    expect("rounded-[4px] first:rounded-l-md".match(ROUNDED)).toEqual(["rounded-[4px]", "first:rounded-l-md"]);
  });
});
