// @vitest-environment jsdom
import { cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import AgentControls, { CHAT_CONTEXT } from "./AgentControls";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const root = "C:\\project";
const snapshot = { generation: 7, isTaskRootApproved: true, providerSharingEnabled: false, writeRoots: [], commandTemplates: [], executionAvailable: false, unsandboxedDisclosure: "Execution disabled." };
beforeEach(() => vi.mocked(invoke).mockImplementation(async (command) => {
  if (command === "get_workspace_settings") return { projectRoots: [root] };
  if (command === "get_agent_permission_snapshot") return snapshot;
  return null;
}));
afterEach(() => { cleanup(); vi.resetAllMocks(); });
it("defaults to Chat and requires explicit task selection and apply", async () => {
  const onChange = vi.fn(); const user = userEvent.setup();
  render(<AgentControls desktop disabled={false} conversationId={null} value={CHAT_CONTEXT} onChange={onChange} onBusy={vi.fn()} />);
  await user.click(screen.getByText("Mode & permissions"));
  await user.selectOptions(screen.getByLabelText("Conversation mode"), "agent");
  expect((screen.getByText("Apply mode & task") as HTMLButtonElement).disabled).toBe(true);
  await user.selectOptions(screen.getByLabelText("Task root"), root);
  await user.type(screen.getByLabelText("Task description"), "Review UI");
  await user.click(screen.getByText("Apply mode & task"));
  expect(onChange).toHaveBeenCalledWith({ mode: "agent", taskRoot: root, taskDescription: "Review UI" });
  expect(invoke).not.toHaveBeenCalledWith("approve_agent_write_root", expect.anything());
});
it("preserves existing mode if native persistence fails", async () => {
  vi.mocked(invoke).mockRejectedValueOnce("unavailable");
  const onChange = vi.fn(); const user = userEvent.setup();
  render(<AgentControls desktop disabled={false} conversationId="saved" value={{ mode: "agent", taskRoot: root }} onChange={onChange} onBusy={vi.fn()} />);
  await user.click(screen.getByText("Mode & permissions"));
  await user.selectOptions(screen.getByLabelText("Conversation mode"), "chat");
  vi.mocked(invoke).mockRejectedValueOnce("busy");
  await user.click(screen.getByText("Apply mode & task"));
  await screen.findByText(/Mode was not changed/);
  expect(onChange).not.toHaveBeenCalled();
});
it("reviews exact root and sends expected generation; stale failure refreshes without retry", async () => {
  const user = userEvent.setup();
  render(<AgentControls desktop disabled={false} conversationId={null} value={{ mode: "agent", taskRoot: root }} onChange={vi.fn()} onBusy={vi.fn()} />);
  await user.click(screen.getByText("Mode & permissions"));
  await user.click(await screen.findByText("Review write-root approval"));
  vi.mocked(invoke).mockRejectedValueOnce("stale generation");
  await user.click(screen.getByText("Confirm approval"));
  await screen.findByText(/Permission was not changed/);
  expect(invoke).toHaveBeenCalledWith("approve_agent_write_root", { projectRoot: root, path: root, expectedGeneration: 7 });
  await waitFor(() => expect(screen.queryByText("Confirm approval")).toBeNull());
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "approve_agent_write_root")).toHaveLength(1);
});
it("locks edits during active requests and makes browser preview inert", async () => {
  const props = { conversationId: null, value: CHAT_CONTEXT, onChange: vi.fn(), onBusy: vi.fn() };
  const { rerender } = render(<AgentControls {...props} desktop disabled />);
  await userEvent.click(screen.getByText("Mode & permissions"));
  expect((screen.getByLabelText("Conversation mode") as HTMLSelectElement).closest("fieldset")?.disabled).toBe(true);
  vi.mocked(invoke).mockClear();
  rerender(<AgentControls {...props} desktop={false} disabled={false} />);
  expect(invoke).not.toHaveBeenCalled();
  expect((screen.getByLabelText("Conversation mode") as HTMLSelectElement).closest("fieldset")?.disabled).toBe(true);
});
it("ignores snapshots from a previous conversation", async () => {
  let resolve!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "get_workspace_settings") return { projectRoots: [root] };
    if ((args as { conversationId?: string } | undefined)?.conversationId === "old") return new Promise((done) => { resolve = done; });
    return snapshot;
  });
  const props = { desktop: true, disabled: false, value: CHAT_CONTEXT, onChange: vi.fn(), onBusy: vi.fn() };
  const { rerender } = render(<AgentControls {...props} conversationId="old" />);
  rerender(<AgentControls {...props} conversationId="new" />);
  await userEvent.click(screen.getByText("Mode & permissions"));
  await screen.findByText(/Permission revision 7/);
  resolve({ ...snapshot, generation: 99 });
  await waitFor(() => expect(screen.queryByText(/Permission revision 99/)).toBeNull());
});
