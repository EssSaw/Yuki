// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke, isTauri } from "@tauri-apps/api/core";
import WorkspacePanel from "./WorkspacePanel";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
afterEach(cleanup);
beforeEach(() => { vi.mocked(invoke).mockReset(); vi.mocked(isTauri).mockReturnValue(true); });

it("loads saved data root and reports picker failures without replacing it", async () => {
  vi.mocked(invoke).mockResolvedValueOnce({ dataRoot: "D:\\Yuki", projectRoots: [] }).mockRejectedValueOnce("Folder is not writable");
  const user = userEvent.setup();
  render(<WorkspacePanel section="Settings" />);
  await screen.findByText("D:\\Yuki");
  await user.click(screen.getByRole("button", { name: "Choose data folder" }));
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Folder is not writable");
  expect(screen.getByText("D:\\Yuki")).toBeTruthy();
});

it("lists approved project children and revokes access without a delete command", async () => {
  const root = "D:\\Game";
  vi.mocked(invoke).mockResolvedValueOnce({ dataRoot: null, projectRoots: [root] })
    .mockResolvedValueOnce([{ name: "Assets", relativePath: "Assets", isDirectory: true }])
    .mockResolvedValueOnce({ dataRoot: null, projectRoots: [] });
  const user = userEvent.setup();
  render(<WorkspacePanel section="Projects" />);
  await user.click(await screen.findByRole("button", { name: root }));
  expect(await screen.findByRole("button", { name: "Open folder: Assets" })).toBeTruthy();
  expect(invoke).toHaveBeenCalledWith("list_project_directory", { root, path: "" });
  await user.click(screen.getByRole("button", { name: `Revoke access to ${root}` }));
  expect(await screen.findByText("No projects approved yet.")).toBeTruthy();
  expect(invoke).toHaveBeenLastCalledWith("remove_project_root", { root });
  expect(screen.queryByRole("heading", { name: "Read-only explorer" })).toBeNull();
});

it("does not invoke native operations in browser preview", () => {
  vi.mocked(isTauri).mockReturnValue(false);
  render(<WorkspacePanel section="Settings" />);
  expect(screen.getByRole("button", { name: "Choose data folder" }).hasAttribute("disabled")).toBe(true);
  expect(invoke).not.toHaveBeenCalled();
});

const project = "D:\\Game";
const samplePath = "D:\\Other\\notes.txt";
const saved = { dataRoot: null, projectRoots: [project], providerSharingGrants: [] };
const firstChunk = { path: samplePath, text: "<script>plain text</script>", offset: 0, nextOffset: 65534, totalBytes: 90000, eof: false };
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

it("reads an explicit outside path and replaces chunks using native byte offsets", async () => {
  vi.mocked(invoke).mockResolvedValueOnce(saved).mockResolvedValueOnce(firstChunk)
    .mockResolvedValueOnce({ ...firstChunk, text: "second range", offset: 65534, nextOffset: 90000, eof: true })
    .mockResolvedValueOnce(firstChunk);
  const user = userEvent.setup();
  render(<WorkspacePanel section="Projects" />);
  await screen.findByRole("button", { name: project });
  await user.type(screen.getByLabelText("Absolute local file path"), samplePath);
  await user.click(screen.getByRole("button", { name: "Read locally" }));
  expect(await screen.findByText(firstChunk.text)).toBeTruthy();
  expect(document.querySelector("script")).toBeNull();
  expect(invoke).toHaveBeenLastCalledWith("read_local_file_chunk", { path: samplePath, offset: 0, chunkSize: 65536 });
  await user.click(screen.getByRole("button", { name: "Next range" }));
  expect(await screen.findByText("second range")).toBeTruthy();
  expect(screen.queryByText(firstChunk.text)).toBeNull();
  expect(invoke).toHaveBeenLastCalledWith("read_local_file_chunk", { path: samplePath, offset: 65534, chunkSize: 65536 });
  expect(screen.getByRole("button", { name: "Next range" }).hasAttribute("disabled")).toBe(true);
  expect(screen.getByText(/Byte range \[65534, 90000\) of 90000 bytes/)).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Back to start" }));
  expect(await screen.findByText(firstChunk.text)).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Close preview" }));
  expect(screen.queryByRole("region", { name: "Local-only file content" })).toBeNull();
});

it("connects explorer files to local preview and clears text on folder changes and revocation", async () => {
  vi.mocked(invoke).mockResolvedValueOnce(saved)
    .mockResolvedValueOnce([{ name: "notes.txt", relativePath: "notes.txt", isDirectory: false }, { name: "Assets", relativePath: "Assets", isDirectory: true }])
    .mockResolvedValueOnce({ ...firstChunk, path: `${project}\\notes.txt` })
    .mockResolvedValueOnce([]).mockResolvedValueOnce(firstChunk)
    .mockRejectedValueOnce("Revocation failed");
  const user = userEvent.setup();
  render(<WorkspacePanel section="Projects" />);
  await user.click(await screen.findByRole("button", { name: project }));
  await user.click(await screen.findByRole("button", { name: "Preview file: notes.txt" }));
  expect(await screen.findByText(firstChunk.text)).toBeTruthy();
  expect(invoke).toHaveBeenLastCalledWith("read_local_file_chunk", { path: `${project}\\notes.txt`, offset: 0, chunkSize: 65536 });
  await user.click(screen.getByRole("button", { name: "Open folder: Assets" }));
  expect(screen.queryByText(firstChunk.text)).toBeNull();
  await user.type(screen.getByLabelText("Absolute local file path"), samplePath);
  await user.click(screen.getByRole("button", { name: "Read locally" }));
  await screen.findByText(firstChunk.text);
  await user.click(screen.getByRole("button", { name: `Revoke access to ${project}` }));
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Revocation failed");
  expect(screen.queryByText(firstChunk.text)).toBeNull();
  expect(screen.getByRole("button", { name: project })).toBeTruthy();
});

it("clears the previous range when a read fails", async () => {
  vi.mocked(invoke).mockResolvedValueOnce(saved).mockResolvedValueOnce(firstChunk).mockRejectedValueOnce("Cannot read file");
  const user = userEvent.setup();
  render(<WorkspacePanel section="Projects" />);
  await screen.findByRole("button", { name: project });
  await user.type(screen.getByLabelText("Absolute local file path"), samplePath);
  await user.click(screen.getByRole("button", { name: "Read locally" }));
  await screen.findByText(firstChunk.text);
  await user.click(screen.getByRole("button", { name: "Next range" }));
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Cannot read file");
  expect(screen.queryByRole("region", { name: "Local-only file content" })).toBeNull();
});

it("ignores out-of-order reads and invalidates results on close, section change and unmount", async () => {
  const older = deferred<typeof firstChunk>();
  const closed = deferred<typeof firstChunk>();
  const switched = deferred<typeof firstChunk>();
  const removed = deferred<typeof firstChunk>();
  vi.mocked(invoke).mockResolvedValueOnce(saved).mockReturnValueOnce(older.promise)
    .mockResolvedValueOnce({ ...firstChunk, text: "newer content" })
    .mockReturnValueOnce(closed.promise).mockReturnValueOnce(switched.promise).mockReturnValueOnce(removed.promise);
  const user = userEvent.setup();
  const view = render(<WorkspacePanel section="Projects" />);
  await screen.findByRole("button", { name: project });
  await user.type(screen.getByLabelText("Absolute local file path"), samplePath);
  await user.click(screen.getByRole("button", { name: "Read locally" }));
  await user.click(screen.getByRole("button", { name: "Read locally" }));
  await screen.findByText("newer content");
  await act(async () => { older.resolve(firstChunk); });
  expect(screen.queryByText(firstChunk.text)).toBeNull();
  expect(screen.getByText("newer content")).toBeTruthy();
  await user.click(screen.getByRole("button", { name: "Read locally" }));
  await user.click(screen.getByRole("button", { name: "Close preview" }));
  await act(async () => { closed.resolve(firstChunk); });
  expect(screen.queryByText(firstChunk.text)).toBeNull();
  await user.click(screen.getByRole("button", { name: "Read locally" }));
  view.rerender(<WorkspacePanel section="Settings" />);
  await act(async () => { switched.resolve(firstChunk); });
  view.rerender(<WorkspacePanel section="Projects" />);
  expect(screen.queryByText(firstChunk.text)).toBeNull();
  await user.type(screen.getByLabelText("Absolute local file path"), samplePath);
  await user.click(screen.getByRole("button", { name: "Read locally" }));
  view.unmount();
  await act(async () => { removed.resolve(firstChunk); });
  expect(screen.queryByText(firstChunk.text)).toBeNull();
});

it("invalidates a pending read when changing projects, even if it later fails", async () => {
  const pending = deferred<typeof firstChunk>();
  vi.mocked(invoke).mockResolvedValueOnce(saved).mockReturnValueOnce(pending.promise).mockResolvedValueOnce([]);
  const user = userEvent.setup();
  render(<WorkspacePanel section="Projects" />);
  await screen.findByRole("button", { name: project });
  await user.type(screen.getByLabelText("Absolute local file path"), samplePath);
  await user.click(screen.getByRole("button", { name: "Read locally" }));
  await user.click(screen.getByRole("button", { name: project }));
  await act(async () => { pending.reject("Stale failure"); });
  expect(screen.queryByRole("alert")).toBeNull();
  expect(screen.queryByRole("region", { name: "Local-only file content" })).toBeNull();
});

it("requires scoped sharing consent, supports cancellation, and disables without confirmation", async () => {
  vi.mocked(invoke).mockResolvedValueOnce(saved)
    .mockResolvedValueOnce({ ...saved, providerSharingGrants: [project] }).mockResolvedValueOnce(saved);
  const user = userEvent.setup();
  render(<WorkspacePanel section="Projects" />);
  await user.click(await screen.findByRole("button", { name: `Enable provider sharing for ${project}` }));
  expect(screen.getByRole("group", { name: `Confirm provider sharing for ${project}` }).textContent).toContain("9router and its upstream model providers");
  expect(invoke).toHaveBeenCalledTimes(1);
  await user.click(screen.getByRole("button", { name: "Cancel" }));
  expect(screen.queryByRole("group")).toBeNull();
  expect(invoke).toHaveBeenCalledTimes(1);
  await user.click(screen.getByRole("button", { name: `Enable provider sharing for ${project}` }));
  await user.click(screen.getByRole("button", { name: "Confirm sharing permission" }));
  expect(await screen.findByRole("button", { name: `Disable provider sharing for ${project}` })).toBeTruthy();
  expect(invoke).toHaveBeenLastCalledWith("set_project_provider_sharing", { root: project, enabled: true });
  await user.click(screen.getByRole("button", { name: `Disable provider sharing for ${project}` }));
  expect(await screen.findByRole("button", { name: `Enable provider sharing for ${project}` })).toBeTruthy();
  expect(invoke).toHaveBeenLastCalledWith("set_project_provider_sharing", { root: project, enabled: false });
  expect(screen.queryByRole("group")).toBeNull();
});

it.each([false, true])("preserves sharing state after failed mutation (initial enabled: %s)", async (enabled) => {
  vi.mocked(invoke).mockResolvedValueOnce({ ...saved, providerSharingGrants: enabled ? [project] : [] }).mockRejectedValueOnce("Settings write failed");
  const user = userEvent.setup();
  render(<WorkspacePanel section="Projects" />);
  const name = `${enabled ? "Disable" : "Enable"} provider sharing for ${project}`;
  await user.click(await screen.findByRole("button", { name }));
  if (!enabled) { await user.click(screen.getByRole("button", { name: "Confirm sharing permission" })); }
  expect(await screen.findByRole("alert")).toHaveProperty("textContent", "Settings write failed");
  expect(screen.getByRole("button", { name }).hasAttribute("disabled")).toBe(false);
  expect(screen.getByRole("button", { name: project })).toBeTruthy();
});

it("isolates browser projects preview from native reading and sharing", async () => {
  vi.mocked(isTauri).mockReturnValue(false);
  const user = userEvent.setup();
  render(<WorkspacePanel section="Projects" />);
  await waitFor(() => expect(screen.getByLabelText("Absolute local file path").hasAttribute("disabled")).toBe(true));
  await user.click(screen.getByRole("button", { name: "Read locally" }));
  await user.click(screen.getByRole("button", { name: "Add project folder" }));
  expect(screen.getByText(/Browser preview has no filesystem access/)).toBeTruthy();
  expect(invoke).not.toHaveBeenCalled();
});
