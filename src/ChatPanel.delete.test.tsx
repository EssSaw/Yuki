// @vitest-environment jsdom
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import ChatPanel from "./ChatPanel";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
afterEach(() => { cleanup(); vi.mocked(invoke).mockReset(); });

it.each([false, true])("requires confirmation and preserves history on failed deletion: %s", async (fail) => {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "get_chat_settings") return { baseUrl: "https://router.example/v1", model: "test", hasApiKey: true };
    if (command === "list_conversations") return [{ id: "fixture-id", title: "My plan", updatedAt: "today" }];
    if (command === "delete_conversation" && fail) throw new Error("Deletion failed");
    return null;
  });
  const user = userEvent.setup(); render(<ChatPanel />);
  await user.click(screen.getByRole("button", { name: /Conversations/ }));
  await user.click(await screen.findByRole("button", { name: "Delete conversation: My plan" }));
  expect(invoke).not.toHaveBeenCalledWith("delete_conversation", expect.anything());
  await user.click(screen.getByRole("button", { name: "Keep conversation" }));
  expect(screen.queryByRole("alertdialog")).toBeNull();
  await user.click(screen.getByRole("button", { name: "Delete conversation: My plan" }));
  await user.click(screen.getByRole("button", { name: "Delete permanently" }));
  expect(invoke).toHaveBeenCalledWith("delete_conversation", { id: "fixture-id" });
  if (fail) {
    expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Error: Deletion failed");
    expect(screen.getByRole("button", { name: "My plan" })).toBeTruthy();
  } else {
    await waitFor(() => expect(screen.queryByRole("button", { name: "My plan" })).toBeNull());
    expect(screen.queryByRole("alertdialog")).toBeNull();
  }
});
