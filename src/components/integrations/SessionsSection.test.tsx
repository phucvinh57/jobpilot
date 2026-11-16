import { act, cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";
import { renderWithToasts as render } from "@/test/render";
import { browserSection, CONNECTED, DISCONNECTED, PAIRING, PAIRING_APPROVED, site } from "@/test/fixtures/integrations";

import * as api from "@/api/sessions";
import { SessionsSection } from "./SessionsSection";

vi.mock("@/api/sessions");

let statusEvent: () => void = () => undefined;

beforeEach(() => {
  vi.mocked(api.loadBrowser).mockResolvedValue(browserSection());
  vi.mocked(api.onBrowserStatus).mockImplementation((handler) => {
    statusEvent = handler;
    return () => undefined;
  });
});

afterEach(cleanup);

/** The primary buttons on screen (the kit marks THE primary with `bg-accent`). */
function primaries(): HTMLElement[] {
  return screen.queryAllByRole("button").filter((b) => b.className.includes("text-on-fill"));
}

describe("SessionsSection: the connection", () => {
  it("names the connected browser, says whether private tabs work and offers Forget", async () => {
    render(<SessionsSection />);
    expect(screen.getByRole("heading", { name: "Browser" })).toBeTruthy();
    expect(await screen.findByText("Chrome 153, JobPilot extension 0.1.0")).toBeTruthy();
    expect(screen.getByText("Connected")).toBeTruthy();
    expect(screen.getByText("Private tabs: allowed")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Forget this browser" })).toBeTruthy();
    expect(screen.queryByRole("list", { name: "Install the extension" }), "no install steps").toBeNull();
    expect(primaries(), "nothing on the page is the primary").toHaveLength(0);
  });

  it("tells how to allow private tabs when the extension may not use Incognito", async () => {
    vi.mocked(api.loadBrowser).mockResolvedValue(
      browserSection({ browser: { ...CONNECTED, incognitoAllowed: false } }),
    );
    render(<SessionsSection />);
    expect(await screen.findByText(/^Private tabs: not allowed \(turn on Allow in Incognito/)).toBeTruthy();
  });

  it("shows the install steps with the folder to load, and copies it", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: DISCONNECTED }));
    render(<SessionsSection />);
    const steps = await screen.findByRole("list", { name: "Install the extension" });
    expect(within(steps).getByText("chrome://extensions")).toBeTruthy();
    expect(within(steps).getByText("Turn on Developer mode (top right).")).toBeTruthy();
    expect(within(steps).getByText('Press "Load unpacked" and pick this folder:')).toBeTruthy();
    expect(within(steps).getByText("/opt/jobpilot/extension")).toBeTruthy();
    expect(screen.getByText("Not connected")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Forget this browser" })).toBeNull();

    fireEvent.click(within(steps).getByRole("button", { name: "Copy the extension folder" }));
    await waitFor(() => expect(writeText).toHaveBeenCalledWith("/opt/jobpilot/extension"));
    expect(await screen.findByRole("status", { name: "Folder copied" })).toBeTruthy();
  });

  it("says when the extension folder is not known", async () => {
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: { ...DISCONNECTED, extensionPath: null } }));
    render(<SessionsSection />);
    expect(await screen.findByText(/not found here: build it with bun run ext:build/)).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Copy the extension folder" })).toBeNull();
  });

  it("says when JobPilot cannot wait for the extension", async () => {
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: { ...DISCONNECTED, listening: false } }));
    render(<SessionsSection />);
    expect(await screen.findByText(/port 47615 is used by another program/)).toBeTruthy();
    expect(screen.getByText("Off")).toBeTruthy();
  });

  it("asks to pair a browser with its code and pairs it on Pair", async () => {
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: PAIRING, sessions: [] }));
    vi.mocked(api.approvePairing).mockResolvedValue(undefined);
    render(<SessionsSection />);
    const dialog = await screen.findByRole("dialog", { name: "Pair this browser?" });
    expect(within(dialog).getByText("482913")).toBeTruthy();
    expect(within(dialog).getByText(/matches the code in the extension's popup/)).toBeTruthy();
    expect(primaries().map((b) => b.textContent)).toEqual(["Pair"]);

    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection());
    fireEvent.click(within(dialog).getByRole("button", { name: "Pair" }));
    await waitFor(() => expect(api.approvePairing).toHaveBeenCalledTimes(1));
    expect(await screen.findByRole("status", { name: "Browser paired" })).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByText("Chrome 153, JobPilot extension 0.1.0")).toBeTruthy();
    expect(api.declinePairing).not.toHaveBeenCalled();
  });

  it("after Pair, waits for Pair in the extension's popup, then says the browser is paired", async () => {
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: PAIRING, sessions: [] }));
    vi.mocked(api.approvePairing).mockResolvedValue(undefined);
    render(<SessionsSection />);
    const dialog = await screen.findByRole("dialog", { name: "Pair this browser?" });

    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: PAIRING_APPROVED, sessions: [] }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Pair" }));
    expect(await within(dialog).findByText("Now press Pair in the extension's popup.")).toBeTruthy();
    const waiting = within(dialog).getByRole("button", { name: "Waiting for the extension" });
    expect((waiting as HTMLButtonElement).disabled).toBe(true);
    expect(primaries().map((b) => b.textContent)).toEqual(["Waiting for the extension"]);
    expect(within(dialog).getByRole("button", { name: "Decline" })).toBeTruthy();
    expect(screen.getByText("Waiting for the extension", { selector: "span" })).toBeTruthy();
    expect(screen.queryByRole("status", { name: "Browser paired" })).toBeNull();

    // The user pressed Pair in the popup: the extension is connected.
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection());
    act(() => statusEvent());
    expect(await screen.findByRole("status", { name: "Browser paired" })).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });

  it("declines a browser the user does not know", async () => {
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: PAIRING }));
    vi.mocked(api.declinePairing).mockResolvedValue(undefined);
    render(<SessionsSection />);
    const dialog = await screen.findByRole("dialog", { name: "Pair this browser?" });
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: DISCONNECTED }));
    fireEvent.click(within(dialog).getByRole("button", { name: "Decline" }));
    await waitFor(() => expect(api.declinePairing).toHaveBeenCalledTimes(1));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(api.approvePairing).not.toHaveBeenCalled();
  });

  it("opens the pairing dialog when a browser asks while the section is open, and again after Close", async () => {
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: DISCONNECTED }));
    render(<SessionsSection />);
    await screen.findByRole("list", { name: "Install the extension" });
    expect(screen.queryByRole("dialog")).toBeNull();

    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: PAIRING }));
    act(() => statusEvent());
    const dialog = await screen.findByRole("dialog", { name: "Pair this browser?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Close" }));
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    const waiting = screen.getByRole("group", { name: "A browser asks to pair" });
    fireEvent.click(within(waiting).getByRole("button", { name: "Review" }));
    expect(await screen.findByRole("dialog", { name: "Pair this browser?" })).toBeTruthy();
  });

  it("reports a failed pairing as a toast", async () => {
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: PAIRING }));
    vi.mocked(api.approvePairing).mockRejectedValue(new CommandError("no browser waits for pairing"));
    render(<SessionsSection />);
    const dialog = await screen.findByRole("dialog", { name: "Pair this browser?" });
    fireEvent.click(within(dialog).getByRole("button", { name: "Pair" }));
    await waitFor(() => expect(screen.queryByRole("dialog"), "closed, so the toast is seen").toBeNull());
    const toast = await screen.findByRole("alert", { name: "Could not pair the browser" });
    expect(within(toast).getByText("no browser waits for pairing")).toBeTruthy();
  });

  it("forgets the browser after a confirmation", async () => {
    vi.mocked(api.forgetBrowser).mockResolvedValue(undefined);
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Forget this browser" }));
    const dialog = await screen.findByRole("dialog", { name: "Forget this browser?" });
    expect(api.forgetBrowser).not.toHaveBeenCalled();
    fireEvent.click(within(dialog).getByRole("button", { name: "Forget" }));
    await waitFor(() => expect(api.forgetBrowser).toHaveBeenCalledTimes(1));
    expect(await screen.findByRole("status", { name: "Browser forgotten" })).toBeTruthy();
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
  });
});

describe("SessionsSection: the sites", () => {
  it("shows one row per site with its state, and no Sign out", async () => {
    render(<SessionsSection />);
    const rows = within(await screen.findByRole("list", { name: "Sites" })).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(within(rows[0] as HTMLElement).getByText("www.linkedin.com - checked 5 min ago")).toBeTruthy();
    expect(within(rows[0] as HTMLElement).getByText("signed in")).toBeTruthy();
    expect(within(rows[1] as HTMLElement).getByText("secure.indeed.com - never checked")).toBeTruthy();
    expect(within(rows[1] as HTMLElement).getByText("signed out")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Sign in to LinkedIn" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Check LinkedIn" })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Delete LinkedIn" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: /Sign out/ })).toBeNull();
    expect(screen.getByRole("button", { name: "Add a site" })).toBeTruthy();
  });

  it("says when there is no site yet", async () => {
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ sessions: [] }));
    render(<SessionsSection />);
    expect(await screen.findByText("No sites yet")).toBeTruthy();
  });

  it("cannot sign in or check while no browser is connected", async () => {
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection({ browser: DISCONNECTED }));
    render(<SessionsSection />);
    expect(await screen.findByRole("button", { name: "Sign in to LinkedIn" })).toHaveProperty("disabled", true);
    expect(screen.getByRole("button", { name: "Check LinkedIn" })).toHaveProperty("disabled", true);
    expect(screen.getByRole("button", { name: "Delete LinkedIn" })).toHaveProperty("disabled", false);
  });

  it("checks a site and reads the section again", async () => {
    vi.mocked(api.checkSession).mockResolvedValue(undefined);
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Check Indeed" }));
    await waitFor(() => expect(api.checkSession).toHaveBeenCalledWith(2));
    await waitFor(() => expect(vi.mocked(api.loadBrowser).mock.calls.length).toBeGreaterThan(1));
  });

  it("opens the sign-in tab of a site", async () => {
    vi.mocked(api.openSession).mockResolvedValue(undefined);
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Sign in to Indeed" }));
    await waitFor(() => expect(api.openSession).toHaveBeenCalledWith(2));
  });

  it("asks Sure? before deleting", async () => {
    vi.mocked(api.deleteSession).mockResolvedValue(undefined);
    render(<SessionsSection />);
    const del = await screen.findByRole("button", { name: "Delete LinkedIn" });
    fireEvent.click(del);
    expect(api.deleteSession).not.toHaveBeenCalled();
    expect(del.textContent).toBe("Sure?");
    fireEvent.click(del);
    await waitFor(() => expect(api.deleteSession).toHaveBeenCalledWith(1));
  });

  it("waits for the user in the sign-in tab and finishes on Done", async () => {
    vi.mocked(api.loadBrowser).mockResolvedValue(
      browserSection({ sessions: [site({ loginInProgress: true, status: "new", verifiedAt: null })] }),
    );
    vi.mocked(api.finishLogin).mockResolvedValue(undefined);
    render(<SessionsSection />);
    expect(await screen.findByText("Sign in in your browser, then press Done")).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Check LinkedIn" })).toBeNull();
    fireEvent.click(screen.getByRole("button", { name: "Done signing in to LinkedIn" }));
    await waitFor(() => expect(api.finishLogin).toHaveBeenCalledWith(1));
  });

  it("adds a site in a dialog from a suggestion, closes it when the tab opens and says what to do", async () => {
    vi.mocked(api.createSession).mockResolvedValue(3);
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Add a site" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a site" });
    expect((within(dialog).getByRole("textbox", { name: "Name" }) as HTMLInputElement).value).toBe("LinkedIn");
    expect(primaries().map((b) => b.textContent)).toEqual(["Open the sign-in tab"]);

    fireEvent.click(within(dialog).getByRole("button", { name: "Indeed" }));
    expect((within(dialog).getByRole("textbox", { name: "Sign-in address" }) as HTMLInputElement).value).toBe(
      "https://secure.indeed.com/account/login",
    );
    fireEvent.click(within(dialog).getByRole("button", { name: "Open the sign-in tab" }));
    await waitFor(() =>
      expect(api.createSession).toHaveBeenCalledWith("Indeed", "https://secure.indeed.com/account/login"),
    );
    await waitFor(() => expect(screen.queryByRole("dialog")).toBeNull());
    expect(await screen.findByRole("status", { name: "Sign in in your browser" })).toBeTruthy();
  });

  it("keeps the dialog open with the reason when the site cannot be added", async () => {
    vi.mocked(api.createSession).mockRejectedValue(new CommandError("not a valid URL"));
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Add a site" }));
    const dialog = await screen.findByRole("dialog", { name: "Add a site" });
    fireEvent.change(within(dialog).getByRole("textbox", { name: "Sign-in address" }), { target: { value: "nope" } });
    fireEvent.submit(within(dialog).getByRole("textbox", { name: "Name" }));
    expect(await within(dialog).findByText("Could not add the site: not a valid URL")).toBeTruthy();
    expect(screen.queryByRole("alert", { name: "Could not add the site" }), "not a toast").toBeNull();
  });

  it("reports a failed row action as a toast", async () => {
    vi.mocked(api.checkSession).mockRejectedValue(new CommandError("the JobPilot browser extension is not connected"));
    render(<SessionsSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Check LinkedIn" }));
    const toast = await screen.findByRole("alert", { name: "The check failed" });
    expect(within(toast).getByText("the JobPilot browser extension is not connected")).toBeTruthy();
  });

  it("says when the section could not be read, with a retry", async () => {
    vi.mocked(api.loadBrowser).mockRejectedValue(new CommandError("database is locked"));
    render(<SessionsSection />);
    const toast = await screen.findByRole("alert", { name: "Could not read the browser" });
    expect(within(toast).getByText("database is locked")).toBeTruthy();
    vi.mocked(api.loadBrowser).mockResolvedValue(browserSection());
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Chrome 153, JobPilot extension 0.1.0")).toBeTruthy();
  });
});
