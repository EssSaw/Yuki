// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import ChatPanel, { type ChatStreamPayload } from "./ChatPanel";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn() }));
let emit: (payload: ChatStreamPayload) => void;
let requestId: string;
let rejectStart: ((reason: string) => void) | undefined;
const unlisten = vi.fn();

beforeEach(() => {
  rejectStart = undefined;
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "get_chat_settings") return { baseUrl: "http://localhost/v1", model: "test", hasApiKey: false };
    if (command === "list_conversations") return [{ id: "saved", title: "Saved chat", updatedAt: "today" }];
    if (command === "load_conversation") return { id: "saved", messages: [{ id: "old", role: "assistant", content: "Saved response", status: "complete" }] };
    if (command === "start_chat") {
      requestId = (args as { requestId: string }).requestId;
      return new Promise((_, reject) => { rejectStart = reject; });
    }
    return null;
  });
  vi.mocked(listen).mockImplementation(async (_event, handler) => {
    emit = (payload) => handler({ payload } as Parameters<typeof handler>[0]);
    return unlisten;
  });
});
afterEach(() => { cleanup(); vi.resetAllMocks(); });

async function send() {
  const user = userEvent.setup();
  await waitFor(() => expect(screen.getByRole("button", { name: /Choose chat model: test/ })).toBeTruthy());
  await user.type(screen.getByRole("textbox", { name: "Message Yuki" }), "Inspect dummy project");
  await user.keyboard("{Enter}");
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_chat", expect.anything()));
  return user;
}
function activity(id = "tool-1", status: "running" | "succeeded" | "failed" = "running", summary = "Inspecting project directory") {
  act(() => emit({ requestId, conversationId: "current", toolActivity: { id, name: "list_project_directory", status, summary } }));
}
function terminal(status: "complete" | "cancelled" | "failed" = "complete") {
  act(() => emit({ requestId, conversationId: "current", done: true, status }));
}

it("updates an operation by id, keeps message streaming, and never renders raw tool metadata", async () => {
  render(<ChatPanel />); await send();
  activity();
  expect(screen.getByText("Running")).toBeTruthy();
  act(() => emit({ requestId, delta: "Answer text" }));
  activity("tool-1", "succeeded", "Found 3 entries");
  const panel = screen.getByRole("region", { name: "Tool activity" });
  expect(within(panel).getAllByRole("listitem")).toHaveLength(1);
  expect(within(panel).getByText("Succeeded")).toBeTruthy();
  expect(screen.getByText("Answer text")).toBeTruthy();
  expect(panel.textContent).not.toContain("list_project_directory");
  expect(panel.getAttribute("aria-live")).toBeNull();
  activity("tool-1");
  expect(screen.queryByText("Running")).toBeNull();
  terminal();
});

it("ignores stale request ids and bounds activity to the last 16 operations", async () => {
  render(<ChatPanel />); await send();
  act(() => emit({ requestId: "stale", conversationId: "other", toolActivity: { id: "stale", name: "read", status: "running", summary: "Stale evidence" } }));
  expect(screen.queryByText("Stale evidence")).toBeNull();
  for (let i = 0; i < 20; i++) activity(`tool-${i}`, "succeeded", `Operation ${i}`);
  expect(within(screen.getByRole("region", { name: "Tool activity" })).getAllByRole("listitem")).toHaveLength(16);
  expect(screen.queryByText("Operation 0")).toBeNull();
  terminal();
});

it.each(["complete", "cancelled", "failed"] as const)("stops unfinished operations on terminal %s without inventing success", async (status) => {
  render(<ChatPanel />); await send();
  activity(); activity("finished", "succeeded", "Found 2 entries");
  activity("denied", "failed", "Access denied");
  terminal(status);
  expect(screen.queryByText("Running")).toBeNull();
  expect(screen.getByText("Stopped - completion not confirmed")).toBeTruthy();
  expect(screen.getByText("Succeeded")).toBeTruthy();
  expect(screen.getByText("Access denied")).toBeTruthy();
});

it("stops activity on stream error and rejected start", async () => {
  render(<ChatPanel />); await send(); activity();
  act(() => emit({ requestId, error: "Request interrupted" }));
  expect(screen.queryByText("Running")).toBeNull();
  act(() => rejectStart?.("Transport failed"));
  await screen.findByText("Transport failed");
  expect(screen.getByText("Stopped - completion not confirmed")).toBeTruthy();
});

it("uses neutral fallback text when the native optional summary is absent", async () => {
  render(<ChatPanel />); await send();
  act(() => emit({ requestId, toolActivity: { id: "no-summary", name: "private-looking-name", status: "running" } }));
  expect(screen.getByText("Tool operation")).toBeTruthy();
  expect(screen.queryByText("private-looking-name")).toBeNull();
  terminal();
});

it("keeps activity running when cancellation is rejected", async () => {
  render(<ChatPanel />); const user = await send(); activity();
  vi.mocked(invoke).mockRejectedValueOnce("Cancel rejected");
  await user.click(screen.getByRole("button", { name: /Cancel/ }));
  await screen.findByText("Failed to cancel: Cancel rejected");
  expect(screen.getByText("Running")).toBeTruthy();
  terminal();
});

it("waits for terminal cancellation rather than treating acknowledgement as completion", async () => {
  render(<ChatPanel />); const user = await send(); activity();
  await user.click(screen.getByRole("button", { name: /Cancel/ }));
  expect(screen.getByText("Running")).toBeTruthy();
  terminal("cancelled");
  expect(screen.queryByText("Running")).toBeNull();
});

it("clears activity on the next send and ignores events from the prior request", async () => {
  render(<ChatPanel />); await send(); activity(); const oldId = requestId; terminal();
  await send();
  expect(screen.queryByRole("region", { name: "Tool activity" })).toBeNull();
  act(() => emit({ requestId: oldId, toolActivity: { id: "old", name: "read", status: "running", summary: "Old evidence" } }));
  expect(screen.queryByText("Old evidence")).toBeNull();
  terminal();
});

it.each(["new", "load", "delete"])("clears transient evidence on %s without reconstructing it from history", async (action) => {
  render(<ChatPanel />); const user = await send(); activity(); terminal();
  if (action === "new") {
    await user.click(screen.getByRole("button", { name: /New chat/ }));
  } else {
    await user.click(screen.getByRole("button", { name: /Conversations/ }));
    if (action === "load") {
      await user.click(screen.getByRole("button", { name: "Saved chat" }));
      await screen.findByText("Saved response");
    } else {
      await user.click(screen.getByRole("button", { name: "Delete conversation: Saved chat" }));
      await user.click(screen.getByRole("button", { name: "Delete permanently" }));
      await waitFor(() => expect(screen.queryByRole("alertdialog")).toBeNull());
    }
  }
  expect(screen.queryByRole("region", { name: "Tool activity" })).toBeNull();
});
