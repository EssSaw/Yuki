// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
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
