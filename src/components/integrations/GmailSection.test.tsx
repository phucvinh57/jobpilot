import { cleanup, fireEvent, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/lib/problem";
import { renderWithToasts as render } from "@/test/render";

import * as api from "@/api/gmail";
import type { Gmail } from "@/types/integrations";
import { GmailSection } from "./GmailSection";

vi.mock("@/api/gmail");

const SIGNED_OUT: Gmail = {
  clientId: "",
  hasClientSecret: false,
  account: null,
  lastSync: null,
  lastReport: null,
  pollMinutes: 10,
};
const SIGNED_IN: Gmail = {
  clientId: "abc.apps.googleusercontent.com",
  hasClientSecret: true,
  account: "me@example.com",
  lastSync: new Date(2026, 10, 14, 9, 5).toISOString(),
  lastReport: { fetched: 4, updated: 1, needsReview: 2, error: null },
  pollMinutes: 10,
};

beforeEach(() => {
  vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_OUT);
  vi.mocked(api.onGmailChanged).mockReturnValue(() => undefined);
});

afterEach(cleanup);

describe("GmailSection", () => {
  it("asks for the OAuth client when not connected, with a password box for the secret", async () => {
    render(<GmailSection />);
    expect(screen.getByRole("heading", { name: "Gmail" })).toBeTruthy();
    expect(await screen.findByText("Not connected")).toBeTruthy();
    expect(screen.getByRole("textbox", { name: "Client ID" })).toBeTruthy();
    const secret = screen.getByLabelText("Client secret") as HTMLInputElement;
    expect(secret.type).toBe("password");
    expect(secret.autocomplete).toBe("off");
    expect(screen.getByRole("button", { name: "Connect Gmail" })).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Sync now" })).toBeNull();
  });

  it("fills the saved client id and says a client secret is saved, without showing it", async () => {
    vi.mocked(api.loadGmail).mockResolvedValue({
      ...SIGNED_OUT,
      clientId: "abc.apps.googleusercontent.com",
      hasClientSecret: true,
    });
    render(<GmailSection />);
    const id = (await screen.findByRole("textbox", { name: "Client ID" })) as HTMLInputElement;
    await waitFor(() => expect(id.value).toBe("abc.apps.googleusercontent.com"));
    const secret = screen.getByLabelText("Client secret (leave empty to keep the saved one)") as HTMLInputElement;
    expect(secret.value).toBe("");
  });

  it("needs a client id before it connects, and says so under the fields", async () => {
    render(<GmailSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Connect Gmail" }));
    expect(await screen.findByText("Could not connect: Enter the OAuth client ID.")).toBeTruthy();
    expect(screen.queryByRole("alert", { name: "Could not connect" }), "a form error, not a toast").toBeNull();
    expect(api.connect).not.toHaveBeenCalled();
  });

  it("connects with the typed client, empties the secret box and says who is connected", async () => {
    vi.mocked(api.connect).mockResolvedValue("me@example.com");
    render(<GmailSection />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Client ID" }), {
      target: { value: " cid.apps.googleusercontent.com " },
    });
    const secret = screen.getByLabelText("Client secret") as HTMLInputElement;
    fireEvent.change(secret, { target: { value: "GOCSPX-secret" } });
    vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_IN);
    fireEvent.click(screen.getByRole("button", { name: "Connect Gmail" }));

    await waitFor(() => expect(api.connect).toHaveBeenCalledWith("cid.apps.googleusercontent.com", "GOCSPX-secret"));
    const toast = await screen.findByRole("status", { name: "Gmail connected" });
    expect(within(toast).getByText("Connected as me@example.com. Syncing…")).toBeTruthy();
    expect(screen.queryByDisplayValue("GOCSPX-secret")).toBeNull();
  });

  it("shows that it waits for the browser and can be cancelled", async () => {
    vi.mocked(api.connect).mockReturnValue(new Promise(() => {}));
    vi.mocked(api.cancelConnect).mockResolvedValue(undefined);
    render(<GmailSection />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Client ID" }), { target: { value: "cid" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect Gmail" }));
    expect(await screen.findByText("waiting for browser…")).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
    expect(api.cancelConnect).toHaveBeenCalled();
  });

  it("shows why connecting failed under the fields", async () => {
    vi.mocked(api.connect).mockRejectedValue(new CommandError("sign-in cancelled"));
    render(<GmailSection />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Client ID" }), { target: { value: "cid" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect Gmail" }));
    expect(await screen.findByText("Could not connect: sign-in cancelled")).toBeTruthy();
    expect(document.getElementById("gmail-error")?.getAttribute("role")).toBe("alert");
    expect(screen.queryByRole("alert", { name: "Could not connect" }), "a form error, not a toast").toBeNull();
  });

  it("shows the account, the last sync and what it did when connected", async () => {
    vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_IN);
    render(<GmailSection />);
    expect(await screen.findByText("Connected as me@example.com")).toBeTruthy();
    expect(screen.getByText("Connected")).toBeTruthy();
    expect(screen.getByText("Last sync: 2026-11-14 09:05")).toBeTruthy();
    expect(screen.getByText("fetched 4, updated 1, 2 need review")).toBeTruthy();
    expect(screen.getByText("New mail is checked every 10 min while JobPilot is open.")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Sync now" })).toBeTruthy();
    expect(screen.queryByRole("textbox", { name: "Client ID" })).toBeNull();
  });

  it("tags a problem of the last sync as state, not in red", async () => {
    vi.mocked(api.loadGmail).mockResolvedValue({
      ...SIGNED_IN,
      lastSync: null,
      lastReport: { fetched: 0, updated: 0, needsReview: 0, error: "Gmail is not connected" },
    });
    render(<GmailSection />);
    const line = await screen.findByText("Gmail is not connected");
    expect(line.className).not.toContain("text-danger");
    expect(screen.getByText("Sync problem")).toBeTruthy();
    expect(screen.getByText("Not synced yet")).toBeTruthy();
  });

  it("syncs now and reads the outcome again", async () => {
    vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_IN);
    vi.mocked(api.syncNow).mockResolvedValue(undefined);
    render(<GmailSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Sync now" }));
    await waitFor(() => expect(api.syncNow).toHaveBeenCalled());
    await waitFor(() => expect(vi.mocked(api.loadGmail).mock.calls.length).toBeGreaterThan(1));
  });

  it("reports a failed sync as a toast", async () => {
    vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_IN);
    vi.mocked(api.syncNow).mockRejectedValue(new CommandError("token revoked"));
    render(<GmailSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Sync now" }));
    const toast = await screen.findByRole("alert", { name: "Sync failed" });
    expect(within(toast).getByText("token revoked")).toBeTruthy();
    expect(document.getElementById("gmail-error"), "no error in the card").toBeNull();
  });

  it("reports a failed disconnect as a toast", async () => {
    vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_IN);
    vi.mocked(api.disconnect).mockRejectedValue(new CommandError("keyring locked"));
    render(<GmailSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Disconnect" }));
    fireEvent.click(screen.getByRole("button", { name: "Click again to disconnect" }));
    const toast = await screen.findByRole("alert", { name: "Could not disconnect" });
    expect(within(toast).getByText("keyring locked")).toBeTruthy();
  });

  it("says when the Gmail settings could not be read", async () => {
    vi.mocked(api.loadGmail).mockRejectedValue(new CommandError("database is locked"));
    render(<GmailSection />);
    const toast = await screen.findByRole("alert", { name: "Could not read the Gmail settings" });
    expect(within(toast).getByText("database is locked")).toBeTruthy();
    expect(screen.getByText("Could not read the Gmail settings.")).toBeTruthy();
    vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_OUT);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("Not connected")).toBeTruthy();
  });

  it("disconnects in two steps", async () => {
    vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_IN);
    vi.mocked(api.disconnect).mockResolvedValue(undefined);
    render(<GmailSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Disconnect" }));
    expect(api.disconnect).not.toHaveBeenCalled();
    vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_OUT);
    fireEvent.click(screen.getByRole("button", { name: "Click again to disconnect" }));
    await waitFor(() => expect(api.disconnect).toHaveBeenCalled());
    expect(await screen.findByRole("status", { name: "Gmail disconnected" })).toBeTruthy();
  });

  it("reads again when a background sync stored mail", async () => {
    let changed: () => void = () => undefined;
    vi.mocked(api.onGmailChanged).mockImplementation((handler) => {
      changed = handler;
      return () => undefined;
    });
    vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_IN);
    render(<GmailSection />);
    await screen.findByText("Last sync: 2026-11-14 09:05");
    vi.mocked(api.loadGmail).mockResolvedValue({
      ...SIGNED_IN,
      lastSync: new Date(2026, 10, 14, 10, 30).toISOString(),
    });
    changed();
    expect(await screen.findByText("Last sync: 2026-11-14 10:30")).toBeTruthy();
  });
});
