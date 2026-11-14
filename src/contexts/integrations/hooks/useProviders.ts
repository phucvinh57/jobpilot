// The use-cases of Settings > AI providers: the list, the form (add, edit, test, load models, make
// default, delete). A key typed here lives in this component's state until the shell has taken it to
// the keyring, and is never read back.

import { useCallback, useMemo, useState } from "react";

import { useQuery, type Query } from "@/shared";

import * as api from "../api/providersApi";
import {
  draftOf,
  inputOf,
  modelsLine,
  switchKind,
  type Draft,
  type KindDefaults,
  type Provider,
  type ProviderKind,
  type Providers,
} from "../domain/providers";
import { useRunner, type Runner } from "./useRunner";

export interface ProvidersCard {
  query: Query<Providers>;
  kinds: KindDefaults[];
  /** The provider being edited. */
  draft: Draft | undefined;
  runner: Runner;
  open: (provider: Provider | undefined) => void;
  close: () => void;
  edit: (patch: Partial<Draft>) => void;
  setKind: (kind: ProviderKind) => void;
  save: () => void;
  test: () => void;
  loadModels: () => void;
  makeDefault: () => void;
  /** Two steps: the first click arms the button, the second deletes. */
  remove: () => void;
}

export function useProviders(): ProvidersCard {
  const query = useQuery(api.loadProviders, []);
  const runner = useRunner();
  const [draft, setDraft] = useState<Draft>();
  const kinds = useMemo(() => query.data?.kinds ?? [], [query.data]);
  const { reload } = query;
  const { reset, run, setNotice } = runner;

  const edit = useCallback((patch: Partial<Draft>) => setDraft((d) => d && { ...d, ...patch }), []);

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
    void withDraft("saving", "Could not save", (form) => api.saveProvider(inputOf(form)).then((saved) => ({ saved, form }))).then((done) => {
      if (!done) {
        return;
      }
      const { saved, form } = done;
      // The key box is emptied: what was typed is in the keyring now.
      setDraft((d) =>
        d && { ...d, id: saved.id, isDefault: saved.isDefault, apiKey: "", hasKey: d.hasKey || form.apiKey.trim() !== "" },
      );
      setNotice("Saved.");
      reload();
    });
  }, [withDraft, setNotice, reload]);

  const test = useCallback(() => {
    void withDraft("testing", "The test failed", (form) => api.testProvider(inputOf(form)).then(() => true)).then((ok) => {
      if (ok) {
        setNotice("✓ Works");
      }
    });
  }, [withDraft, setNotice]);

  const loadModels = useCallback(() => {
    void withDraft("models", "Could not load models", (form) => api.listModels(inputOf(form))).then((models) => {
      if (models) {
        edit({ models });
        setNotice(modelsLine(models.length));
      }
    });
  }, [withDraft, edit, setNotice]);

  const makeDefault = useCallback(() => {
    if (!draft || draft.id === 0) {
      return;
    }
    void withDraft("saving", "Could not make it the default", (form) => api.makeDefault(form.id).then(() => true)).then((ok) => {
      if (ok) {
        edit({ isDefault: true });
        setNotice("This is now the default provider.");
        reload();
      }
    });
  }, [draft, withDraft, edit, setNotice, reload]);

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
        setDraft(undefined);
        reload();
      }
    });
  }, [draft, close, edit, run, reload]);

  return { query, kinds, draft, runner, open, close, edit, setKind, save, test, loadModels, makeDefault, remove };
}
