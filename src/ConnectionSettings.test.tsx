// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import ConnectionSettings from "./ConnectionSettings";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: () => true }));
afterEach(() => { cleanup(); vi.mocked(invoke).mockReset(); });
it("saves entered endpoint and key before testing instead of testing old localhost", async () => {
  const calls: string[] = [];
  vi.mocked(invoke).mockImplementation(async (command) => {
    calls.push(command);
    if (command === "get_chat_settings") return { baseUrl: "http://127.0.0.1:20128/v1", model: "", hasApiKey: false };
    if (command === "save_chat_settings") return { baseUrl: "https://router.example/v1", model: "", hasApiKey: true };
    if (command === "list_chat_models") return ["test-model"];
    return null;
  });
  const user = userEvent.setup();
  render(<ConnectionSettings />);
  const endpoint = await screen.findByLabelText("Endpoint URL");
  await user.clear(endpoint); await user.type(endpoint, "https://router.example/v1");
  await user.type(screen.getByLabelText("API Key"), "fixture-key-not-secret");
  await user.click(screen.getByRole("button", { name: /connection.*load models/i }));
  await screen.findByText(/Connected: 1 model/);
  expect(invoke).toHaveBeenCalledWith("save_chat_settings", { baseUrl: "https://router.example/v1", model: "", apiKey: "fixture-key-not-secret" });
  expect(calls.indexOf("save_chat_settings")).toBeLessThan(calls.indexOf("list_chat_models"));
});
