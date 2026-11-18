// The use-cases of Settings > AI providers: the list, the dialog that adds or edits one (save, test,
// delete), and making one the default. The models of the provider are listed on their own (see
// `useModels`), so the model box can suggest them. A key typed here lives in this
// component's state until the shell has taken it to the keyring, and is never read back.

import { useCallback, useEffect, useMemo, useRef, useState } from "react";

import { useAction, useQuery, type Action, type Query } from "@/hooks/useAsync";

import * as api from "@/api/providers";
import {
  accountLine,
  draftOf,
  inputOf,
  modelsSignature,
  probeInputOf,
  switchKind,
  type Draft,
} from "@/lib/integrations/providers";
import { toast } from "@/stores/toast";
import type { KindDefaults, Provider, ProviderKind, Providers } from "@/types/integrations";
import { useRunner, type Runner } from "./useRunner";

export interface ProvidersCard {
  query: Query<Providers>;
  kinds: KindDefaults[];
  /** The provider in the dialog; `undefined` while it is closed. */
  draft: Draft | undefined;
  /** The models of the provider in the dialog are being listed. */
  modelsLoading: boolean;
  /** List the models again if the address, key or kind changed (on leaving their box). */
  listModels: () => void;
  runner: Runner;
  open: (provider: Provider | undefined) => void;
  close: () => void;
  edit: (patch: Partial<Draft>) => void;
  setKind: (kind: ProviderKind) => void;
  save: () => void;
  test: () => void;
  /** Two steps: the first click arms the button, the second deletes. */
  remove: () => void;
  /** Codex: sign in with ChatGPT in the system browser (the form is saved first). */
  signIn: () => void;
  /** Codex: abort a sign-in that waits for the browser. */
  cancelSignIn: () => void;
  /** Codex: use the Codex CLI's login (the form is saved first). */
  importCli: () => void;
  /** Codex: a Codex CLI login is there to reuse (read when the Codex kind is in the dialog). */
  cliAvailable: boolean;
  /** Make a listed provider the one generation uses. */
  makeDefault: Action<[Provider], boolean>;
}

/** The models of the provider in the dialog. They are listed when the dialog opens, when the kind
 *  changes and when the user leaves the address or key box (`list`), never while typing, so nothing
 *  goes to a half-typed address; and only when what they depend on changed. A failure only leaves
 *  the list empty: the model id can always be typed. */
function useModels(
  draft: Draft | undefined,
  setModels: (models: string[]) => void,
): { loading: boolean; list: () => void } {
  const [loading, setLoading] = useState(false);
  const [asked, setAsked] = useState(0);
  const reads = useRef(0);
  const listed = useRef<string | undefined>(undefined);
  const latest = useRef(draft);
  latest.current = draft;
  const open = draft !== undefined;

  // biome-ignore lint/correctness/useExhaustiveDependencies: `asked` is the request to list again (the form is read through `latest`)
  useEffect(() => {
    const form = latest.current;
    if (!open || !form) {
      reads.current += 1;
      listed.current = undefined;
      setLoading(false);
      return;
    }
    const input = probeInputOf(form);
    const signature = modelsSignature(input);
    if (input.baseUrl === "" || signature === listed.current) {
      return;
    }
    listed.current = signature;
    const read = ++reads.current;
    setLoading(true);
    api
      .listModels(input)
      .then(
        (models) => read === reads.current && setModels(models),
        () => read === reads.current && setModels([]),
      )
      .finally(() => read === reads.current && setLoading(false));
  }, [open, asked, setModels]);

  // Answers after the dialog is gone are dropped.
  useEffect(
    () => () => {
      reads.current += 1;
    },
    [],
  );
  const list = useCallback(() => setAsked((n) => n + 1), []);
  return { loading, list };
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
  const { loading: modelsLoading, list: listModels } = useModels(draft, setModels);

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

  const signingIn = runner.busy === "signing-in";
  const close = useCallback(() => {
    if (signingIn) {
      // A sign-in that waits for the browser ends with the dialog.
      api.codexCancelSignIn().catch(() => undefined);
    }
    reset();
    setDraft(undefined);
  }, [reset, signingIn]);

  const setKind = useCallback(
    (kind: ProviderKind) => {
      reset();
      setDraft((d) => d && switchKind(d, kind, kinds));
      listModels();
    },
    [kinds, reset, listModels],
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
    void withDraft("testing", "The test failed", (form) => api.testProvider(probeInputOf(form)).then(() => true)).then(
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

  /** Run a Codex sign-in for the provider in the dialog. The form is saved first, always (an edited
   *  kind or name takes effect, and a new provider gets its id); the dialog stays open on it. */
  const codex = useCallback(
    (what: string, title: string, call: (id: number) => Promise<string>) => {
      void withDraft(what, title, async (form) => {
        const saved = await api.saveProvider(inputOf(form));
        const id = saved.id;
        setDraft(
          (d) =>
            d && {
              ...d,
              id,
              savedKind: saved.kind,
              savedBaseUrl: saved.baseUrl,
              isDefault: saved.isDefault,
              account: saved.account,
              hasKey: saved.account !== null,
              apiKey: "",
            },
        );
        reload();
        const account = await call(id);
        setDraft((d) => (d && d.id === id ? { ...d, account, hasKey: true } : d));
        return account;
      }).then((account) => {
        if (account) {
          toast.ok("Signed in with ChatGPT", accountLine(account));
          reload();
        }
      });
    },
    [withDraft, reload],
  );

  const signIn = useCallback(() => codex("signing-in", "Could not sign in", api.codexSignIn), [codex]);
  const importCli = useCallback(
    () => codex("importing", "Could not use the Codex CLI login", api.codexImportCli),
    [codex],
  );
  const cancelSignIn = useCallback(() => {
    api.codexCancelSignIn().catch(() => undefined);
  }, []);

  // Whether a Codex CLI login can be reused: read when the Codex kind is in the dialog.
  const [cliAvailable, setCliAvailable] = useState(false);
  const codexOpen = draft?.kind === "codex";
  useEffect(() => {
    if (!codexOpen) {
      return;
    }
    let live = true;
    api.codexCliAvailable().then(
      (available) => live && setCliAvailable(available),
      () => live && setCliAvailable(false),
    );
    return () => {
      live = false;
    };
  }, [codexOpen]);

  const makeDefault = useAction(async (provider: Provider) => {
    await api.makeDefault(provider.id);
    toast.ok("Default provider", `${provider.name} is used for generation now.`);
    reload();
    return true;
  });

  return {
    query,
    kinds,
    draft,
    modelsLoading,
    listModels,
    runner,
    open,
    close,
    edit,
    setKind,
    save,
    test,
    remove,
    signIn,
    cancelSignIn,
    importCli,
    cliAvailable,
    makeDefault,
  };
}
