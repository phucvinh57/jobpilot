// The use-cases of Settings > About you: list the documents, add a file or a note, remove one, and
// read them into the profile.
//
// Every answer is the whole section as the shell has it after the call. Calls never wait for each
// other, so an answer is dropped when a newer call has started (`seq`): the newer answer is the
// newer state. After an add or a removal the documents are read again on their own (one model
// call); while a read runs the status line says so and Re-read is off.

import { useCallback, useEffect, useRef, useState } from "react";

import { CommandError, toCommandError } from "@/shared";

import * as api from "../api/profileApi";
import { noteProblem, type About } from "../domain/about";

/** A failed call and the banner title it is shown under. */
export interface AboutFailure {
  error: CommandError;
  title: string;
}

export interface AboutCard {
  /** `undefined` until the first read answered. */
  about: About | undefined;
  /** The first read failed. */
  loadError: CommandError | undefined;
  retry: () => void;
  /** An add, a removal or a read failed. */
  failure: AboutFailure | undefined;
  clearFailure: () => void;
  /** The documents are being read into the profile. */
  reading: boolean;
  /** A file is being added (picked and extracted). */
  adding: boolean;
  /** Ids of the documents being removed. */
  removing: ReadonlySet<number>;
  addFile: () => void;
  remove: (id: number) => void;
  reread: () => void;

  /** The "Add text" form. */
  noteOpen: boolean;
  setNoteOpen: (open: boolean) => void;
  noteTitle: string;
  setNoteTitle: (title: string) => void;
  noteBody: string;
  setNoteBody: (body: string) => void;
  /** Why the note cannot be saved yet (`null`: it can). */
  noteBlocked: string | null;
  savingNote: boolean;
  saveNote: () => void;
}

export function useAbout(): AboutCard {
  const [about, setAbout] = useState<About>();
  const [loadError, setLoadError] = useState<CommandError>();
  const [failure, setFailure] = useState<AboutFailure>();
  const [reading, setReading] = useState(false);
  const [adding, setAdding] = useState(false);
  const [removing, setRemoving] = useState<ReadonlySet<number>>(new Set());
  const [noteOpen, setNoteOpen] = useState(false);
  const [noteTitle, setNoteTitle] = useState("");
  const [noteBody, setNoteBody] = useState("");
  const [savingNote, setSavingNote] = useState(false);

  /** Calls started so far: only the latest one's answer is shown. */
  const seq = useRef(0);
  /** Reads started so far: only the latest one ends "reading". */
  const readSeq = useRef(0);
  const alive = useRef(true);

  const show = useCallback((next: About, mine: number) => {
    if (alive.current && mine === seq.current) {
      setAbout(next);
    }
  }, []);

  const fail = useCallback((thrown: unknown, title: string) => {
    if (alive.current) {
      setFailure({ error: toCommandError(thrown), title });
    }
  }, []);

  const load = useCallback(() => {
    const mine = seq.current;
    setLoadError(undefined);
    api.readAbout().then(
      (next) => show(next, mine),
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

  /** Read the documents into the profile. */
  const read = useCallback(async () => {
    const mine = ++seq.current;
    const myRead = ++readSeq.current;
    setReading(true);
    setFailure(undefined);
    try {
      show(await api.readDocuments(), mine);
    } catch (thrown) {
      if (myRead === readSeq.current) {
        fail(thrown, "Your documents were not read");
      }
    } finally {
      if (alive.current && myRead === readSeq.current) {
        setReading(false);
      }
    }
  }, [show, fail]);

  /** Run a change of the documents; read them again when the answer says the profile is behind. */
  const change = useCallback(
    async (call: () => Promise<About>, title: string): Promise<boolean> => {
      const mine = ++seq.current;
      setFailure(undefined);
      let next: About;
      try {
        next = await call();
      } catch (thrown) {
        fail(thrown, title);
        return false;
      }
      show(next, mine);
      if (alive.current && mine === seq.current && (next.status === "unread" || next.status === "stale")) {
        void read();
      }
      return true;
    },
    [show, fail, read],
  );

  const addFile = useCallback(() => {
    if (adding) {
      return;
    }
    void (async () => {
      let path: string | null;
      try {
        path = await api.pickDocument();
      } catch (thrown) {
        fail(new CommandError(`The file dialog did not open: ${toCommandError(thrown).message}`), "Not added");
        return;
      }
      if (path === null) {
        return;
      }
      setAdding(true);
      try {
        await change(() => api.addFile(path), "Not added");
      } finally {
        if (alive.current) {
          setAdding(false);
        }
      }
    })();
  }, [adding, change, fail]);

  const remove = useCallback(
    (id: number) => {
      if (removing.has(id)) {
        return;
      }
      setRemoving((ids) => new Set(ids).add(id));
      void change(() => api.removeSource(id), "Not removed").finally(() => {
        if (alive.current) {
          setRemoving((ids) => {
            const next = new Set(ids);
            next.delete(id);
            return next;
          });
        }
      });
    },
    [removing, change],
  );

  const noteBlocked = about === undefined ? "Loading…" : noteProblem(noteBody, about);

  const saveNote = useCallback(() => {
    if (savingNote || noteBlocked !== null) {
      return;
    }
    setSavingNote(true);
    void change(() => api.addText(noteTitle, noteBody), "Not added")
      .then((saved) => {
        if (saved && alive.current) {
          setNoteTitle("");
          setNoteBody("");
          setNoteOpen(false);
        }
      })
      .finally(() => {
        if (alive.current) {
          setSavingNote(false);
        }
      });
  }, [savingNote, noteBlocked, change, noteTitle, noteBody]);

  const reread = useCallback(() => {
    if (!reading) {
      void read();
    }
  }, [reading, read]);

  return {
    about,
    loadError,
    retry: load,
    failure,
    clearFailure: () => setFailure(undefined),
    reading,
    adding,
    removing,
    addFile,
    remove,
    reread,
    noteOpen,
    setNoteOpen,
    noteTitle,
    setNoteTitle,
    noteBody,
    setNoteBody,
    noteBlocked,
    savingNote,
    saveNote,
  };
}
