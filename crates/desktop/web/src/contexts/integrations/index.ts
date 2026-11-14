// Bounded context `integrations`. What JobPilot connects to: AI Providers, Gmail, browser Sessions.
//
//   domain/  types mirroring the shell's DTOs + pure helpers (no React, no shell) + tests
//   api/     the context's port: typed functions over `@/shared/bridge` (invoke / events)
//   hooks/   use-cases as React hooks: state, loading, Problem
//   ui/      screens and components; they read hooks only
//
// Other contexts and the app shell import from this file only.

export { ProvidersSection } from "./ui/ProvidersSection";
export { GmailSection } from "./ui/GmailSection";
export { SessionsSection } from "./ui/SessionsSection";
