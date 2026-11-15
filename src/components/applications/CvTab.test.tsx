import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";
import { useNav } from "@/stores/nav";

import * as api from "@/api/applications";
import { cv } from "@/test/fixtures/applications";
import type { Rendered } from "@/types/applications";
import { CvTab } from "./CvTab";

vi.mock("@/api/applications");
vi.mock("@/api/bridge", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  onDataChanged: () => () => undefined,
  onGraphProgress: () => () => undefined,
}));
// CodeMirror needs a layout engine; the editor is replaced by a text box that shows what it was given.
vi.mock("./CvEditor", async () => {
  const { createElement } = await import("react");
  return {
    CvEditor: ({
      doc,
      readOnly,
      onChange,
    }: {
      doc?: { text: string };
      readOnly: boolean;
      onChange: (text: string) => void;
    }) =>
      createElement(
        "div",
        null,
        createElement("textarea", {
          "aria-label": "cv.yaml",
          readOnly,
          onChange: (e: { target: { value: string } }) => onChange(e.target.value),
        }),
        createElement("output", { "aria-label": "loaded" }, doc?.text ?? ""),
      ),
  };
});

const mocked = vi.mocked(api);
const revise = vi.fn<(instruction: string, yaml: string) => Promise<void>>();

const RENDERED: Rendered = { pageCount: 1, warnings: [] };

function deferred<T>() {
  let resolve: (value: T) => void = () => undefined;
  let reject: (error: unknown) => void = () => undefined;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function show(busy = false, cvEpoch = 0) {
  return render(<CvTab id={7} cvEpoch={cvEpoch} busy={busy} revise={revise} />);
}

/** Type into the editor, then let the 800 ms of quiet pass. */
async function edit(text: string) {
  const box = await screen.findByRole("textbox", { name: "cv.yaml" });
  vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
  try {
    fireEvent.change(box, { target: { value: text } });
    act(() => {
      vi.advanceTimersByTime(800);
    });
  } finally {
    vi.useRealTimers();
  }
}

beforeEach(() => {
  revise.mockReset();
  revise.mockResolvedValue(undefined);
  Object.defineProperty(URL, "createObjectURL", {
    value: vi.fn(() => "blob:page"),
    configurable: true,
    writable: true,
  });
  Object.defineProperty(URL, "revokeObjectURL", { value: vi.fn(), configurable: true, writable: true });
  mocked.readCv.mockResolvedValue(cv());
  mocked.readCvPage.mockResolvedValue(new Blob(["png"], { type: "image/png" }));
  mocked.saveCv.mockResolvedValue(RENDERED);
});

afterEach(() => {
  vi.useRealTimers();
  cleanup();
});

describe("the CV tab", () => {
  it("shows what the posting asks for against the profile, the fact check and the pages", async () => {
    show();
    const checklist = await screen.findByRole("region", { name: "What they ask for" });
    expect(within(checklist).getByRole("img", { name: "Covered" })).toBeTruthy();
    expect(within(checklist).getByRole("img", { name: "Not in your profile" })).toBeTruthy();
    expect(within(checklist).getByText("Nice to have")).toBeTruthy();
    expect(within(checklist).getByText("Nothing in your documents shows it")).toBeTruthy();
    expect(screen.getByRole("group", { name: "Every fact traced to your profile" })).toBeTruthy();
    expect(screen.getByLabelText("loaded").textContent).toBe("name: Ada\nrole: Engineer\n");
    expect((await screen.findByRole("img", { name: "CV page 1 of 1" })).getAttribute("src")).toBe("blob:page");
    expect(mocked.readCvPage).toHaveBeenCalledWith(7, 0);
  });

  it("does not claim the facts are traced when the fact guard did not run clean", async () => {
    mocked.readCv.mockResolvedValue(cv({ guardClean: false }));
    show();
    await screen.findByRole("region", { name: "What they ask for" });
    expect(screen.queryByRole("group", { name: "Every fact traced to your profile" })).toBeNull();
  });

  it("sends the user to Profile > About you when the tailoring step reported gaps", async () => {
    const user = userEvent.setup();
    useNav.setState({ screen: "application", profileFocus: undefined });
    show();
    await user.click(await screen.findByRole("button", { name: "Add to your profile" }));
    expect(useNav.getState().screen).toBe("profile");
    expect(useNav.getState().profileFocus).toBe("About you");
  });

  it("renders a CV whose previews are missing or out of date, once", async () => {
    mocked.readCv.mockResolvedValue(cv({ stale: true, pageCount: 0 }));
    show();
    await waitFor(() => expect(mocked.saveCv).toHaveBeenCalledWith(7, "name: Ada\nrole: Engineer\n"));
    expect(mocked.saveCv).toHaveBeenCalledTimes(1);
    await waitFor(() => expect(mocked.readCvPage).toHaveBeenCalledWith(7, 0));
  });
});

describe("editing", () => {
  it("saves and renders 800 ms after the last keystroke, noting what is going on", async () => {
    const render1 = deferred<Rendered>();
    mocked.saveCv.mockReturnValue(render1.promise);
    show();
    await screen.findByRole("textbox", { name: "cv.yaml" });
    fireEvent.change(screen.getByRole("textbox", { name: "cv.yaml" }), { target: { value: "name: Grace\n" } });
    expect(screen.getByText("editing...")).toBeTruthy();
    expect(mocked.saveCv).not.toHaveBeenCalled();

    await edit("name: Grace\nrole: Admiral\n");
    await waitFor(() => expect(mocked.saveCv).toHaveBeenCalledWith(7, "name: Grace\nrole: Admiral\n"));
    expect(screen.getByText("rendering...")).toBeTruthy();
    await act(async () => render1.resolve(RENDERED));
    await waitFor(() => expect(screen.queryByText("rendering...")).toBeNull());
    expect(screen.queryByText("editing...")).toBeNull();
  });

  it("renders once more for the edits that arrive during a render", async () => {
    const first = deferred<Rendered>();
    mocked.saveCv.mockReturnValueOnce(first.promise).mockResolvedValue(RENDERED);
    show();
    await edit("a: 1\n");
    await waitFor(() => expect(mocked.saveCv).toHaveBeenCalledTimes(1));
    await edit("a: 2\n");
    await edit("a: 3\n");
    expect(mocked.saveCv).toHaveBeenCalledTimes(1);
    await act(async () => first.resolve(RENDERED));
    await waitFor(() => expect(mocked.saveCv).toHaveBeenCalledTimes(2));
    expect(mocked.saveCv).toHaveBeenLastCalledWith(7, "a: 3\n");
  });

  it("keeps an edit that is still waiting when the tab is left", async () => {
    const { unmount } = show();
    const box = await screen.findByRole("textbox", { name: "cv.yaml" });
    fireEvent.change(box, { target: { value: "a: 1\n" } });
    unmount();
    expect(mocked.saveCv).toHaveBeenCalledWith(7, "a: 1\n");
  });

  it("shows fact-guard findings as warnings and keeps the edit", async () => {
    mocked.saveCv.mockResolvedValue({ pageCount: 1, warnings: ["employer 'Initech' is not in the profile"] });
    show();
    await edit("a: 1\n");
    const warnings = await screen.findByRole("group", {
      name: "Not backed by your documents (kept, check before approving):",
    });
    expect(within(warnings).getByText("employer 'Initech' is not in the profile")).toBeTruthy();
    expect(screen.queryByRole("group", { name: "Every fact traced to your profile" })).toBeNull();
  });

  it("says when the CV does not compile, with what to do about it", async () => {
    mocked.saveCv.mockRejectedValue(
      new CommandError("typst: command not found", {
        kind: "noTypst",
        challenge: null,
        advice: "Install Typst.",
        settingsSection: null,
        retryable: false,
      }),
    );
    show();
    await edit("a: 1\n");
    const banner = await screen.findByRole("group", { name: "The CV does not compile" });
    expect(within(banner).getByText("typst: command not found")).toBeTruthy();
    expect(within(banner).getByRole("button", { name: "Check again" })).toBeTruthy();
    expect(screen.queryByRole("group", { name: "Every fact traced to your profile" })).toBeNull();
  });

  it("sends a missing profile to Profile > About you, not Settings", async () => {
    const user = userEvent.setup();
    mocked.saveCv.mockRejectedValue(
      new CommandError("no profile yet: add your CV or describe your work in Profile > About you", {
        kind: "noProfile",
        challenge: null,
        advice: "Add your CV or describe your work in Profile > About you.",
        settingsSection: "About you",
        retryable: false,
      }),
    );
    useNav.setState({ screen: "application", profileFocus: undefined });
    show();
    await edit("a: 1\n");
    await user.click(await screen.findByRole("button", { name: "Open Profile > About you" }));
    expect(useNav.getState()).toMatchObject({ screen: "profile", profileFocus: "About you" });
  });

  it("reads the CV again when something rewrote it, and replaces the text", async () => {
    const { rerender } = show();
    await waitFor(() => expect(screen.getByLabelText("loaded").textContent).toBe("name: Ada\nrole: Engineer\n"));
    mocked.readCv.mockResolvedValue(cv({ yaml: "name: Ada\nrole: Staff Engineer\n" }));
    rerender(<CvTab id={7} cvEpoch={1} busy={false} revise={revise} />);
    await waitFor(() => expect(screen.getByLabelText("loaded").textContent).toBe("name: Ada\nrole: Staff Engineer\n"));
  });
});

describe("Ask AI", () => {
  it("sends the instruction with the editor text and clears the box", async () => {
    const user = userEvent.setup();
    show();
    const box = await screen.findByRole("textbox", { name: "cv.yaml" });
    fireEvent.change(box, { target: { value: "name: Grace\n" } });
    const ask = screen.getByRole("textbox", { name: "Ask AI" });
    await user.type(ask, "shorter, stress Rust{Enter}");
    expect(revise).toHaveBeenCalledWith("shorter, stress Rust", "name: Grace\n");
    expect((ask as HTMLInputElement).value).toBe("");
    // The edit went with the instruction: nothing is left to save on its own.
    expect(screen.queryByText("editing...")).toBeNull();
  });

  it("does nothing for an empty instruction", async () => {
    const user = userEvent.setup();
    show();
    await user.type(await screen.findByRole("textbox", { name: "Ask AI" }), "   {Enter}");
    expect(revise).not.toHaveBeenCalled();
  });

  it("is locked while a job works on the CV", async () => {
    show(true);
    await screen.findByRole("textbox", { name: "cv.yaml" });
    expect(screen.getByText("locked while the AI works")).toBeTruthy();
    expect(screen.getByRole("textbox", { name: "cv.yaml" }).hasAttribute("readonly")).toBe(true);
    expect(screen.getByRole("textbox", { name: "Ask AI" }).hasAttribute("disabled")).toBe(true);
  });
});

describe("without a CV", () => {
  it("offers Generate and keeps the editor and Ask AI away", async () => {
    mocked.readCv.mockResolvedValue(
      cv({ hasCv: false, yaml: null, pageCount: 0, requirements: [], hasGaps: false, guardClean: false }),
    );
    show();
    expect(await screen.findByText("No CV yet: Generate creates one from the profile.")).toBeTruthy();
    expect(screen.queryByRole("textbox", { name: "cv.yaml" })).toBeNull();
    expect(screen.getByRole("textbox", { name: "Ask AI" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByText("No preview yet.")).toBeTruthy();
  });

  it("says the CV is being written while a job runs", async () => {
    mocked.readCv.mockResolvedValue(
      cv({ hasCv: false, yaml: null, pageCount: 0, requirements: [], hasGaps: false, guardClean: false }),
    );
    show(true);
    expect(await screen.findByText("The CV is being written...")).toBeTruthy();
  });
});
