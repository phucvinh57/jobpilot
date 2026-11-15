import "./styles.css";

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";

import { App } from "./App";
import { applyTheme, cachedAppearance } from "@/lib/theme";

// Colours before the first frame (the shell's saved choice replaces the cached one once it answers).
applyTheme(cachedAppearance());

const root = document.getElementById("root");
if (root) {
  createRoot(root).render(
    <StrictMode>
      <App />
    </StrictMode>,
  );
}
