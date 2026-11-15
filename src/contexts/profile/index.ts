// Bounded context `profile`. The user's documents and the profile read from them (About you), and
// the job-search preferences.
//
//   domain/  types mirroring the shell's DTOs + pure helpers (no React, no shell) + tests
//   api/     the context's port: typed functions over `@/shared/bridge` (invoke / events)
//   hooks/   use-cases as React hooks: state, loading, Problem
//   ui/      screens and components; they read hooks only
//
// Other contexts and the app shell import from this file only.

export { AboutYouSection } from "./ui/AboutYouSection";
export { ProfileSection } from "./ui/ProfileSection";
