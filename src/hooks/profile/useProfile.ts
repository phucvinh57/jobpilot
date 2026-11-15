// The use-cases of Profile > Preferences.
//
// Every change is saved at once through the shell (which edits the whole document under the
// core's one lock), and the answer is the document as it is after that edit: that is what the card
// shows. Edits never wait for each other, so an answer is dropped when a newer edit has started
// (`seq`): the newer answer is the newer document. The model reads the notes in the background
// after they are saved; its outcome is a muted line, never a blocker.

import { useCallback, useEffect, useRef, useState } from "react";

import { CommandError, toCommandError } from "@/lib/problem";

import * as api from "@/api/profile";
import { changed, cleanNotes, followNotes, NOTES_IDLE, savePlan, type NotesStatus } from "@/lib/profile";
import type { FieldKey, JobType, Profile } from "@/types/profile";

export interface ProfileCard {
  /** `undefined` until the first read answered. */
  profile: Profile | undefined;
  /** The first read failed. */
  loadError: CommandError | undefined;
  /** A change failed or was refused (the card shows it). */
  error: CommandError | undefined;
  clearError: () => void;
  retry: () => void;
  /** The label of the last save and a counter that restarts its fade. */
  flash: { n: number; label: string } | undefined;
  /** Undo is on: there is a change to undo and no save is running. */
  canUndo: boolean;
  notesText: string;
  setNotesText: (text: string) => void;
  notesStatus: NotesStatus;
  /** Save is on: the notes differ from the saved ones, or the last read of them failed. */
  canSaveNotes: boolean;
  addItem: (field: FieldKey, text: string) => void;
  removeItem: (field: FieldKey, item: string) => void;
  setJobType: (jobType: JobType) => void;
  saveNotes: () => void;
  undo: () => void;
}

export function useProfile(): ProfileCard {
  const [profile, setProfile] = useState<Profile>();
  const [loadError, setLoadError] = useState<CommandError>();
  const [error, setError] = useState<CommandError>();
  const [flash, setFlash] = useState<{ n: number; label: string }>();
  const [saving, setSaving] = useState(0);
  const [notesText, setNotesTextState] = useState("");
  const [notesStatus, setNotesStatusState] = useState<NotesStatus>(NOTES_IDLE);

  /** What is shown, for the "did it change" test and the stale checks. */
  const shown = useRef<Profile | undefined>(undefined);
  /** Edits started so far. */
  const seq = useRef(0);
  /** Readings of the notes started so far: only the latest one sets the line under the notes. */
  const notesSeq = useRef(0);
  /** What the notes box last got from the store (loaded or saved). */
  const synced = useRef("");
  const typed = useRef("");
  const status = useRef<NotesStatus>(NOTES_IDLE);
  const alive = useRef(true);

  const setNotesText = useCallback((text: string) => {
    typed.current = text;
    setNotesTextState(text);
  }, []);

  const setStatus = useCallback((next: NotesStatus) => {
    status.current = next;
    setNotesStatusState(next);
  }, []);

  /** Show `next`, unless `latest` is false (a later edit has started: it will bring a newer
   *  document). `label` is the "Saved" label of a change that was really made, even when it is not shown. */
  const show = useCallback(
    (next: Profile, label: string | undefined, latest: boolean) => {
      if (!alive.current) {
        return;
      }
      if (label !== undefined && changed(shown.current, next)) {
        setFlash((f) => ({ n: (f?.n ?? 0) + 1, label }));
      }
      if (!latest) {
        return;
      }
      // The notes box follows the stored notes unless the user has typed since it was last synced.
      const follow = followNotes(typed.current, synced.current, next.notes, next.limits.maxNotesChars);
      if (follow !== undefined) {
        setNotesText(follow);
      }
      synced.current = next.notes;
      shown.current = next;
      setProfile(next);
    },
    [setNotesText],
  );

  const fail = useCallback((thrown: unknown) => {
    if (alive.current) {
      setError(toCommandError(thrown));
    }
  }, []);

  const load = useCallback(() => {
    const mine = seq.current;
    setLoadError(undefined);
    api.readProfile().then(
      (next) => show(next, undefined, mine === seq.current),
      (thrown: unknown) => {
        if (alive.current) {
          setLoadError(toCommandError(thrown));
        }
      },
    );
  }, [show]);

  useEffect(() => {
    alive.current = true;
    load();
    return () => {
      alive.current = false;
    };
  }, [load]);

  /** Run a change; its answer is the card. `label` is the "Saved" label. */
  const edit = useCallback(
    async <T>(
      call: () => Promise<T>,
      toProfile: (result: T) => Profile,
      label: string | undefined,
    ): Promise<T | undefined> => {
      const mine = ++seq.current;
      if (label !== undefined) {
        setSaving((n) => n + 1);
      }
      try {
        const result = await call();
        if (alive.current) {
          setError(undefined);
        }
        show(toProfile(result), label, mine === seq.current);
        return result;
      } catch (thrown) {
        fail(thrown);
        return undefined;
      } finally {
        if (label !== undefined && alive.current) {
          setSaving((n) => n - 1);
        }
      }
    },
    [show, fail],
  );

  const addItem = useCallback(
    (field: FieldKey, text: string) => {
      void edit(
        () => api.addItem(field, text),
        (r) => r.profile,
        "Saved",
      ).then((result) => {
        // A topic in a list the form does not show (nice or avoided skills), a full list: say so.
        if (result?.message && alive.current) {
          setError(new CommandError(result.message));
        }
      });
    },
    [edit],
  );

  const removeItem = useCallback(
    (field: FieldKey, item: string) =>
      void edit(
        () => api.removeItem(field, item),
        (p) => p,
        "Saved",
      ),
    [edit],
  );

  const setJobType = useCallback(
    (jobType: JobType) =>
      void edit(
        () => api.setJobType(jobType),
        (p) => p,
        "Saved",
      ),
    [edit],
  );

  /** The model reads `text` once, so it can fill what the form hides. Reads and edits queue in the shell. */
  const readNotes = useCallback(
    async (text: string) => {
      const mine = ++notesSeq.current;
      const editSeq = seq.current;
      setStatus({ kind: "reading" });
      try {
        const result = await api.readNotes(text);
        if (!alive.current) {
          return;
        }
        if (mine === notesSeq.current) {
          setStatus(result.line === null ? NOTES_IDLE : { kind: "failed", line: result.line });
        }
        // What the model merged shows, unless an edit started meanwhile (its answer comes later).
        show(result.profile, undefined, editSeq === seq.current);
      } catch (thrown) {
        if (mine === notesSeq.current && alive.current) {
          setStatus(NOTES_IDLE);
        }
        fail(thrown);
      }
    },
    [setStatus, show, fail],
  );

  const saveNotes = useCallback(() => {
    const current = shown.current;
    if (!current) {
      return;
    }
    const text = cleanNotes(typed.current, current.limits.maxNotesChars);
    // Ctrl/Cmd+Enter also inserted a line break: drop it.
    if (typed.current !== text) {
      setNotesText(text);
    }
    const plan = savePlan(text, current.notes, status.current.kind === "failed");
    if (plan === "nothing") {
      return;
    }
    setError(undefined);
    void (async () => {
      if (plan === "saveAndRead") {
        synced.current = text;
        if (
          (await edit(
            () => api.saveNotes(text),
            (p) => p,
            "Saved",
          )) === undefined
        ) {
          return;
        }
      }
      if (text === "") {
        notesSeq.current += 1;
        setStatus(NOTES_IDLE);
      } else {
        await readNotes(text);
      }
    })();
  }, [edit, readNotes, setNotesText, setStatus]);

  const undo = useCallback(() => {
    // The notes box follows the restored notes.
    synced.current = typed.current;
    // A reading that is still going no longer says anything about what is shown.
    notesSeq.current += 1;
    setStatus(NOTES_IDLE);
    void edit(
      () => api.undo(),
      (p) => p,
      "Undone",
    );
  }, [edit, setStatus]);

  const canSaveNotes =
    profile !== undefined &&
    savePlan(cleanNotes(notesText, profile.limits.maxNotesChars), profile.notes, notesStatus.kind === "failed") !==
      "nothing";

  return {
    profile,
    loadError,
    error,
    clearError: () => setError(undefined),
    retry: load,
    flash,
    canUndo: (profile?.canUndo ?? false) && saving === 0,
    notesText,
    setNotesText,
    notesStatus,
    canSaveNotes,
    addItem,
    removeItem,
    setJobType,
    saveNotes,
    undo,
  };
}
