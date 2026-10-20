// @vitest-environment jsdom
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import ChatPanel from "./ChatPanel";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: () => true }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(async () => vi.fn()) }));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
function setup() {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "get_chat_settings") return { model: "test", baseUrl: "http://localhost", hasApiKey: false };
    if (command === "list_conversations") return [];
    if (command === "get_workspace_settings") return { projectRoots: ["C:\\fixture"] };
    if (command === "get_agent_permission_snapshot") return { generation: 1, writeRoots: [], commandTemplates: [], providerSharingEnabled: false, isTaskRootApproved: true };
    return null;
  });
  render(<ChatPanel />);
  return userEvent.setup();
}
it("passes mode and private task metadata separately from message content before first send", async () => {
  const user = setup();
  await user.click(screen.getByText("Mode & permissions"));
  await user.selectOptions(screen.getByLabelText("Conversation mode"), "agent");
  await user.selectOptions(screen.getByLabelText("Task root"), "C:\\fixture");
  await user.type(screen.getByLabelText("Task description"), "Inspect");
  await user.click(screen.getByText("Apply mode & task"));
  await user.type(screen.getByRole("textbox", { name: "Message Yuki" }), "Hello");
  await user.keyboard("{Enter}");
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_chat", expect.objectContaining({ content: "Hello", mode: "agent", taskRoot: "C:\\fixture", taskDescription: "Inspect" })));
  expect((screen.getByLabelText("Conversation mode") as HTMLSelectElement).closest("fieldset")?.disabled).toBe(true);
});
it("new chat clears an unapplied Agent draft and sends default Chat", async () => {
  const user = setup();
  await user.click(screen.getByText("Mode & permissions"));
  await user.selectOptions(screen.getByLabelText("Conversation mode"), "agent");
  await user.click(screen.getByRole("button", { name: "+ New chat" }));
  await user.click(screen.getByText("Mode & permissions"));
  expect((screen.getByLabelText("Conversation mode") as HTMLSelectElement).value).toBe("chat");
  await user.type(screen.getByRole("textbox", { name: "Message Yuki" }), "Hello");
  await user.keyboard("{Enter}");
  await waitFor(() => expect(invoke).toHaveBeenCalledWith("start_chat", expect.objectContaining({ content: "Hello", mode: "chat", taskRoot: null, taskDescription: null })));
});
