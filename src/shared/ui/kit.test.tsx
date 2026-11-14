import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { cn } from "./cn";
import { Button, Checkbox, DataTable, Drawer, StageBar, Switch } from "./kit";

afterEach(cleanup);

describe("cn", () => {
  it("keeps an app text size next to a text colour", () => {
    expect(cn("text-small text-muted")).toBe("text-small text-muted");
    expect(cn("text-body", "text-small")).toBe("text-small");
    expect(cn("text-ink", "text-muted")).toBe("text-muted");
  });
});

describe("Button", () => {
  it("marks the primary button with bg-accent, and only it", () => {
    render(
      <>
        <Button variant="primary">Go</Button>
        <Button>Later</Button>
      </>,
    );
    expect(screen.getByRole("button", { name: "Go" }).className).toContain("bg-accent");
    expect(screen.getByRole("button", { name: "Later" }).className).not.toContain("bg-accent");
  });

  it("is disabled while busy", () => {
    render(<Button busy>Save</Button>);
    expect(screen.getByRole("button", { name: "Save" })).toHaveProperty("disabled", true);
  });

  it("keeps the pointer on a disabled button, so its title shows", async () => {
    const onClick = vi.fn();
    render(
      <Button disabled title="No provider yet" onClick={onClick}>
        Generate
      </Button>,
    );
    const button = screen.getByRole("button", { name: "Generate" });
    const classes = button.className.split(" ");
    expect(classes).toContain("disabled:pointer-events-auto");
    expect(classes).toContain("disabled:cursor-not-allowed");
    expect(classes, "the primitive's rule is merged away").not.toContain("disabled:pointer-events-none");
    expect(button.title).toBe("No provider yet");
    // Still never clicked.
    await userEvent.click(button, { pointerEventsCheck: 0 });
    expect(onClick).not.toHaveBeenCalled();
  });
});

describe("Checkbox and Switch", () => {
  it("gives onChange the native checked state", async () => {
    const onChange = vi.fn();
    render(<Checkbox label="Remote only" checked={false} onChange={(e) => onChange(e.target.checked)} />);
    await userEvent.click(screen.getByRole("checkbox", { name: "Remote only" }));
    expect(onChange).toHaveBeenCalledWith(true);
  });

  it("switches with role switch", async () => {
    const onChange = vi.fn();
    render(<Switch ariaLabel="Collect" checked={false} onChange={onChange} />);
    const toggle = screen.getByRole("switch", { name: "Collect" });
    expect(toggle.getAttribute("aria-checked")).toBe("false");
    await userEvent.click(toggle);
    expect(onChange).toHaveBeenCalledWith(true);
  });
});

describe("Drawer", () => {
  function Harness({ onClose }: { onClose: () => void }) {
    const [open, setOpen] = useState(false);
    return (
      <>
        <button type="button" onClick={() => setOpen(true)}>
          Open it
        </button>
        <button type="button">Another row</button>
        <Drawer
          open={open}
          title="Acme: Backend Engineer"
          onClose={() => {
            onClose();
            setOpen(false);
          }}
          actions={<Button>Dismiss</Button>}
        >
          <p>Body text</p>
        </Drawer>
      </>
    );
  }

  it("is named by its title and closes with its close button", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    expect(screen.queryByRole("dialog")).toBeNull();
    await user.click(screen.getByRole("button", { name: "Open it" }));
    const drawer = screen.getByRole("dialog", { name: "Acme: Backend Engineer" });
    expect(drawer.id).toBe("drawer");
    expect(screen.getByText("Body text")).toBeTruthy();
    expect(screen.getByRole("button", { name: "Dismiss" })).toBeTruthy();
    await user.click(screen.getByRole("button", { name: "Close" }));
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
  });

  it("closes on Escape and gives focus back", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    const opener = screen.getByRole("button", { name: "Open it" });
    await user.click(opener);
    const drawer = screen.getByRole("dialog");
    expect(drawer.contains(document.activeElement)).toBe(true);
    await user.keyboard("{Escape}");
    expect(onClose).toHaveBeenCalledTimes(1);
    expect(screen.queryByRole("dialog")).toBeNull();
    expect(document.activeElement).toBe(opener);
  });

  it("stays open while the page behind is used", async () => {
    const user = userEvent.setup();
    const onClose = vi.fn();
    render(<Harness onClose={onClose} />);
    await user.click(screen.getByRole("button", { name: "Open it" }));
    await user.click(screen.getByRole("button", { name: "Another row" }));
    expect(onClose).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeTruthy();
  });
});

describe("DataTable", () => {
  interface Row {
    id: string;
    company: string;
    role: string;
  }
  const rows: Row[] = [
    { id: "1", company: "Acme", role: "Backend" },
    { id: "2", company: "Globex", role: "Platform" },
  ];
  const columns = [
    {
      key: "pick",
      header: "Pick",
      cell: (row: Row) => <input type="checkbox" aria-label={`Select ${row.company}`} />,
    },
    { key: "company", header: "Company", cell: (row: Row) => row.company },
    { key: "role", header: "Role", cell: (row: Row) => row.role },
  ];

  it("opens a row by click or Enter and marks it current", async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    const { rerender } = render(
      <DataTable label="Leads" columns={columns} rows={rows} rowKey={(r) => r.id} rowLabel={(r) => `${r.company}: ${r.role}`} onOpen={onOpen} />,
    );
    expect(screen.getByRole("table", { name: "Leads" })).toBeTruthy();
    await user.click(screen.getByText("Globex"));
    expect(onOpen).toHaveBeenLastCalledWith(rows[1]);

    const acme = screen.getByRole("row", { name: "Acme: Backend" });
    acme.focus();
    await user.keyboard("{Enter}");
    expect(onOpen).toHaveBeenLastCalledWith(rows[0]);
    expect(onOpen).toHaveBeenCalledTimes(2);

    rerender(
      <DataTable label="Leads" columns={columns} rows={rows} rowKey={(r) => r.id} rowLabel={(r) => `${r.company}: ${r.role}`} onOpen={onOpen} openKey="1" />,
    );
    expect(screen.getByRole("row", { name: "Acme: Backend" }).getAttribute("aria-current")).toBe("true");
    expect(screen.getByRole("row", { name: "Globex: Platform" }).getAttribute("aria-current")).toBeNull();
  });

  it("does not open a row when a control inside it is used", async () => {
    const user = userEvent.setup();
    const onOpen = vi.fn();
    render(<DataTable label="Leads" columns={columns} rows={rows} rowKey={(r) => r.id} onOpen={onOpen} />);
    await user.click(screen.getByRole("checkbox", { name: "Select Acme" }));
    expect(onOpen).not.toHaveBeenCalled();
  });

  it("shows the empty text without rows", () => {
    render(<DataTable label="Leads" columns={columns} rows={[]} rowKey={(r) => r.id} empty="No leads yet." />);
    expect(screen.getByText("No leads yet.")).toBeTruthy();
  });
});

describe("StageBar", () => {
  const stages = [
    { key: "draft", label: "Draft", count: 2 },
    { key: "applied", label: "Applied", count: 5 },
  ] as const;

  it("selects a stage by click and by arrow key", async () => {
    const user = userEvent.setup();
    const onChange = vi.fn();
    render(<StageBar label="Stages" stages={stages} value="draft" onChange={onChange} />);
    expect(screen.getByRole("tablist", { name: "Stages" })).toBeTruthy();
    const draft = screen.getByRole("tab", { name: "Draft, 2" });
    expect(draft.id).toBe("stage-draft");
    expect(draft.getAttribute("aria-selected")).toBe("true");
    expect(screen.getByRole("tab", { name: "Applied, 5" }).getAttribute("aria-selected")).toBe("false");
    await user.click(screen.getByRole("tab", { name: "Applied, 5" }));
    expect(onChange).toHaveBeenLastCalledWith("applied");
    draft.focus();
    await user.keyboard("{ArrowRight}");
    expect(onChange).toHaveBeenLastCalledWith("applied");
  });
});
