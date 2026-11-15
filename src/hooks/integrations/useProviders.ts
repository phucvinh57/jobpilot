// The use-cases of Settings > AI providers: the list, the dialog that adds or edits one (save, test,
// delete), and making one the default. The models of the provider are listed on their own while
// the dialog is open, so the model box can suggest them. A key typed here lives in this
// component's state until the shell has taken it to the keyring, and is never read back.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useAction, useQuery, type Action, type Query } from "@/hooks/useAsync";

import * as api from "@/api/providers";
import { draftOf, inputOf, switchKind, type Draft } from "@/lib/integrations/providers";
import { toast } from "@/stores/toast";
import type { KindDefaults, Provider, ProviderKind, Providers } from "@/types/integrations";
import { useRunner, type Runner } from "./useRunner";

/** Wait this long after the last keystroke in the address or key before listing the models. */
export const MODELS_DELAY_MS = 500;

export interface ProvidersCard {
  query: Query<Providers>;
  kinds: KindDefaults[];
  /** The provider in the dialog; `undefined` while it is closed. */
  draft: Draft | undefined;
  /** The models of the provider in the dialog are being listed. */
  modelsLoading: boolean;
  runner: Runner;
  open: (provider: Provider | undefined) => void;
  close: () => void;
  edit: (patch: Partial<Draft>) => void;
  setKind: (kind: ProviderKind) => void;
  save: () => void;
  test: () => void;
  /** Two steps: the first click arms the button, the second deletes. */
  remove: () => void;
  /** Make a listed provider the one generation uses. */
  makeDefault: Action<[Provider], boolean>;
}

/** The models of the provider in the dialog, listed again when its kind, address or typed key
 *  change. A failure only leaves the list empty: the model id can always be typed. */
function useModels(draft: Draft | undefined, setModels: (models: string[]) => void): boolean {
  const [loading, setLoading] = useState(false);
  const reads = useRef(0);
  const open = draft !== undefined;
  const id = draft?.id ?? 0;
  const kind = draft?.kind;
  const baseUrl = draft?.baseUrl.trim() ?? "";
  const apiKey = draft?.apiKey.trim() ?? "";
  const latest = useRef(draft);
  latest.current = draft;

  // biome-ignore lint/correctness/useExhaustiveDependencies: the list depends on `id`, `kind`, `baseUrl` and `apiKey` (read through `latest`), not on the rest of the form
  useEffect(() => {
    const read = ++reads.current;
    const form = latest.current;
    if (!open || !form || baseUrl === "") {
      setLoading(false);
      return;
    }
    setLoading(true);
    const timer = setTimeout(() => {
      api
        .listModels(inputOf(form))
        .then(
          (models) => read === reads.current && setModels(models),
          () => read === reads.current && setModels([]),
        )
        .finally(() => read === reads.current && setLoading(false));
    }, MODELS_DELAY_MS);
    return () => clearTimeout(timer);
  }, [open, id, kind, baseUrl, apiKey, setModels]);

  // Answers after the dialog closed are dropped.
  useEffect(
    () => () => {
      reads.current += 1;
    },
    [],
  );
  return loading;
}

export function useProviders(): ProvidersCard {
  const query = useQuery(api.loadProviders, []);
  const runner = useRunner();
  const [draft, setDraft] = useState<Draft>();
  const kinds = useMemo(() => query.data?.kinds ?? [], [query.data]);
  const { reload } = query;
  const { reset, run, setNotice } = runner;

  const edit = useCallback((patch: Partial<Draft>) => setDraft((d) => d && { ...d, ...patch }), []);
  const setModels = useCallback((models: string[]) => edit({ models }), [edit]);
  const modelsLoading = useModels(draft, setModels);

  const open = useCallback(
    (provider: Provider | undefined) => {
      reset();
      setDraft(draftOf(provider, kinds));
      if (provider) {
        // Whether a key is saved: the key itself is never read.
        api.hasKey(provider.id).then(
          (hasKey) => setDraft((d) => (d && d.id === provider.id ? { ...d, hasKey } : d)),
          () => undefined,
        );
      }
    },
    [kinds, reset],
  );

  const close = useCallback(() => {
    reset();
    setDraft(undefined);
  }, [reset]);

  const setKind = useCallback(
    (kind: ProviderKind) => {
      reset();
      setDraft((d) => d && switchKind(d, kind, kinds));
    },
    [kinds, reset],
  );

  /** Run `call` with a form that is not armed for deletion any more. */
  const withDraft = useCallback(
    async <T>(what: string, title: string, call: (form: Draft) => Promise<T>): Promise<T | undefined> => {
      if (!draft) {
        return undefined;
      }
      setDraft({ ...draft, confirmDelete: false });
      return run(what, title, () => call(draft));
    },
    [draft, run],
  );

  const save = useCallback(() => {
    void withDraft("saving", "Could not save", (form) => api.saveProvider(inputOf(form))).then((saved) => {
      if (!saved) {
        return;
      }
      // The dialog closes: what was typed as a key is in the keyring now.
      close();
      toast.ok("Provider saved", saved.name);
      reload();
    });
  }, [withDraft, close, reload]);

  const test = useCallback(() => {
    void withDraft("testing", "The test failed", (form) => api.testProvider(inputOf(form)).then(() => true)).then(
      (ok) => {
        if (ok) {
          setNotice("✓ Works");
        }
      },
    );
  }, [withDraft, setNotice]);

  const remove = useCallback(() => {
    if (!draft) {
      return;
    }
    if (draft.id === 0) {
      close();
      return;
    }
    if (!draft.confirmDelete) {
      edit({ confirmDelete: true });
      return;
    }
    void run("saving", "Could not delete", () => api.removeProvider(draft.id).then(() => true)).then((ok) => {
      if (ok) {
        close();
        toast.ok("Provider deleted", draft.name);
        reload();
      }
    });
  }, [draft, close, edit, run, reload]);

  const makeDefault = useAction(async (provider: Provider) => {
    await api.makeDefault(provider.id);
    toast.ok("Default provider", `${provider.name} is used for generation now.`);
    reload();
    return true;
  });

  return { query, kinds, draft, modelsLoading, runner, open, close, edit, setKind, save, test, remove, makeDefault };
}
