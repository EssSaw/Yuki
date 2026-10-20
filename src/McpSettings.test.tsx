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
    if (command === "get_mcp_executable_grants") { return []; }
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
  vi.mocked(invoke).mockImplementation(async (command) => command === "get_mcp_executable_grants" ? [] : command === "get_mcp_grants" ? [grant] : inventory);
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
  vi.mocked(invoke).mockImplementation(async (command) => command === "get_mcp_executable_grants" ? [] : command === "get_mcp_grants" ? [] : { ...inventory, candidates: [] });
  render(<McpSettings />);
  await screen.findByText(/No candidate endpoints found/);
  vi.mocked(invoke).mockRejectedValueOnce("local discovery failed");
  await userEvent.click(screen.getByRole("button", { name: "Discover local endpoints" }));
  await screen.findByRole("alert");
  vi.mocked(invoke).mockImplementation(async (command) => command === "get_mcp_executable_grants" ? [] : command === "get_mcp_grants" ? [] : { ...inventory, candidates: [{ ...inventory.candidates[0], reachability: { type: "unreachable", reason: "connection refused" } }] });
  await userEvent.click(screen.getByRole("button", { name: "Discover local endpoints" }));
  await screen.findByText("HTTP unavailable: connection refused");
});
it("ignores stale lifecycle responses and late unmounted results", async () => {
  let resolveOld!: (value: McpDiscoveryResult) => void;
  const old = new Promise<McpDiscoveryResult>((resolve) => { resolveOld = resolve; });
  let lists = 0;
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "get_mcp_executable_grants") { return []; }
    if (command === "get_mcp_grants") { return []; }
    lists++;
    return lists === 1 ? old : inventory;
  });
  const view = render(<StrictMode><McpSettings /></StrictMode>);
  await screen.findByText("Local candidate");
  await act(async () => { resolveOld({ ...inventory, candidates: [] }); });
  expect(screen.queryByText(/No candidate endpoints found/)).toBeNull();
  let finish!: (value: McpDiscoveryResult) => void;
  vi.mocked(invoke).mockImplementation(async (command) => command === "get_mcp_executable_grants" ? [] : command === "get_mcp_grants" ? [] : new Promise<McpDiscoveryResult>((resolve) => { finish = resolve; }));
  await userEvent.click(screen.getByRole("button", { name: "Discover local endpoints" }));
  view.unmount();
  await act(async () => { finish(inventory); });
  await waitFor(() => expect(screen.queryByText("Local candidate")).toBeNull());
});

const connected = { sessionId: "session-1", endpointUrl: endpoint, protocolVersion: "2024-11-05" };
const bound = { ...connected, boundInstanceId: "Example@abc", isPlaying: false, isPaused: false, isCompiling: false, projectVerification: { type: "verified", canonical_root: "E:\\Example", matched_approved_root: "E:\\Example", provider_sharing_enabled: false }, providerSharingEligible: false };
function sessionMock() {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "get_mcp_executable_grants") { return []; }
    if (command === "get_mcp_grants") { return [grant]; }
    if (command === "connect_approved_mcp_endpoint") { return connected; }
    if (command === "list_connected_unity_instances") { return [{ id: "Example@abc", name: "Example", hash: "abc", isActive: true }]; }
    if (command === "bind_unity_instance" || command === "get_bound_unity_state") { return bound; }
    if (command === "disconnect_mcp_session") { return { success: true }; }
    if (command === "revoke_mcp_endpoint") { return { canonicalUrl: endpoint, isApproved: false, approvedEndpoints: [] }; }
    return inventory;
  });
}
async function openSession() {
  await userEvent.click(await screen.findByRole("button", { name: "Connect approved endpoint" }));
  await screen.findByRole("option", { name: "Example@abc (server active)" });
}
async function bindSession() {
  await userEvent.selectOptions(screen.getByLabelText("Exact Unity instance"), "Example@abc");
  await userEvent.click(screen.getByRole("button", { name: "Bind selected editor" }));
  await screen.findByText("Verified approved project");
}
it("connects only explicitly, requires exact selection, binds and refreshes local state", async () => {
  sessionMock(); render(<McpSettings />);
  await screen.findByRole("button", { name: "Connect approved endpoint" });
  expect(invoke).not.toHaveBeenCalledWith("connect_approved_mcp_endpoint", expect.anything());
  await openSession();
  expect(screen.getByRole("button", { name: "Bind selected editor" })).toHaveProperty("disabled", true);
  expect(invoke).not.toHaveBeenCalledWith("bind_unity_instance", expect.anything());
  await bindSession();
  expect(invoke).toHaveBeenCalledWith("bind_unity_instance", { sessionId: "session-1", exactId: "Example@abc" });
  expect(screen.getAllByText("E:\\Example")).toHaveLength(2);
  await userEvent.click(screen.getByRole("button", { name: "Refresh editor state" }));
  await screen.findByText("Verified approved project");
  expect(invoke).toHaveBeenCalledWith("get_bound_unity_state", { sessionId: "session-1" });
  await userEvent.click(screen.getByRole("button", { name: "Disconnect" }));
  expect(screen.queryByText("Verified approved project")).toBeNull();
  expect(invoke).toHaveBeenCalledWith("disconnect_mcp_session", { sessionId: "session-1" });
});
it("reports connect failure without a binding", async () => {
  sessionMock(); render(<McpSettings />);
  await screen.findByRole("button", { name: "Connect approved endpoint" });
  vi.mocked(invoke).mockRejectedValueOnce("initialize rejected");
  await userEvent.click(screen.getByRole("button", { name: "Connect approved endpoint" }));
  expect((await screen.findByRole("alert")).textContent).toContain("initialize rejected");
  expect(screen.queryByLabelText("Exact Unity instance")).toBeNull();
});
it("clears binding on native session 404 without reconnecting automatically", async () => {
  sessionMock(); render(<McpSettings />); await openSession(); await bindSession();
  vi.mocked(invoke).mockRejectedValueOnce("Session expired: HTTP 404");
  await userEvent.click(screen.getByRole("button", { name: "Refresh editor state" }));
  expect((await screen.findByRole("alert")).textContent).toContain("404");
  expect(screen.queryByText("Verified approved project")).toBeNull();
  expect(vi.mocked(invoke).mock.calls.filter(([name]) => name === "connect_approved_mcp_endpoint")).toHaveLength(1);
});
it("revocation clears binding even when saving the grant fails", async () => {
  sessionMock(); render(<McpSettings />); await openSession(); await bindSession();
  await userEvent.click(screen.getByRole("button", { name: "Revoke saved grant" }));
  const original = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args) => command === "revoke_mcp_endpoint" ? Promise.reject("disk busy") : original(command, args));
  await userEvent.click(screen.getByRole("button", { name: "Confirm revoke" }));
  await screen.findByRole("alert");
  expect(screen.queryByText("Verified approved project")).toBeNull();
  expect(screen.getByText("Approved")).toBeTruthy();
});
it("ignores a late bind after disconnect", async () => {
  sessionMock(); render(<McpSettings />); await openSession();
  let finish!: (value: typeof bound) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  await userEvent.selectOptions(screen.getByLabelText("Exact Unity instance"), "Example@abc");
  await userEvent.click(screen.getByRole("button", { name: "Bind selected editor" }));
  await userEvent.click(screen.getByRole("button", { name: "Disconnect" }));
  await act(async () => { finish(bound); });
  expect(screen.queryByText("Verified approved project")).toBeNull();
});
it("disconnects a connection that completes after unmount", async () => {
  sessionMock(); const view = render(<McpSettings />);
  await screen.findByRole("button", { name: "Connect approved endpoint" });
  let finish!: (value: typeof connected) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  await userEvent.click(screen.getByRole("button", { name: "Connect approved endpoint" }));
  view.unmount();
  await act(async () => { finish(connected); });
  expect(invoke).toHaveBeenCalledWith("disconnect_mcp_session", { sessionId: "session-1" });
  expect(invoke).not.toHaveBeenCalledWith("list_connected_unity_instances", expect.anything());
});
it("closes an established session on unmount", async () => {
  sessionMock(); const view = render(<StrictMode><McpSettings /></StrictMode>);
  await openSession(); view.unmount();
  expect(invoke).toHaveBeenCalledWith("disconnect_mcp_session", { sessionId: "session-1" });
});
it("displays native mismatch instead of inferring verification from the instance name", async () => {
  sessionMock(); render(<McpSettings />); await openSession();
  vi.mocked(invoke).mockResolvedValueOnce({ ...bound, projectVerification: { type: "mismatched", reported_root: "E:\\Other", reason: "Not an approved root" } });
  await userEvent.selectOptions(screen.getByLabelText("Exact Unity instance"), "Example@abc");
  await userEvent.click(screen.getByRole("button", { name: "Bind selected editor" }));
  await screen.findByText("Project mismatch");
  expect(screen.getByText("Not an approved root")).toBeTruthy();
  expect(screen.queryByText("Verified approved project")).toBeNull();
});
it("clears an unsuccessful bind and disconnects its session", async () => {
  sessionMock(); render(<McpSettings />); await openSession();
  vi.mocked(invoke).mockRejectedValueOnce("Exact instance not found");
  await userEvent.selectOptions(screen.getByLabelText("Exact Unity instance"), "Example@abc");
  await userEvent.click(screen.getByRole("button", { name: "Bind selected editor" }));
  expect((await screen.findByRole("alert")).textContent).toContain("Exact instance not found");
  expect(screen.queryByLabelText("Exact Unity instance")).toBeNull();
  expect(invoke).toHaveBeenCalledWith("disconnect_mcp_session", { sessionId: "session-1" });
});

const relayPath = "C:\\Fixture\\relay_win.exe";
const relayGrant = { canonicalPath: relayPath, approvedAt: "2026-10-20" };
const relayConnection = { sessionId: "stdio-fixture", canonicalPath: relayPath, protocolVersion: "2025-11-25", serverName: "Fixture relay", serverVersion: "0.1" };
function relayMock(saved = true) {
  vi.mocked(invoke).mockImplementation(async (command) => {
    if (command === "get_mcp_executable_grants") { return saved ? [relayGrant] : []; }
    if (command === "get_mcp_grants") { return []; }
    if (command === "approve_mcp_executable") { return { canonicalPath: relayPath, isApproved: true, approvedExecutables: [relayGrant] }; }
    if (command === "revoke_mcp_executable") { return { canonicalPath: relayPath, isApproved: false, approvedExecutables: [] }; }
    if (command === "connect_approved_mcp_stdio") { return relayConnection; }
    if (command === "disconnect_mcp_session") { return { success: true }; }
    if (command === "list_mcp_stdio_capabilities") { return { sessionId: relayConnection.sessionId, tools: [{ name: "fixture_tool" }], resources: [{ name: "fixture_resource" }], editorBindingStatus: { type: "pendingOrUndocumented", details: "Fixture: not bound" } }; }
    return inventory;
  });
}
it("requires explicit exact executable consent, permits cancel, and never launches on mount or approval", async () => {
  relayMock(false);
  render(<McpSettings />);
  await screen.findByText("Local candidate");
  expect(invoke).toHaveBeenCalledWith("get_mcp_executable_grants");
  expect(vi.mocked(invoke).mock.calls.some(([name]) => name.includes("connect_approved"))).toBe(false);
  const input = screen.getByLabelText("Relay executable absolute path");
  await userEvent.type(input, "relative.exe");
  expect(screen.getByRole("button", { name: "Review executable approval" })).toHaveProperty("disabled", true);
  await userEvent.clear(input);
  await userEvent.type(input, relayPath);
  await userEvent.click(screen.getByRole("button", { name: "Review executable approval" }));
  expect(screen.getByRole("group").textContent).toContain(`"${relayPath}" --mcp`);
  expect(screen.getByRole("group").textContent).toContain("starts this local process");
  expect(screen.getByRole("group").textContent).toContain("Pending Connections");
  await userEvent.click(screen.getByRole("button", { name: "Cancel executable review" }));
  expect(invoke).not.toHaveBeenCalledWith("approve_mcp_executable", expect.anything());
  await userEvent.click(screen.getByRole("button", { name: "Review executable approval" }));
  vi.mocked(invoke).mockRejectedValueOnce("grant refused");
  await userEvent.click(screen.getByRole("button", { name: "Confirm executable approval" }));
  expect((await screen.findByRole("alert")).textContent).toContain("grant refused");
  expect(screen.queryByRole("button", { name: "Connect approved relay" })).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Confirm executable approval" }));
  await screen.findByRole("button", { name: "Connect approved relay" });
  expect(invoke).toHaveBeenCalledWith("approve_mcp_executable", { path: relayPath });
  expect(invoke).not.toHaveBeenCalledWith("connect_approved_mcp_stdio", expect.anything());
});
it("explicitly connects with fixed native args and inspects untrusted metadata without execution", async () => {
  relayMock(); render(<McpSettings />);
  await userEvent.click(await screen.findByRole("button", { name: "Connect approved relay" }));
  await screen.findByText(/Server-reported: Fixture relay/);
  expect(invoke).toHaveBeenCalledWith("connect_approved_mcp_stdio", { executablePath: relayPath });
  expect(invoke).not.toHaveBeenCalledWith("list_mcp_stdio_capabilities", expect.anything());
  await userEvent.click(screen.getByRole("button", { name: "Inspect relay capabilities" }));
  await screen.findByText("fixture_tool");
  expect(screen.getByText("Tools (1)")).toBeTruthy();
  expect(screen.getByText("Resources (1)")).toBeTruthy();
  expect(screen.getByText(/Binding pending or undocumented: Fixture/)).toBeTruthy();
  expect(invoke).toHaveBeenCalledWith("list_mcp_stdio_capabilities", { sessionId: "stdio-fixture" });
  expect(screen.queryByRole("button", { name: /execute/i })).toBeNull();
  await userEvent.click(screen.getByRole("button", { name: "Disconnect relay" }));
  await waitFor(() => expect(screen.queryByText("fixture_tool")).toBeNull());
  expect(invoke).toHaveBeenCalledWith("disconnect_mcp_session", { sessionId: "stdio-fixture" });
});
it("clears relay session on revoke failure while preserving saved permission", async () => {
  relayMock(); render(<McpSettings />);
  await userEvent.click(await screen.findByRole("button", { name: "Connect approved relay" }));
  await screen.findByRole("button", { name: "Inspect relay capabilities" });
  await userEvent.click(screen.getByRole("button", { name: "Revoke executable grant" }));
  const normal = vi.mocked(invoke).getMockImplementation()!;
  vi.mocked(invoke).mockImplementation((command, args, options) => command === "revoke_mcp_executable" ? Promise.reject("disk refused") : normal(command, args, options));
  await userEvent.click(screen.getByRole("button", { name: "Confirm executable revoke" }));
  await screen.findByRole("alert");
  expect(screen.queryByRole("button", { name: "Inspect relay capabilities" })).toBeNull();
  expect(screen.getByRole("button", { name: "Connect approved relay" })).toBeTruthy();
  expect(invoke).toHaveBeenCalledWith("revoke_mcp_executable", { path: relayPath });
  vi.mocked(invoke).mockImplementation(normal);
  await userEvent.click(screen.getByRole("button", { name: "Confirm executable revoke" }));
  await waitFor(() => expect(screen.queryByRole("button", { name: "Connect approved relay" })).toBeNull());
});
it("reports relay connection and capability failures without stale session data", async () => {
  relayMock(); render(<McpSettings />);
  await screen.findByRole("button", { name: "Connect approved relay" });
  vi.mocked(invoke).mockRejectedValueOnce("launch refused");
  await userEvent.click(screen.getByRole("button", { name: "Connect approved relay" }));
  expect((await screen.findByRole("alert")).textContent).toContain("launch refused");
  await userEvent.click(screen.getByRole("button", { name: "Connect approved relay" }));
  await screen.findByRole("button", { name: "Inspect relay capabilities" });
  vi.mocked(invoke).mockRejectedValueOnce("inventory timeout");
  await userEvent.click(screen.getByRole("button", { name: "Inspect relay capabilities" }));
  expect((await screen.findByRole("alert")).textContent).toContain("inventory timeout");
  expect(screen.queryByRole("button", { name: "Inspect relay capabilities" })).toBeNull();
});
it.each([false, true])("closes late relay connection after cancellation/unmount %s", async (unmount) => {
  relayMock();
  const view = render(<McpSettings />);
  await screen.findByRole("button", { name: "Connect approved relay" });
  let finish!: (value: typeof relayConnection) => void;
  vi.mocked(invoke).mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
  await userEvent.click(screen.getByRole("button", { name: "Connect approved relay" }));
  if (unmount) { view.unmount(); } else { await userEvent.click(screen.getByRole("button", { name: "Disconnect relay" })); }
  await act(async () => { finish(relayConnection); });
  expect(invoke).toHaveBeenCalledWith("disconnect_mcp_session", { sessionId: "stdio-fixture" });
  expect(screen.queryByRole("button", { name: "Inspect relay capabilities" })).toBeNull();
});
it("keeps official relay controls inert in browser preview", () => {
  vi.mocked(isTauri).mockReturnValue(false); render(<McpSettings />);
  expect(screen.getByLabelText("Relay executable absolute path")).toHaveProperty("disabled", true);
  expect(screen.getByRole("button", { name: "Review executable approval" })).toHaveProperty("disabled", true);
  expect(invoke).not.toHaveBeenCalled();
});
