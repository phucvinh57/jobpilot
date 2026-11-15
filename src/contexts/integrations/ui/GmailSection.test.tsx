import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { CommandError } from "@/shared";

import * as api from "../api/gmailApi";
import type { Gmail } from "../domain/gmail";
import { GmailSection } from "./GmailSection";

vi.mock("../api/gmailApi");

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

  it("needs a client id before it connects", async () => {
    render(<GmailSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Connect Gmail" }));
    expect(await screen.findByText("Enter the OAuth client ID.")).toBeTruthy();
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
    expect(await screen.findByText("Connected as me@example.com. Syncing…")).toBeTruthy();
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

  it("shows why connecting failed", async () => {
    vi.mocked(api.connect).mockRejectedValue(new CommandError("sign-in cancelled"));
    render(<GmailSection />);
    fireEvent.change(await screen.findByRole("textbox", { name: "Client ID" }), { target: { value: "cid" } });
    fireEvent.click(screen.getByRole("button", { name: "Connect Gmail" }));
    expect(await screen.findByText("sign-in cancelled")).toBeTruthy();
    expect(screen.getByRole("group", { name: "Could not connect" })).toBeTruthy();
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

  it("shows a sync problem in the danger colour", async () => {
    vi.mocked(api.loadGmail).mockResolvedValue({
      ...SIGNED_IN,
      lastSync: null,
      lastReport: { fetched: 0, updated: 0, needsReview: 0, error: "Gmail is not connected" },
    });
    render(<GmailSection />);
    const line = await screen.findByText("Sync problem: Gmail is not connected");
    expect(line.className).toContain("text-danger");
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

  it("disconnects in two steps", async () => {
    vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_IN);
    vi.mocked(api.disconnect).mockResolvedValue(undefined);
    render(<GmailSection />);
    fireEvent.click(await screen.findByRole("button", { name: "Disconnect" }));
    expect(api.disconnect).not.toHaveBeenCalled();
    vi.mocked(api.loadGmail).mockResolvedValue(SIGNED_OUT);
    fireEvent.click(screen.getByRole("button", { name: "Click again to disconnect" }));
    await waitFor(() => expect(api.disconnect).toHaveBeenCalled());
    expect(await screen.findByText("Disconnected.")).toBeTruthy();
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
