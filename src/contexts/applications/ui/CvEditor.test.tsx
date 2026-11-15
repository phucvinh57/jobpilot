import { cleanup, render } from "@testing-library/react";
import { afterEach, beforeAll, describe, expect, it, vi } from "vitest";

import { CvEditor } from "./CvEditor";

// jsdom has no layout engine: CodeMirror measures with these.
beforeAll(() => {
  const rect = { x: 0, y: 0, top: 0, left: 0, right: 0, bottom: 0, width: 0, height: 0, toJSON: () => ({}) };
  Range.prototype.getClientRects = () =>
    ({ length: 0, item: () => null, [Symbol.iterator]: [][Symbol.iterator] }) as unknown as DOMRectList;
  Range.prototype.getBoundingClientRect = () => rect as DOMRect;
  document.elementFromPoint = () => null;
});

afterEach(() => {
  cleanup();
});

describe("the cv.yaml editor", () => {
  it("shows the text, names itself, and tells onChange only about the user's edits", () => {
    const onChange = vi.fn();
    const { container, rerender } = render(
      <CvEditor doc={{ text: "name: Ada\n" }} readOnly={false} onChange={onChange} />,
    );
    const content = container.querySelector(".cm-content") as HTMLElement;
    expect(content.getAttribute("aria-label")).toBe("cv.yaml");
    expect(content.textContent).toBe("name: Ada");
    expect(content.getAttribute("contenteditable")).toBe("true");

    // A new document from outside replaces the text without being reported as an edit.
    rerender(<CvEditor doc={{ text: "name: Grace\nrole: Admiral\n" }} readOnly={false} onChange={onChange} />);
    expect(container.querySelector(".cm-content")?.textContent).toBe("name: Gracerole: Admiral");
    expect(onChange).not.toHaveBeenCalled();
  });

  it("colours the keys and the quoted strings with the app's classes", () => {
    const { container } = render(
      <CvEditor doc={{ text: 'name: "Ada"\nyears: 12\n' }} readOnly={false} onChange={() => undefined} />,
    );
    expect(Array.from(container.querySelectorAll(".cm-jp-key")).map((n) => n.textContent)).toEqual(["name", "years"]);
    expect(container.querySelector(".cm-jp-string")?.textContent).toBe('"Ada"');
    expect(container.querySelector(".cm-jp-number")?.textContent).toBe("12");
  });

  it("cannot be edited while it is locked", () => {
    const { container, rerender } = render(<CvEditor doc={{ text: "a: 1\n" }} readOnly onChange={() => undefined} />);
    expect(container.querySelector(".cm-content")?.getAttribute("contenteditable")).toBe("false");
    rerender(<CvEditor doc={{ text: "a: 1\n" }} readOnly={false} onChange={() => undefined} />);
    expect(container.querySelector(".cm-content")?.getAttribute("contenteditable")).toBe("true");
  });
});
