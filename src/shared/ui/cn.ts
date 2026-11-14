// `cn`: join class names (clsx) and resolve Tailwind conflicts (tailwind-merge), as the shadcn
// primitives expect. The app's own text sizes are taught to tailwind-merge so `text-small` is
// read as a font size, not a colour (otherwise `text-small text-muted` would drop one of them).

import { clsx, type ClassValue } from "clsx";
import { extendTailwindMerge } from "tailwind-merge";

const twMerge = extendTailwindMerge({
  extend: {
    theme: {
      text: ["h1", "h2", "body", "small", "tiny"],
    },
  },
});

export function cn(...inputs: ClassValue[]): string {
  return twMerge(clsx(inputs));
}
