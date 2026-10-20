import { useEffect, useRef, useState } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import "./McpSettings.css";

// Enum payload fields retain Rust snake_case; struct fields use camelCase.
export type McpCandidate = {
  id: string;
  serverName: string;
  endpointUrl?: string;
  transport: string;
  source: { type: "documentedDefault"; default_url: string } | { type: "projectConfig"; project_root: string; config_path: string; server_name: string };
  projectRelation: { type: "explicitUnverified"; details: string };
  isApproved: boolean;
  supported: boolean;
  reachability: { type: "notProbed" } | { type: "reachable"; status_code: number } | { type: "unreachable" | "unsupported"; reason: string };
  identityStatus: { type: "unverified"; details: string };
  warnings?: string[];
};
type Grant = { canonicalUrl: string; approvedAt: string };
export type McpDiscoveryResult = { candidates: McpCandidate[]; scannedRoots: string[]; configsInspected: string[]; warnings?: string[] };
type GrantResult = { canonicalUrl: string; isApproved: boolean; approvedEndpoints: Grant[] };
type Confirmation = { endpoint: string; revoke: boolean };

function validateInventory(result: McpDiscoveryResult, grants: Grant[]) {
  if (!result || !Array.isArray(result.candidates) || !Array.isArray(result.scannedRoots) || !Array.isArray(result.configsInspected) || !Array.isArray(grants)) {
    throw new Error("Local MCP inventory could not be read. Try discovery again.");
  }
}

function availability(candidate: McpCandidate) {
  const status = candidate.reachability;
  switch (status.type) {
    case "notProbed": return "HTTP availability not checked";
    case "reachable": return `HTTP ${status.status_code} received - listener responds; identity and authorization not verified`;
    case "unreachable": return `HTTP unavailable: ${status.reason}`;
    case "unsupported": return `Not probed: ${status.reason}`;
  }
}

export default function McpSettings() {
  const desktop = isTauri();
  const [result, setResult] = useState<McpDiscoveryResult | null>(null);
  const [grants, setGrants] = useState<Grant[]>([]);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [message, setMessage] = useState("");
  const [confirmation, setConfirmation] = useState<Confirmation | null>(null);
  const generation = useRef(0);
  const mutation = useRef(false);
  const confirmButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    const request = ++generation.current;
    if (!desktop) { return; }
    setBusy("Reading local candidates...");
    Promise.all([
      invoke<McpDiscoveryResult>("list_mcp_candidates"),
      invoke<Grant[]>("get_mcp_grants"),
    ]).then(([candidates, saved]) => {
      if (request !== generation.current) { return; }
      validateInventory(candidates, saved);
      setResult(candidates);
      setGrants(saved);
    }).catch((cause: unknown) => {
      if (request === generation.current) { setError(String(cause)); }
    }).finally(() => {
      if (request === generation.current) { setBusy(""); }
    });
    return () => { ++generation.current; };
  }, [desktop]);

  useEffect(() => {
    if (confirmation) { confirmButton.current?.focus(); }
  }, [confirmation]);

  async function discover() {
    if (!desktop || mutation.current || confirmation) { return; }
    const request = ++generation.current;
    setBusy("Checking local HTTP availability...");
    setError("");
    setMessage("");
    try {
      const [candidates, saved] = await Promise.all([
        invoke<McpDiscoveryResult>("discover_mcp_candidates"),
        invoke<Grant[]>("get_mcp_grants"),
      ]);
      if (request !== generation.current) { return; }
      validateInventory(candidates, saved);
      setResult(candidates);
      setGrants(saved);
      setMessage("Discovery complete. HTTP responses do not verify Unity identity.");
    } catch (cause) {
      if (request === generation.current) { setError(String(cause)); }
    } finally {
      if (request === generation.current) { setBusy(""); }
    }
  }

  async function confirm() {
    if (!desktop || !confirmation || mutation.current) { return; }
    const action = confirmation;
    const request = ++generation.current;
    mutation.current = true;
    setBusy("Saving endpoint permission...");
    setError("");
    setMessage("");
    try {
      const saved = await invoke<GrantResult>(action.revoke ? "revoke_mcp_endpoint" : "approve_mcp_endpoint", { endpoint: action.endpoint });
      if (request !== generation.current) { return; }
      setGrants(saved.approvedEndpoints);
      setConfirmation(null);
      setMessage(`${saved.canonicalUrl}: first-connection grant ${saved.isApproved ? "approved" : "revoked"}. No MCP connection was opened.`);
    } catch (cause) {
      if (request === generation.current) { setError(`Permission change failed. Displayed grants are unchanged. ${String(cause)}`); }
    } finally {
      mutation.current = false;
      if (request === generation.current) { setBusy(""); }
    }
  }

  function requestChange(endpoint: string, revoke: boolean) {
    setError("");
    setMessage("");
    setConfirmation({ endpoint, revoke });
  }

  return (
    <section className="mcp-settings" aria-labelledby="mcp-settings-title">
      <h2 id="mcp-settings-title">Local Unity MCP</h2>
      <p>Find local endpoint candidates, then choose which may receive a future first connection. Discovery checks HTTP availability only; it does not connect to or initialize Unity, select an editor, or access tools.</p>
      <p className="mcp-boundary">Endpoint approval does not enable provider sharing or send project content to a provider. Server identity and project-to-editor binding remain <strong>UNVERIFIED</strong>.</p>
      {!desktop && <p role="status">Browser preview is inert. Open the desktop app to list, discover, or approve local endpoints.</p>}
      <div className="mcp-actions">
        <button type="button" onClick={discover} disabled={!desktop || !!busy || !!confirmation}>Discover local endpoints</button>
        <span>Explicitly sends bounded HTTP GET probes to supported loopback candidates.</span>
      </div>
      {busy && <p role="status">{busy}</p>}
      {error && <div className="mcp-error" role="alert"><p>{error}</p><button type="button" onClick={() => setError("")}>Dismiss error</button></div>}
      {message && <p role="status">{message}</p>}
      {confirmation && (
        <div className="mcp-confirmation" role="group" aria-labelledby="mcp-confirm-title">
          <h3 id="mcp-confirm-title">{confirmation.revoke ? "Revoke" : "Approve"} this endpoint's first-connection grant?</h3>
          <code>{confirmation.endpoint}</code>
          <p>This changes only a saved local permission for this exact URL, port and path. It does not open an MCP session, initialize Unity, select an editor, grant tool access, or enable provider sharing.</p>
          <div className="mcp-actions">
            <button ref={confirmButton} type="button" disabled={!!busy} onClick={confirm}>{confirmation.revoke ? "Confirm revoke" : "Confirm approval"}</button>
            <button type="button" disabled={!!busy} onClick={() => setConfirmation(null)}>Cancel</button>
          </div>
        </div>
      )}
      {result && <>
        <h3>Candidate endpoints</h3>
        <p>{result.scannedRoots.length} approved project root(s) scanned; {result.configsInspected.length} configuration file(s) inspected.</p>
        {result.candidates.length === 0 && <p>No candidate endpoints found. Discovery can be retried when local configuration changes.</p>}
        <ul className="mcp-candidates">
          {result.candidates.map((candidate) => {
            const approved = grants.some((grant) => grant.canonicalUrl === candidate.endpointUrl);
            return <li key={candidate.id}>
              <h4>{candidate.serverName}</h4>
              <code>{candidate.endpointUrl || "No supported HTTP endpoint"}</code>
              <dl>
                <dt>Source</dt><dd>{candidate.source.type === "documentedDefault" ? `Documented default: ${candidate.source.default_url} (not project-specific)` : <>Project configuration: <span>{candidate.source.config_path}</span><br />Project root: <span>{candidate.source.project_root}</span><br />Configured name: {candidate.source.server_name}</>}</dd>
                <dt>Project relationship</dt><dd>UNVERIFIED - {candidate.projectRelation.details}</dd>
                <dt>Transport</dt><dd>{candidate.transport}{!candidate.supported && " (unsupported; no process will be launched)"}</dd>
                <dt>Availability</dt><dd>{availability(candidate)}</dd>
                <dt>Identity</dt><dd>UNVERIFIED - {candidate.identityStatus.details}</dd>
                <dt>First-connection grant</dt><dd>{approved ? "Approved" : "Not approved"}</dd>
              </dl>
              {candidate.warnings?.map((warning, i) => <p key={i}>{warning}</p>)}
              {candidate.supported && candidate.endpointUrl && <button type="button" disabled={!desktop || !!busy || !!confirmation} onClick={() => requestChange(candidate.endpointUrl!, approved)}>{approved ? "Revoke endpoint approval" : "Review endpoint approval"}</button>}
            </li>;
          })}
        </ul>
        {result.warnings?.map((warning, i) => <p key={i} className="mcp-warning">{warning}</p>)}
      </>}
      {grants.length > 0 && <>
        <h3>Saved first-connection grants</h3>
        <p>Saved permissions remain visible here even when a configuration no longer lists the endpoint.</p>
        <ul className="mcp-grants">{grants.map((grant) => <li key={grant.canonicalUrl}><code>{grant.canonicalUrl}</code><span>Approved at {grant.approvedAt}</span><button type="button" disabled={!desktop || !!busy || !!confirmation} onClick={() => requestChange(grant.canonicalUrl, true)}>Revoke saved grant</button></li>)}</ul>
      </>}
    </section>
  );
}
