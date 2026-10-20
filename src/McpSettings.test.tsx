// @vitest-environment jsdom
import { act, cleanup, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { StrictMode } from "react";
import McpSettings, { type McpDiscoveryResult } from "./McpSettings";
vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(), isTauri: vi.fn() }));
const endpoint = "http://localhost:8080/mcp";
const grant = { canonicalUrl: endpoint, approvedAt: "2026-10-20" };
const inventory: McpDiscoveryResult = { scannedRoots: [], configsInspected: [], candidates: [{ id: "default", serverName: "Local candidate", endpointUrl: endpoint, transport: "http", source: { type: "documentedDefault", default_url: endpoint }, projectRelation: { type: "explicitUnverified", details: "No verified project binding" }, isApproved: false, supported: true, reachability: { type: "notProbed" }, identityStatus: { type: "unverified", details: "No handshake" } }] };
beforeEach(() => {
  vi.mocked(isTauri).mockReturnValue(true);
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "get_mcp_grants") { return []; }
    if (command === "discover_mcp_candidates") { return { ...inventory, candidates: [{ ...inventory.candidates[0], reachability: { type: "reachable", status_code: 405 } }] }; }
    return inventory;
  });
});
afterEach(() => { cleanup(); vi.resetAllMocks(); });

it("lists without probes and discovers HTTP status only on explicit action", async () => {
  render(<McpSettings />);
  await screen.findByText("HTTP availability not checked");
  expect(invoke).not.toHaveBeenCalledWith("discover_mcp_candidates");
  await userEvent.click(screen.getByRole("button", { name: "Discover local endpoints" }));
  await screen.findByText(/HTTP 405 received/);
  expect(invoke).toHaveBeenCalledWith("discover_mcp_candidates");
  expect(screen.getByText(/UNVERIFIED - No handshake/)).toBeTruthy();
});
it("cancels approval without a mutation then uses only returned confirmed state", async () => {
  render(<McpSettings />);
  await userEvent.click(await screen.findByRole("button", { name: "Review endpoint approval" }));
  expect(screen.getByRole("group").textContent).toContain(endpoint);
  await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
  expect(vi.mocked(invoke).mock.calls.some(([name]) => name === "approve_mcp_endpoint")).toBe(false);
  await userEvent.click(screen.getByRole("button", { name: "Review endpoint approval" }));
  vi.mocked(invoke).mockResolvedValueOnce({ canonicalUrl: endpoint, isApproved: true, approvedEndpoints: [grant] });
  await userEvent.click(screen.getByRole("button", { name: "Confirm approval" }));
  await screen.findByText("Approved");
  expect(invoke).toHaveBeenCalledWith("approve_mcp_endpoint", { endpoint });
});
it("preserves not-approved state on failed approval and exposes dismissible error", async () => {
  render(<McpSettings />);
  await userEvent.click(await screen.findByRole("button", { name: "Review endpoint approval" }));
  vi.mocked(invoke).mockRejectedValueOnce("write refused");
  await userEvent.click(screen.getByRole("button", { name: "Confirm approval" }));
  expect((await screen.findByRole("alert")).textContent).toContain("write refused");
  expect(screen.getByText("Not approved")).toBeTruthy();
  await userEvent.click(screen.getByRole("button", { name: "Dismiss error" }));
  expect(screen.queryByRole("alert")).toBeNull();
});
it("preserves saved grants on revoke failure, then revokes from confirmed response", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => command === "get_mcp_grants" ? [grant] : inventory);
  render(<McpSettings />);
  await userEvent.click(await screen.findByRole("button", { name: "Revoke saved grant" }));
  vi.mocked(invoke).mockRejectedValueOnce("disk busy");
  await userEvent.click(screen.getByRole("button", { name: "Confirm revoke" }));
  await screen.findByRole("alert");
  expect(screen.getByText("Approved")).toBeTruthy();
  vi.mocked(invoke).mockResolvedValueOnce({ canonicalUrl: endpoint, isApproved: false, approvedEndpoints: [] });
  await userEvent.click(screen.getByRole("button", { name: "Confirm revoke" }));
  await screen.findByText("Not approved");
  expect(screen.queryByRole("button", { name: "Revoke saved grant" })).toBeNull();
});
it("keeps browser preview inert", () => {
  vi.mocked(isTauri).mockReturnValue(false);
  render(<McpSettings />);
  expect(screen.getByRole("button", { name: "Discover local endpoints" })).toHaveProperty("disabled", true);
  expect(invoke).not.toHaveBeenCalled();
});
it("renders empty results and failed/offline discovery honestly", async () => {
  vi.mocked(invoke).mockImplementation(async (command) => command === "get_mcp_grants" ? [] : { ...inventory, candidates: [] });
  render(<McpSettings />);
  await screen.findByText(/No candidate endpoints found/);
  vi.mocked(invoke).mockRejectedValueOnce("local discovery failed");
  await userEvent.click(screen.getByRole("button", { name: "Discover local endpoints" }));
  await screen.findByRole("alert");
  vi.mocked(invoke).mockImplementation(async (command) => command === "get_mcp_grants" ? [] : { ...inventory, candidates: [{ ...inventory.candidates[0], reachability: { type: "unreachable", reason: "connection refused" } }] });
  await userEvent.click(screen.getByRole("button", { name: "Discover local endpoints" }));
  await screen.findByText("HTTP unavailable: connection refused");
});
it("ignores stale lifecycle responses and late unmounted results", async () => {
  let resolveOld!: (value: McpDiscoveryResult) => void;
  const old = new Promise<McpDiscoveryResult>((resolve) => { resolveOld = resolve; });
  let lists = 0;
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "get_mcp_grants") { return []; }
    lists++;
    return lists === 1 ? old : inventory;
  });
  const view = render(<StrictMode><McpSettings /></StrictMode>);
  await screen.findByText("Local candidate");
  await act(async () => { resolveOld({ ...inventory, candidates: [] }); });
  expect(screen.queryByText(/No candidate endpoints found/)).toBeNull();
  let finish!: (value: McpDiscoveryResult) => void;
  vi.mocked(invoke).mockImplementation(async (command) => command === "get_mcp_grants" ? [] : new Promise<McpDiscoveryResult>((resolve) => { finish = resolve; }));
  await userEvent.click(screen.getByRole("button", { name: "Discover local endpoints" }));
  view.unmount();
  await act(async () => { finish(inventory); });
  await waitFor(() => expect(screen.queryByText("Local candidate")).toBeNull());
});
