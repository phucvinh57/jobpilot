// What the rest of the app may use from the bridge: subscriptions to the shell's events and their
// types. The raw calls (`invoke`, `listen`, `pickFile`, `notifyChanged`) stay inside `src/api/`
// (Biome refuses `@/api/bridge` anywhere else).

export {
  onDataChanged,
  onFileDrop,
  onGraphProgress,
  type FileDrop,
  type GraphProgress,
  type Scope,
} from "./bridge";
