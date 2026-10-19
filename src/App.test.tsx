// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it } from "vitest";
import App from "./App";

afterEach(cleanup);

describe("desktop foundation", () => {
  it("opens chat without pretending a model is connected", () => {
    render(<App />);
    expect(screen.getByRole("heading", { name: "A little space to think together." })).toBeTruthy();
    expect(screen.getByRole("button", { name: "Send message" }).hasAttribute("disabled")).toBe(true);
    expect(screen.getByText("9router not connected")).toBeTruthy();
  });

  it("navigates planned sections and returns to chat", async () => {
    const user = userEvent.setup();
    render(<App />);
    for (const section of ["Projects", "Notes & canvas", "Schedule", "Settings"]) {
      await user.click(screen.getByRole("button", { name: section }));
      expect(screen.getByRole("heading", { name: section })).toBeTruthy();
      expect(screen.getByText("Not connected yet")).toBeTruthy();
    }
    await user.click(screen.getByRole("button", { name: "Chat" }));
    expect(screen.getByRole("heading", { name: "A little space to think together." })).toBeTruthy();
  });
});
