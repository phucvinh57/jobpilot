// Lint + the architecture rules of the web app:
//   - layers inside a context: ui -> hooks -> api / domain (never the other way)
//   - a context reaches another context only through its `index.ts`
//   - `shared` knows no context; only `shared/bridge` talks to Tauri, and only a context's
//     `api/` layer (or the app shell) calls the bridge
import js from "@eslint/js";
import importPlugin from "eslint-plugin-import";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";
import tseslint from "typescript-eslint";

const contexts = ["discovery", "applications", "profile", "integrations", "trends"];

/** A context may import another context's `index.ts` only. */
const contextZones = contexts.map((name) => ({
  target: `./src/contexts/${name}`,
  from: "./src/contexts",
  except: [`./${name}`, ...contexts.filter((c) => c !== name).map((c) => `./${c}/index.ts`)],
  message: "Import another context through its index.ts only.",
}));

/** Inside a context: domain <- api <- hooks <- ui. */
const layerZones = contexts.flatMap((name) => {
  const dir = `./src/contexts/${name}`;
  const deny = (target, from) => ({
    target: `${dir}/${target}`,
    from: `${dir}/${from}`,
    message: `${target}/ must not import ${from}/ (ui -> hooks -> api / domain).`,
  });
  return [
    deny("domain", "api"),
    deny("domain", "hooks"),
    deny("domain", "ui"),
    deny("api", "hooks"),
    deny("api", "ui"),
    deny("hooks", "ui"),
  ];
});

const tauriOnlyInBridge = {
  group: ["@tauri-apps/*"],
  message: "Use @/shared/bridge (from a context's api/ layer).",
};

export default tseslint.config(
  { ignores: ["dist", "node_modules"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: { globals: globals.browser },
    plugins: { import: importPlugin, "react-hooks": reactHooks },
    settings: { "import/resolver": { typescript: { project: "./tsconfig.json" } } },
    rules: {
      "react-hooks/rules-of-hooks": "error",
      "react-hooks/exhaustive-deps": "error",
      "import/no-restricted-paths": [
        "error",
        {
          zones: [
            ...contextZones,
            ...layerZones,
            { target: "./src/shared", from: "./src/contexts", message: "shared/ knows no context." },
            { target: "./src/shared", from: "./src/app", message: "shared/ does not depend on the app shell." },
            { target: "./src/contexts", from: "./src/app", message: "A context does not depend on the app shell." },
          ],
        },
      ],
      "no-restricted-imports": ["error", { patterns: [tauriOnlyInBridge] }],
    },
  },
  {
    files: ["src/shared/bridge/**"],
    rules: { "no-restricted-imports": "off" },
  },
  {
    // The bridge is a context's port to the shell: only its api/ layer calls it.
    files: ["src/contexts/*/domain/**", "src/contexts/*/hooks/**", "src/contexts/*/ui/**"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            tauriOnlyInBridge,
            { group: ["@/shared/bridge", "@/shared/bridge/*"], message: "Call the shell from the context's api/ layer." },
          ],
        },
      ],
    },
  },
  { files: ["*.config.{js,ts}"], languageOptions: { globals: globals.node } },
);
