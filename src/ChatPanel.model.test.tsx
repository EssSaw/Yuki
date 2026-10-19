// @vitest-environment jsdom
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import ChatPanel from "./ChatPanel";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
afterEach(() => { cleanup(); vi.mocked(invoke).mockReset(); });
it("loads models explicitly in chat and saves the selected model without changing credentials", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "get_chat_settings") return { baseUrl: "https://router.example/v1", model: "", hasApiKey: true };
    if (command === "list_conversations") return [];
    if (command === "list_chat_models") return ["first", "second"];
    if (command === "save_chat_settings") return { baseUrl: "https://router.example/v1", model: "second", hasApiKey: true };
    return null;
  });
  const user = userEvent.setup(); render(<ChatPanel />);
  const load = screen.getByRole("button", { name: "Load models" });
  await waitFor(() => expect(load.hasAttribute("disabled")).toBe(false));
  expect(invoke).not.toHaveBeenCalledWith("list_chat_models");
  await user.click(load);
  await screen.findByRole("option", { name: "second" });
  await user.selectOptions(screen.getByRole("combobox", { name: "Chat model" }), "second");
  expect(invoke).toHaveBeenCalledWith("save_chat_settings", { baseUrl: "https://router.example/v1", model: "second" });
  await waitFor(() => expect(screen.getByRole("combobox", { name: "Chat model" })).toHaveProperty("value", "second"));
});
