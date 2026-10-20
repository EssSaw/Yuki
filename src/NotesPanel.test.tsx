// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke, isTauri } from "@tauri-apps/api/core";
import NotesPanel, { type NotesGuard } from "./NotesPanel";

const windowMock = vi.hoisted(() => ({ onCloseRequested: vi.fn(), close: vi.fn(), stop: vi.fn() }));
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
vi.mock("@tauri-apps/api/window", () => ({ getCurrentWindow: () => windowMock }));
const first = { schemaVersion: 1, id: "one", title: "First note", body: "# First", createdAt: "now", updatedAt: "now", versionToken: "v1" };
const second = { ...first, id: "two", title: "Second note" };
let closeHandler: (event: { preventDefault: () => void }) => void;
let guard: { current: NotesGuard };
beforeEach(() => {
  vi.clearAllMocks();
  guard = { current: null };
  vi.mocked(isTauri).mockReturnValue(true);
  windowMock.onCloseRequested.mockImplementation(async (handler) => { closeHandler = handler; return windowMock.stop; });
  windowMock.close.mockResolvedValue(undefined);
  vi.mocked(invoke).mockImplementation(async (command, args) => {
    if (command === "list_notes") { return [first, second]; }
    if (command === "load_note") { return (args as { id: string }).id === "one" ? first : second; }
    if (command === "save_note" || command === "create_note") { return { ...first, ...args as object, versionToken: "v2" }; }
    throw new Error("Unexpected command");
  });
});
afterEach(cleanup);
async function open() {
  const user = userEvent.setup();
  render(<NotesPanel guard={guard} />);
  await waitFor(() => expect((screen.getByRole("button", { name: "New note" }) as HTMLButtonElement).disabled).toBe(false));
  await user.click(await screen.findByRole("button", { name: "First note" }));
  await screen.findByRole("textbox", { name: "Markdown body" });
  return user;
}
it("opens, renames and explicitly saves using the loaded version", async () => {
  const user = await open();
  await user.clear(screen.getByLabelText("Title"));
  await user.type(screen.getByLabelText("Title"), "Renamed");
  expect(screen.getByText("Unsaved")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByText("Saved");
  expect(invoke).toHaveBeenCalledWith("save_note", { id: "one", title: "Renamed", body: "# First", expectedVersion: "v1" });
});
it.each(["Conflict: modified externally", "disk unavailable"])("keeps the draft and stays after failed navigation save: %s", async (failure) => {
  const user = await open();
  await user.type(screen.getByLabelText("Markdown body"), " draft");
  vi.mocked(invoke).mockRejectedValueOnce(failure);
  const leave = vi.fn();
  act(() => guard.current?.(leave));
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() => expect(screen.getByText("Failed")).toBeTruthy());
  expect(leave).not.toHaveBeenCalled();
  expect((screen.getByLabelText("Markdown body") as HTMLTextAreaElement).value).toBe("# First draft");
  expect(screen.getByRole("alertdialog")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("alertdialog")).toBeNull();
});
it("cancels switching notes, then discards and opens the requested note", async () => {
  const user = await open();
  await user.type(screen.getByLabelText("Title"), " edited");
  await user.click(screen.getByRole("button", { name: "Second note" }));
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe("First note edited");
  await user.click(screen.getByRole("button", { name: "Second note" }));
  await user.click(screen.getByRole("button", { name: "Discard" }));
  await waitFor(() => expect((screen.getByLabelText("Title") as HTMLInputElement).value).toBe("Second note"));
});
it("creates only on Save, keeps a failed new draft, and retries with Ctrl+S", async () => {
  const user = await open();
  await user.click(screen.getByRole("button", { name: "New note" }));
  await user.type(screen.getByLabelText("Markdown body"), "new draft");
  expect(invoke).not.toHaveBeenCalledWith("create_note", expect.anything());
  vi.mocked(invoke).mockRejectedValueOnce("write failed");
  await user.click(screen.getByRole("button", { name: "Save" }));
  await screen.findByText("Failed");
  expect((screen.getByLabelText("Markdown body") as HTMLTextAreaElement).value).toBe("new draft");
  fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  await screen.findByText("Saved");
  expect(invoke).toHaveBeenLastCalledWith("create_note", { title: "Untitled note", body: "new draft" });
});
it("blocks oversize titles without truncating the buffer", async () => {
  const user = await open();
  fireEvent.change(screen.getByLabelText("Title"), { target: { value: "x".repeat(201) } });
  await user.click(screen.getByRole("button", { name: "Save" }));
  expect(screen.getByText("Failed")).toBeTruthy();
  expect((screen.getByLabelText("Title") as HTMLInputElement).value.length).toBe(201);
  expect(invoke).not.toHaveBeenCalledWith("save_note", expect.anything());
});
it("native close cancels, then saves before closing and allows the resulting close event", async () => {
  const user = await open();
  await user.type(screen.getByLabelText("Title"), " edit");
  const preventDefault = vi.fn();
  act(() => closeHandler({ preventDefault }));
  expect(preventDefault).toHaveBeenCalled();
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(windowMock.close).not.toHaveBeenCalled();
  act(() => closeHandler({ preventDefault }));
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() => expect(windowMock.close).toHaveBeenCalledTimes(1));
  expect(windowMock.stop).toHaveBeenCalledTimes(1);
});
it("discards before native close", async () => {
  const user = await open();
  await user.type(screen.getByLabelText("Title"), " edit");
  act(() => closeHandler({ preventDefault: vi.fn() }));
  await user.click(screen.getByRole("button", { name: "Discard" }));
  expect(windowMock.close).toHaveBeenCalledTimes(1);
});
it("browser preview is inert including Ctrl+S", () => {
  vi.mocked(isTauri).mockReturnValue(false);
  render(<NotesPanel guard={guard} />);
  fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  expect(invoke).not.toHaveBeenCalled();
  expect(windowMock.onCloseRequested).not.toHaveBeenCalled();
  expect((screen.getByRole("button", { name: "New note" }) as HTMLButtonElement).disabled).toBe(true);
});
it("cleans up listeners even when registration resolves after unmount", async () => {
  let resolve!: (stop: () => void) => void;
  windowMock.onCloseRequested.mockReturnValue(new Promise((done) => { resolve = done; }));
  const { unmount } = render(<NotesPanel guard={guard} />);
  unmount();
  await act(async () => resolve(windowMock.stop));
  expect(windowMock.stop).toHaveBeenCalledTimes(1);
  expect(guard.current).toBeNull();
  vi.mocked(invoke).mockClear();
  fireEvent.keyDown(window, { key: "s", ctrlKey: true });
  expect(invoke).not.toHaveBeenCalled();
});
it("serializes saving with navigation and ignores a result after unmount", async () => {
  const user = userEvent.setup();
  const view = render(<NotesPanel guard={guard} />);
  await user.click(await screen.findByRole("button", { name: "First note" }));
  await user.type(await screen.findByLabelText("Title"), " edit");
  let resolve!: (note: typeof first) => void;
  vi.mocked(invoke).mockReturnValueOnce(new Promise((done) => { resolve = done; }));
  await user.click(screen.getByRole("button", { name: "Save" }));
  const leave = vi.fn();
  act(() => guard.current?.(leave));
  expect(leave).not.toHaveBeenCalled();
  expect((screen.getByLabelText("Title") as HTMLInputElement).disabled).toBe(true);
  view.unmount();
  await act(async () => resolve({ ...first, versionToken: "v2" }));
  expect(windowMock.stop).toHaveBeenCalled();
});

it("saves before section navigation and uses the acknowledged version for later saves", async () => {
  const user = await open();
  await user.type(screen.getByLabelText("Title"), " edit");
  const leave = vi.fn();
  act(() => guard.current?.(leave));
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await waitFor(() => expect(leave).toHaveBeenCalledTimes(1));
  await user.type(screen.getByLabelText("Title"), " again");
  await user.click(screen.getByRole("button", { name: "Save" }));
  expect(invoke).toHaveBeenLastCalledWith("save_note", expect.objectContaining({ expectedVersion: "v2", title: "First note edit again" }));
});

it("never closes after a conflicting save", async () => {
  const user = await open();
  await user.type(screen.getByLabelText("Markdown body"), " keep me");
  vi.mocked(invoke).mockRejectedValueOnce("Conflict: newer version");
  act(() => closeHandler({ preventDefault: vi.fn() }));
  await user.click(screen.getByRole("button", { name: "Save changes" }));
  await screen.findByText("Failed");
  expect(windowMock.close).not.toHaveBeenCalled();
  expect((screen.getByLabelText("Markdown body") as HTMLTextAreaElement).value).toBe("# First keep me");
});
