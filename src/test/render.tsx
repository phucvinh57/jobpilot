// Render a view together with the toasts, for tests that check what a failure or a success says.

import { render } from "@testing-library/react";
import type { ReactElement } from "react";

import { Toaster } from "@/components/layout/Toaster";

export function renderWithToasts(ui: ReactElement) {
  return render(
    <>
      {ui}
      <Toaster />
    </>,
  );
}
