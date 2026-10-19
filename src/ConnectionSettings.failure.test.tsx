// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import ConnectionSettings from "./ConnectionSettings";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: () => true }));
afterEach(() => { cleanup(); vi.mocked(invoke).mockReset(); });
it("does not test stale saved connection when saving the draft fails", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "get_chat_settings") return { baseUrl: "http://localhost:20128/v1", model: "", hasApiKey: false };
    if (command === "save_chat_settings") throw new Error("Could not save connection");
    return [];
  });
  const user = userEvent.setup(); render(<ConnectionSettings />);
  await user.click(await screen.findByRole("button", { name: "Save connection & load models" }));
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Error: Could not save connection");
  expect(invoke).not.toHaveBeenCalledWith("list_chat_models");
});
