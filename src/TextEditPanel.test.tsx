// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke } from "@tauri-apps/api/core";
import TextEditPanel from "./TextEditPanel";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
const props = { desktop: true, disabled: false, conversationId: "conversation", taskRoot: "C:\\project", generation: 7, grants: [{ id: "grant", canonicalPath: "C:\\project" }], onBusy: vi.fn() };
const proposal = { id: "proposal", path: "C:\\project\\file.txt", conversationId: "conversation", grantId: "grant", generation: 7, originalBytes: 3, replacementBytes: 5, applyAvailable: true };
beforeEach(() => { vi.mocked(invoke).mockClear(); vi.mocked(invoke).mockImplementation(async (name) => {
  if (name === "prepare_local_text_edit") { return proposal; }
  if (name === "apply_local_text_edit") { return "change"; }
  if (name === "list_local_edit_recovery") { return []; }
  return undefined;
}); });
afterEach(() => { cleanup(); vi.resetAllMocks(); });
async function draft() {
  const user = userEvent.setup();
  await user.click(screen.getByText("Local text edit & recovery"));
  await user.type(screen.getByLabelText("Absolute file path"), proposal.path);
  await user.type(screen.getByLabelText("Complete replacement text"), "draft");
  return user;
}
it("does not invoke native operations on mount; browser is inert", async () => {
  const { rerender } = render(<TextEditPanel {...props} />);
  expect(invoke).not.toHaveBeenCalled();
  rerender(<TextEditPanel {...props} desktop={false} />);
  await userEvent.click(screen.getByText("Local text edit & recovery"));
  await userEvent.click(screen.getByText("Refresh recovery records"));
  expect(invoke).not.toHaveBeenCalled();
});
it("prepares explicitly, previews only replacement, applies and undoes with exact scope", async () => {
  render(<TextEditPanel {...props} />); const user = await draft();
  await user.click(screen.getByText("Prepare local edit"));
  expect(invoke).toHaveBeenCalledWith("prepare_local_text_edit", { conversationId: "conversation", path: proposal.path, generation: 7, replacement: "draft" });
  expect(screen.getByText(/not a diff/)).toBeTruthy();
  expect(invoke).not.toHaveBeenCalledWith("apply_local_text_edit", expect.anything());
  await user.click(screen.getByText("Apply replacement"));
  await user.click(screen.getByText("Undo last confirmed change"));
  expect(invoke).toHaveBeenCalledWith("undo_local_text_edit", { changeId: "change", generation: 7 });
  expect((screen.getByLabelText("Complete replacement text") as HTMLTextAreaElement).value).toBe("draft");
});
it.each(["prepare_local_text_edit", "apply_local_text_edit", "undo_local_text_edit"])("preserves draft when %s fails", async (failure) => {
  const normal = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation(async (name, args, options) => { if (name === failure) { throw new Error("conflict"); } return normal(name, args, options); });
  render(<TextEditPanel {...props} />); const user = await draft();
  await user.click(screen.getByText("Prepare local edit"));
  if (failure !== "prepare_local_text_edit") { await user.click(screen.getByText("Apply replacement")); }
  if (failure === "undo_local_text_edit") { await user.click(screen.getByText("Undo last confirmed change")); }
  expect((await screen.findByRole("alert")).textContent).toContain("Draft preserved");
  expect((screen.getByLabelText("Complete replacement text") as HTMLTextAreaElement).value).toBe("draft");
});
it("Stop acknowledges a request without claiming rollback and accepts confirmed late success", async () => {
  let finish!: (id: string) => void;
  const normal = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((name, args, options) => name === "apply_local_text_edit" ? new Promise((resolve) => { finish = resolve; }) : normal(name, args, options));
  render(<TextEditPanel {...props} />); const user = await draft();
  await user.click(screen.getByText("Prepare local edit")); await user.click(screen.getByText("Apply replacement"));
  await user.click(screen.getByText("Stop local operation"));
  expect(await screen.findByText(/not confirmation of cancellation or rollback/)).toBeTruthy();
  await act(async () => finish("change"));
  expect(screen.getByText(/Apply confirmed/)).toBeTruthy();
  expect((screen.getByText("Undo last confirmed change") as HTMLButtonElement).disabled).toBe(false);
});
it("resets scope and rejects stale prepare results", async () => {
  let finish!: (value: unknown) => void;
  vi.mocked(invoke).mockImplementation(() => new Promise((resolve) => { finish = resolve; }));
  const { rerender } = render(<TextEditPanel {...props} />); const user = await draft();
  await user.click(screen.getByText("Prepare local edit"));
  rerender(<TextEditPanel {...props} conversationId="another" />);
  await act(async () => finish(proposal));
  expect(screen.queryByText("Apply replacement")).toBeNull();
  expect((screen.getByLabelText("Complete replacement text") as HTMLTextAreaElement).value).toBe("");
});
it("blocks chat-active operations and displays unknown recovery as review-only", async () => {
  vi.mocked(invoke).mockResolvedValue([{ id: "record", state: "future", reviewRequired: false }]);
  const { rerender } = render(<TextEditPanel {...props} disabled />);
  await userEvent.click(screen.getByText("Local text edit & recovery"));
  await userEvent.click(screen.getByText("Refresh recovery records")); expect(invoke).not.toHaveBeenCalled();
  rerender(<TextEditPanel {...props} />);
  await userEvent.click(screen.getByText("Refresh recovery records"));
  await waitFor(() => expect(screen.getByText(/future - Requires review/)).toBeTruthy());
  expect(vi.mocked(invoke).mock.calls.map(([name]) => name)).toEqual(["list_local_edit_recovery"]);
});
