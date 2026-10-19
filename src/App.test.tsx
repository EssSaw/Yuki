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

  it("follows workspace palettes only in Auto mode", async () => {
    const user = userEvent.setup();
    const { container } = render(<App />);
    const theme = () => container.querySelector(".workspace")?.getAttribute("data-theme");
    expect(theme()).toBe("balanced");
    await user.click(screen.getByRole("button", { name: "Projects" }));
    expect(theme()).toBe("technical");
    await user.click(screen.getByRole("button", { name: "Creative" }));
    expect(theme()).toBe("creative");
    await user.click(screen.getByRole("button", { name: "Settings" }));
    expect(theme()).toBe("creative");
    await user.click(screen.getByRole("button", { name: "Auto" }));
    expect(theme()).toBe("balanced");
    await user.click(screen.getByRole("button", { name: "Notes & canvas" }));
    expect(theme()).toBe("creative");
  });

  it("navigates planned sections and returns to chat", async () => {
    const user = userEvent.setup();
    render(<App />);
    for (const section of ["Projects", "Notes & canvas", "Schedule", "Settings"]) {
      await user.click(screen.getByRole("button", { name: section }));
      expect(screen.getByRole("heading", { name: section })).toBeTruthy();
      if (section === "Projects" || section === "Settings") {
        expect(screen.getByText(/Browser preview has no filesystem access/)).toBeTruthy();
      } else {
        expect(screen.getByText("Not connected yet")).toBeTruthy();
      }
    }
    await user.click(screen.getByRole("button", { name: "Chat" }));
    expect(screen.getByRole("heading", { name: "A little space to think together." })).toBeTruthy();
  });
});
