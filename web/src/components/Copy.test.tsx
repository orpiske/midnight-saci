import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { COPY_FEEDBACK_MS, CopyText } from "./Copy";

function mockClipboard(writeText: (text: string) => Promise<void>) {
  Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true });
}

async function clickCopy() {
  await act(async () => {
    fireEvent.click(screen.getByRole("button", { name: "Copy run ID" }));
  });
}

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

describe("CopyText", () => {
  it("says Copied after a copy, then clears the message", async () => {
    vi.useFakeTimers();
    const writeText = vi.fn(() => Promise.resolve());
    mockClipboard(writeText);
    render(<CopyText value="run-123" title="run ID" />);
    expect(screen.getByRole("status").textContent).toBe("");

    await clickCopy();
    expect(writeText).toHaveBeenCalledWith("run-123");
    expect(screen.getByRole("status").textContent).toBe("Copied");

    act(() => {
      vi.advanceTimersByTime(COPY_FEEDBACK_MS);
    });
    expect(screen.getByRole("status").textContent).toBe("");
  });

  it("says Copy failed when the clipboard rejects the write", async () => {
    mockClipboard(() => Promise.reject(new Error("denied")));
    render(<CopyText value="run-123" title="run ID" />);

    await clickCopy();
    expect(screen.getByRole("status").textContent).toBe("Copy failed");
  });
});
