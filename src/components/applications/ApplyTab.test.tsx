import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";

import * as api from "@/api/applications";
import { idleApply, primary, reviewApply, row } from "@/test/fixtures/applications";
import type { Apply, ApplyProgress, Primary } from "@/types/applications";
import { useApplySession } from "@/hooks/applications/useApplySession";
import { ApplyTab } from "./ApplyTab";

vi.mock("@/api/applications");
const changed = vi.hoisted(() => new Set<(scope: string) => void>());

vi.mock("@/api/bridge", () => ({
  invoke: vi.fn().mockResolvedValue(undefined),
  onDataChanged: (handler: (scope: string) => void) => {
    changed.add(handler);
    return () => changed.delete(handler);
  },
  onGraphProgress: () => () => undefined,
}));

const mocked = vi.mocked(api);
let onPrimary: (p: Primary | null) => void;
let applyProgress: ((p: ApplyProgress) => void) | undefined;

function Harness() {
  const session = useApplySession(7, onPrimary);
  return <ApplyTab id={7} session={session} />;
}

function show(apply: Apply) {
  mocked.readApply.mockResolvedValue(apply);
  render(<Harness />);
}

const ROWS = [
  row({ id: "name", label: "Full name", value: "Ada Lovelace" }),
  row({
    id: "pitch",
    label: "Why us?",
    input: "area",
    kind: "textarea",
    value: "",
    required: true,
    tag: { label: "needs input", tone: "danger" },
    flagged: true,
  }),
  row({
    id: "remote",
    label: "Remote?",
    input: "chips",
    kind: "radio",
    options: ["Yes", "No"],
    value: "",
    required: false,
  }),
  row({
    id: "tools",
    label: "Tools",
    input: "multiChips",
    kind: "checkbox_group",
    options: ["Rust", "Go"],
    value: "Rust",
    required: false,
  }),
  row({
    id: "terms",
    label: "Terms",
    input: "checkbox",
    kind: "checkbox",
    value: "no",
    required: false,
    rememberable: false,
  }),
  row({
    id: "cv",
    label: "Resume",
    input: "readonly",
    kind: "file",
    value: "/tmp/cv.pdf",
    shown: "cv.pdf",
    required: true,
    rememberable: false,
  }),
  row({
    id: "phone",
    label: "Phone",
    input: "readonly",
    editable: false,
    step: 1,
    value: "",
    shown: "",
    required: true,
    rememberable: false,
  }),
];

beforeEach(() => {
  onPrimary = vi.fn();
  changed.clear();
  applyProgress = undefined;
  mocked.onApplyProgress.mockImplementation((handler) => {
    applyProgress = handler;
    return () => undefined;
  });
  mocked.syncApply.mockResolvedValue(
    primary({ action: "submit", label: "Submit", enabled: false, hint: "1 required field(s) still empty" }),
  );
  mocked.setDryRun.mockResolvedValue(undefined);
  mocked.saveApplyUrl.mockResolvedValue(undefined);
  mocked.discardApply.mockResolvedValue(undefined);
  mocked.openVisible.mockResolvedValue(undefined);
  mocked.markApply.mockResolvedValue(undefined);
  mocked.prepareApply.mockResolvedValue(undefined);
  mocked.readShot.mockResolvedValue(new Blob(["png"], { type: "image/png" }));
  Object.defineProperty(URL, "createObjectURL", {
    value: vi.fn(() => "blob:shot"),
    configurable: true,
    writable: true,
  });
  Object.defineProperty(URL, "revokeObjectURL", { value: vi.fn(), configurable: true, writable: true });
});

afterEach(() => {
  cleanup();
});

describe("before a form is open", () => {
  it("says where the application goes, how the browser is set up, and that nothing is sent before Submit", async () => {
    show(idleApply());
    expect(await screen.findByRole("region", { name: "Greenhouse" })).toBeTruthy();
    expect(screen.getByText("https://boards.example.com/acme/1")).toBeTruthy();
    expect(screen.getByText("found in the job description")).toBeTruthy();
    expect(screen.getByText("No login needed: runs in a throwaway browser profile.")).toBeTruthy();
    expect(screen.getByText(/nothing is sent until you press Submit/)).toBeTruthy();
  });

  it("switches the dry run on", async () => {
    const user = userEvent.setup();
    show(idleApply());
    await user.click(await screen.findByRole("checkbox", { name: "Dry run: fill the form but never submit" }));
    expect(mocked.setDryRun).toHaveBeenCalledWith(true);
  });

  it("keeps the dry run on when the environment forces it", async () => {
    const base = idleApply();
    show({ ...base, idle: base.idle && { ...base.idle, dryRun: true, dryRunForced: true } });
    const box = await screen.findByRole("checkbox", { name: "Dry run: fill the form but never submit" });
    expect((box as HTMLInputElement).checked).toBe(true);
    expect(box.hasAttribute("disabled")).toBe(true);
    expect(screen.getByText("JOBPILOT_DRY_RUN is set: submitting is disabled.")).toBeTruthy();
  });

  it("asks for the apply URL when none was found, and saves it", async () => {
    const user = userEvent.setup();
    const base = idleApply();
    show({ ...base, idle: base.idle && { ...base.idle, applyUrl: null, urlNote: null } });
    expect(await screen.findByText("No apply URL found in the job description.")).toBeTruthy();
    await user.type(screen.getByRole("textbox", { name: "Apply URL" }), "https://jobs.example.com/apply");
    await user.click(screen.getByRole("button", { name: "Save" }));
    expect(mocked.saveApplyUrl).toHaveBeenCalledWith(7, "https://jobs.example.com/apply");
  });

  it("offers no dry run for an application that was sent, and shows the confirmation", async () => {
    const base = idleApply();
    show({ ...base, idle: base.idle && { ...base.idle, applied: true, hasProof: true } });
    expect(await screen.findByText("This application was sent. Replies are tracked from your email.")).toBeTruthy();
    expect(screen.queryByRole("checkbox", { name: /Dry run/ })).toBeNull();
    expect(await screen.findByRole("img", { name: "Confirmation" })).toBeTruthy();
    expect(mocked.readShot).toHaveBeenCalledWith(7, "proof", 0);
  });

  it("shows a failure that cannot be fixed by trying again without a Retry button", async () => {
    show(idleApply({ error: new CommandError("could not start Chrome"), canRetry: true }));
    expect(await screen.findByText("could not start Chrome")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Retry" })).toBeNull();
  });

  it("offers Retry for a retryable failure", async () => {
    const user = userEvent.setup();
    const network = new CommandError("connection refused", {
      kind: "network",
      challenge: null,
      advice: "The connection failed.",
      settingsSection: null,
      retryable: true,
    });
    show(idleApply({ error: network, canRetry: true }));
    await user.click(await screen.findByRole("button", { name: "Retry" }));
    expect(mocked.prepareApply).toHaveBeenCalledWith(7);
  });
});

describe("while the form is being filled", () => {
  it("shows the steps so far, with the latest one marked", async () => {
    show(
      idleApply({ phase: "preparing", working: { title: "Preparing the application", lines: ["Opening the page"] } }),
    );
    expect(await screen.findByText("Preparing the application")).toBeTruthy();
    expect(screen.getByText("Opening the page")).toBeTruthy();
    act(() => applyProgress?.({ appId: 7, lines: ["Opening the page", "Reading the fields"] }));
    const lines = within(screen.getByRole("list", { name: "Progress" })).getAllByRole("listitem");
    expect(lines.map((l) => l.textContent)).toEqual(["Opening the page", "›Reading the fields"]);
    expect(screen.getByText(/can take up to a minute/)).toBeTruthy();
  });
});

describe("the review", () => {
  it("shows every field with where its answer came from, and highlights what needs a look", async () => {
    show(reviewApply(ROWS));
    expect(await screen.findByRole("region", { name: "Greenhouse - boards.example.com" })).toBeTruthy();
    expect((screen.getByRole("textbox", { name: "Full name" }) as HTMLInputElement).value).toBe("Ada Lovelace");
    expect(screen.getByText("Why us? *")).toBeTruthy();
    expect(screen.getByText("needs input")).toBeTruthy();
    expect(document.getElementById("row-pitch")?.className).toContain("bg-warn-bg");
    expect(document.getElementById("row-name")?.className).not.toContain("bg-warn-bg");
    // A file is shown as its name; a field of a later step is read-only and tagged with its step.
    expect(screen.getByText("cv.pdf", { selector: "p" })).toBeTruthy();
    expect(screen.getByText("(empty)")).toBeTruthy();
    expect(screen.getByText("step 2")).toBeTruthy();
    expect(screen.getByText("attached")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Discard" })).toBeTruthy();
  });

  it("tells the shell what was typed (latest wins) and shows the primary action it answers with", async () => {
    const user = userEvent.setup();
    show(reviewApply(ROWS));
    await user.type(await screen.findByRole("textbox", { name: "Why us?" }), "Rust");
    await waitFor(() => {
      const calls = mocked.syncApply.mock.calls;
      expect(calls[calls.length - 1]?.[1].values.pitch).toBe("Rust");
    });
    const calls = mocked.syncApply.mock.calls;
    const last = calls[calls.length - 1];
    expect(last?.[0]).toBe(7);
    // The editable rows go over; a row of a later step does not.
    expect(Object.keys(last?.[1].values ?? {}).sort()).toEqual(["cv", "name", "pitch", "remote", "terms", "tools"]);
    expect(onPrimary).toHaveBeenLastCalledWith(expect.objectContaining({ action: "submit", enabled: false }));
  });

  it("highlights a required answer that is empty, until it is typed", async () => {
    const user = userEvent.setup();
    show(reviewApply([row({ id: "city", label: "City", value: "", required: true, flagged: false })]));
    const input = await screen.findByRole("textbox", { name: "City" });
    expect(document.getElementById("row-city")?.className).toContain("bg-warn-bg");
    await user.type(input, "Berlin");
    expect(document.getElementById("row-city")?.className).not.toContain("bg-warn-bg");
  });

  it("keeps the highlight of a row the run flagged, whatever is typed", async () => {
    const user = userEvent.setup();
    show(reviewApply([row({ id: "pitch", label: "Why us?", value: "", flagged: true, warning: "Check this answer" })]));
    await user.type(await screen.findByRole("textbox", { name: "Why us?" }), "x");
    expect(document.getElementById("row-pitch")?.className).toContain("bg-warn-bg");
    expect(screen.getByText("Check this answer")).toBeTruthy();
  });

  it("toggles choices: one of a few, several of a few, and a yes box", async () => {
    const user = userEvent.setup();
    show(reviewApply(ROWS));
    const remote = await screen.findByRole("group", { name: "Remote?" });
    await user.click(within(remote).getByRole("button", { name: "Yes" }));
    expect(within(remote).getByRole("button", { name: "Yes" }).getAttribute("aria-pressed")).toBe("true");
    await user.click(within(remote).getByRole("button", { name: "No" }));
    expect(within(remote).getByRole("button", { name: "Yes" }).getAttribute("aria-pressed")).toBe("false");
    await user.click(within(remote).getByRole("button", { name: "No" }));
    expect(within(remote).getByRole("button", { name: "No" }).getAttribute("aria-pressed")).toBe("false");

    const tools = screen.getByRole("group", { name: "Tools" });
    await user.click(within(tools).getByRole("button", { name: "Go" }));
    expect(within(tools).getByRole("button", { name: "Rust" }).getAttribute("aria-pressed")).toBe("true");
    expect(within(tools).getByRole("button", { name: "Go" }).getAttribute("aria-pressed")).toBe("true");
    await user.click(within(tools).getByRole("button", { name: "Rust" }));
    expect(within(tools).getByRole("button", { name: "Rust" }).getAttribute("aria-pressed")).toBe("false");

    const terms = screen.getByRole("checkbox", { name: "Yes" });
    await user.click(terms);
    expect((terms as HTMLInputElement).checked).toBe(true);
    await waitFor(() => {
      const calls = mocked.syncApply.mock.calls;
      expect(calls[calls.length - 1]?.[1].values.terms).toBe("yes");
    });
  });

  it("offers to remember an answer in the profile only when it can be saved and is not empty", async () => {
    const user = userEvent.setup();
    show(reviewApply(ROWS));
    await screen.findByRole("textbox", { name: "Full name" });
    // `name` can be remembered; the empty `pitch` and the unrememberable `terms` offer nothing.
    expect(screen.getAllByRole("checkbox", { name: "Remember this answer" })).toHaveLength(2);
    await user.click(document.getElementById("remember-name") as HTMLElement);
    await waitFor(() => {
      const calls = mocked.syncApply.mock.calls;
      expect(calls[calls.length - 1]?.[1].remember).toEqual(["name"]);
    });
  });

  it("shows the notice and the notes of the shell, and discards the review on request", async () => {
    const user = userEvent.setup();
    const base = reviewApply(ROWS);
    mocked.discardApply.mockResolvedValue(undefined);
    show({
      ...base,
      review: base.review && {
        ...base.review,
        closeLabel: "Close dry run",
        notice: {
          tone: "warn",
          title: "Required fields empty",
          body: "1 required field(s) still empty: fill them to enable the Submit button.",
        },
        notes: [
          {
            text: "Typst is not installed",
            problem: {
              kind: "noTypst",
              challenge: null,
              advice: "Install typst.",
              settingsSection: null,
              retryable: false,
            },
          },
        ],
      },
    });
    expect(await screen.findByRole("group", { name: "Required fields empty" })).toBeTruthy();
    expect(screen.getByText("Typst is not installed")).toBeTruthy();
    expect(screen.getByText("Install typst.")).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Close dry run" }));
    expect(mocked.discardApply).toHaveBeenCalledWith(7);
  });

  it("starts from a new review when a new one arrives, and keeps what was typed until then", async () => {
    const user = userEvent.setup();
    show(reviewApply(ROWS));
    const input = await screen.findByRole("textbox", { name: "Full name" });
    await user.clear(input);
    await user.type(input, "Grace");

    // The same review read again (a progress step, a sync): the typed answer stays.
    act(() => changed.forEach((handler) => handler("applications")));
    await waitFor(() => expect(mocked.readApply.mock.calls.length).toBeGreaterThan(1));
    expect((screen.getByRole("textbox", { name: "Full name" }) as HTMLInputElement).value).toBe("Grace");

    // A new review (a new generation) starts the editors again from the filled form.
    mocked.readApply.mockResolvedValue(reviewApply([row({ id: "name", value: "Ada Lovelace" })], { generation: 2 }));
    act(() => changed.forEach((handler) => handler("applications")));
    await waitFor(() =>
      expect((screen.getByRole("textbox", { name: "Full name" }) as HTMLInputElement).value).toBe("Ada Lovelace"),
    );
  });
});

describe("the end of a run", () => {
  it("confirms a submitted application and shows the proof", async () => {
    show({
      ...idleApply(),
      phase: "done",
      idle: null,
      generation: 3,
      done: { body: "boards.example.com confirmed the application for Engineer.", hasProof: true },
    });
    expect(await screen.findByRole("group", { name: "Application submitted" })).toBeTruthy();
    expect(screen.getByText("boards.example.com confirmed the application for Engineer.")).toBeTruthy();
    expect(await screen.findByRole("img", { name: "Proof" })).toBeTruthy();
    expect(mocked.readShot).toHaveBeenCalledWith(7, "done", 0);
  });

  it("hands a form that stopped over to the user, who finishes in a visible browser and marks the result", async () => {
    const user = userEvent.setup();
    show({
      ...idleApply(),
      phase: "handoff",
      idle: null,
      handoff: {
        title: "A CAPTCHA is in the way",
        reason: "The page asked for a CAPTCHA.",
        captcha: true,
        headed: false,
        hasScreenshot: true,
      },
    });
    expect(await screen.findByRole("group", { name: "A CAPTCHA is in the way" })).toBeTruthy();
    expect(screen.getByText(/Open a visible browser on the same profile/)).toBeTruthy();
    expect(await screen.findByRole("img", { name: "What the page looked like" })).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Open visible browser" }));
    expect(mocked.openVisible).toHaveBeenCalledWith(7);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Mark as applied" }).hasAttribute("disabled")).toBe(false),
    );
    await user.click(screen.getByRole("button", { name: "Mark as applied" }));
    expect(mocked.markApply).toHaveBeenCalledWith(7, true);
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Mark failed" }).hasAttribute("disabled")).toBe(false),
    );
    await user.click(screen.getByRole("button", { name: "Mark failed" }));
    expect(mocked.markApply).toHaveBeenLastCalledWith(7, false);
  });

  it("disables the handoff buttons while a short action runs, and says which", async () => {
    show({
      ...idleApply(),
      phase: "handoff",
      idle: null,
      busy: "Saving...",
      handoff: {
        title: "The form needs you",
        reason: "A field could not be filled.",
        captcha: false,
        headed: true,
        hasScreenshot: false,
      },
    });
    expect(await screen.findByText("Saving...")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Mark as applied" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByRole("button", { name: "Reopen visible browser" })).toBeTruthy();
    expect(screen.getByText(/The visible browser is open at the form/)).toBeTruthy();
  });
});
