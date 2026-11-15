import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";
import { renderWithToasts as render } from "@/test/render";

import * as api from "@/api/sessions";
import type { Download, Session, Sessions } from "@/types/integrations";
import { SessionsSection } from "./SessionsSection";

vi.mock("@/api/sessions");

const IDLE: Download = { active: false, percent: null };

function session(patch: Partial<Session> = {}): Session {
  return {
    id: 1,
    name: "LinkedIn",
    host: "www.linkedin.com",
    status: "ok",
    verifiedAt: new Date(Date.now() - 5 * 60_000).toISOString(),
    loginInProgress: false,
    ...patch,
  };
}

function sessions(patch: Partial<Sessions> = {}): Sessions {
  return {
    sessions: [
      session(),
      session({ id: 2, name: "Indeed", host: "secure.indeed.com", status: "expired", verifiedAt: null }),
    ],
    chrome: { path: "/usr/bin/chromium", source: "system" },
    download: IDLE,
    ...patch,
  };
}

let downloadEvent: (download: Download) => void = () => undefined;

beforeEach(() => {
  vi.mocked(api.loadSessions).mockResolvedValue(sessions());
  vi.mocked(api.onBrowserDownload).mockImplementation((handler) => {
    downloadEvent = handler;
    return () => undefined;
  });
});

afterEach(cleanup);

describe("SessionsSection", () => {
  it("shows the browser, the tag and one row per session with its state", async () => {
    render(<SessionsSection />);
    expect(screen.getByRole("heading", { name: "Browser sessions" })).toBeTruthy();
    expect(await screen.findByText("Browser: /usr/bin/chromium (system)")).toBeTruthy();
    expect(screen.getByText("Chrome found")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Update browser" })).toBeTruthy();

    const rows = within(screen.getByRole("list", { name: "Sessions" })).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(within(rows[0] as HTMLElement).getByText("www.linkedin.com - verified 5 min ago")).toBeTruthy();
    expect(within(rows[0] as HTMLElement).getByText("ok")).toBeTruthy();
    expect(within(rows[1] as HTMLElement).getByText("secure.indeed.com - never verified")).toBeTruthy();
    expect(within(rows[1] as HTMLElement).getByText("expired")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Check LinkedIn" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "New session" })).toBeTruthy();
  });

  it("says when there is no browser and no session", async () => {
    vi.mocked(api.loadSessions).mockResolvedValue(sessions({ sessions: [], chrome: null }));
    render(<SessionsSection />);
    expect(await screen.findByText("Browser: not found")).toBeTruthy();
    expect(screen.getByText("No browser")).toBeTruthy();
    expect(screen.getByText("No sessions yet")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Download browser" })).toBeTruthy();
  });

  it("checks a session and reads the list again", async () => {
    vi.mocked(api.checkSession).mockResolvedValue(undefined);
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Check Indeed" }));
    await waitFor(() => expect(api.checkSession).toHaveBeenCalledWith(2));
    await waitFor(() => expect(vi.mocked(api.loadSessions).mock.calls.length).toBeGreaterThan(1));
  });

  it("asks Sure? before signing out or deleting", async () => {
    vi.mocked(api.deleteSession).mockResolvedValue(undefined);
    vi.mocked(api.signOutSession).mockResolvedValue(undefined);
    render(<SessionsSection />);
    const del = await screen.findByRole("button", { name: "Delete LinkedIn" });
    fireEvent.click(del);
    expect(api.deleteSession).not.toHaveBeenCalled();
    expect(del.textContent).toBe("Sure?");
    fireEvent.click(del);
    await waitFor(() => expect(api.deleteSession).toHaveBeenCalledWith(1));

    const out = await screen.findByRole("button", { name: "Sign out of Indeed" });
    fireEvent.click(out);
    expect(out.textContent).toBe("Sure?");
    fireEvent.click(out);
    await waitFor(() => expect(api.signOutSession).toHaveBeenCalledWith(2));
  });

  it("waits for the user in a login window and finishes on Done", async () => {
    vi.mocked(api.loadSessions).mockResolvedValue(
      sessions({ sessions: [session({ loginInProgress: true, status: "new", verifiedAt: null })] }),
    );
    vi.mocked(api.finishLogin).mockResolvedValue(undefined);
    render(<SessionsSection />);
    expect(await screen.findByText("Log in in the browser window, then press Done")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Check LinkedIn" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Done logging in to LinkedIn" }));
    await waitFor(() => expect(api.finishLogin).toHaveBeenCalledWith(1));
  });

  it("adds a session in a dialog from a suggestion, closes it when the browser opens and says what to do", async () => {
    vi.mocked(api.createSession).mockResolvedValue(3);
    render(<SessionsSection />);
    expect(await screen.findByRole("button", { name: "New session" })).toBeTruthy();
    expect(screen.queryByRole("dialog"), "the dialog opens on a click").toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "New session" }));
    const dialog = await screen.findByRole("dialog", { name: "New session" });
    expect((within(dialog).getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("LinkedIn");
    expect((within(dialog).getByRole("textbox", { name: "Login address" }) as HTMLInputElement).value).toBe(
      "https://www.linkedin.com/login",
    );

    fireEvent.click(within(dialog).getByRole("button", { name: "Indeed" }));
    expect((within(dialog).getByRole("textbox", { name: "Login address" }) as HTMLInputElement).value).toBe(
      "https://secure.indeed.com/account/login",
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "Open browser to log in" }));
    await waitFor(() =>
      expect(api.createSession).toHaveBeenCalledWith("Indeed", "https://secure.indeed.com/account/login"),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByRole("status", { name: "Log in in the browser window" })).toBeTruthy();
  });

  it("submits the new session with Enter", async () => {
    vi.mocked(api.createSession).mockResolvedValue(3);
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "New session" }));
    const dialog = await screen.findByRole("dialog", { name: "New session" });
    fireEvent.submit(within(dialog).getByRole("textbox", { name: "Name" }));
    await waitFor(() => expect(api.createSession).toHaveBeenCalledWith("LinkedIn", "https://www.linkedin.com/login"));
  });

  it("keeps the dialog open with the reason when the session cannot be created", async () => {
    vi.mocked(api.createSession).mockRejectedValue(new CommandError("not a valid URL"));
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "New session" }));
    const dialog = await screen.findByRole("dialog", { name: "New session" });
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Login address" }), { target: { value: "nope" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Open browser to log in" }));
    expect(await within(dialog).findByText("Could not create the session: not a valid URL")).toBeTruthy();
    expect(screen.getByRole("dialog", { name: "New session" })).toBeTruthy();
    expect(screen.queryByRole("alert", { name: "Could not create the session" }), "not a toast").toBeNull();
  });

  it("cancels the new session dialog", async () => {
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "New session" }));
    const dialog = await screen.findByRole("dialog", { name: "New session" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(api.createSession).not.toHaveBeenCalled();
    expect(screen.getByRole("button", { name: "New session" })).toBeTruthy();
  });

  it("shows the download moving, then reads the browser again", async () => {
    vi.mocked(api.loadSessions).mockResolvedValue(sessions({ chrome: null }));
    vi.mocked(api.downloadBrowser).mockReturnValue(new Promise(() => {}));
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Download browser" }));
    expect(await screen.findByText("Downloading browser...")).toBeTruthy();
    expect(screen.getByText("Downloading")).toBeTruthy();

    act(() => downloadEvent({ active: true, percent: 42 }));
    expect(await screen.findByText("Downloading browser... 42%")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Download browser" })).toHaveProperty("disabled", true);

    vi.mocked(api.loadSessions).mockResolvedValue(sessions());
    act(() => downloadEvent(IDLE));
    expect(await screen.findByText("Browser: /usr/bin/chromium (system)")).toBeTruthy();
  });

  it("picks up a download that started before the card opened", async () => {
    vi.mocked(api.loadSessions).mockResolvedValue(sessions({ chrome: null, download: { active: true, percent: 10 } }));
    render(<SessionsSection />);
    expect(await screen.findByText("Downloading browser... 10%")).toBeTruthy();
  });

  it("reports a failed row action as a toast", async () => {
    vi.mocked(api.checkSession).mockRejectedValue(new CommandError("Chrome not found"));
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Check LinkedIn" }));
    const toast = await screen.findByRole("alert", { name: "The check failed" });
    expect(within(toast).getByText("Chrome not found")).toBeTruthy();
    expect(document.getElementById("sessions-error"), "no banner in the card").toBeNull();
  });

  it("reports a failed download as a toast", async () => {
    vi.mocked(api.loadSessions).mockResolvedValue(sessions({ chrome: null }));
    vi.mocked(api.downloadBrowser).mockRejectedValue(new CommandError("offline"));
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Download browser" }));
    const toast = await screen.findByRole("alert", { name: "Could not download the browser" });
    expect(within(toast).getByText("offline")).toBeTruthy();
  });

  it("says when the sessions could not be read, with a retry", async () => {
    vi.mocked(api.loadSessions).mockRejectedValue(new CommandError("database is locked"));
    render(<SessionsSection />);
    const toast = await screen.findByRole("alert", { name: "Could not read the sessions" });
    expect(within(toast).getByText("database is locked")).toBeTruthy();
    expect(screen.getByText("Could not read the sessions.")).toBeTruthy();
    vi.mocked(api.loadSessions).mockResolvedValue(sessions());
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Browser: /usr/bin/chromium (system)")).toBeTruthy();
  });
});
