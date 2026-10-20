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
type Connection = { sessionId: string; endpointUrl: string; protocolVersion: string; serverName?: string; serverVersion?: string };
type Instance = { id: string; name: string; hash: string; isActive: boolean; port?: number };
type EditorState = {
  sessionId: string; boundInstanceId: string; endpointUrl: string;
  isPlaying: boolean; isPaused: boolean; isCompiling: boolean; unityVersion?: string;
  projectVerification:
    | { type: "verified"; canonical_root: string; matched_approved_root: string; provider_sharing_enabled: boolean }
    | { type: "mismatched"; reported_root: string; reason: string }
    | { type: "unverifiable"; reason: string };
  providerSharingEligible: boolean; warnings?: string[];
};

async function closeSession(sessionId: string) {
  await invoke("disconnect_mcp_session", { sessionId });
}

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
  const [connection, setConnection] = useState<Connection | null>(null);
  const [instances, setInstances] = useState<Instance[]>([]);
  const [selected, setSelected] = useState("");
  const [editor, setEditor] = useState<EditorState | null>(null);
  const session = useRef<Connection | null>(null);
  const sessionGeneration = useRef(0);
  const sessionBusy = useRef(false);
  const [sessionPending, setSessionPending] = useState(false);

  useEffect(() => () => {
    ++sessionGeneration.current;
    const current = session.current;
    session.current = null;
    if (current) { void closeSession(current.sessionId).catch(() => {}); }
  }, []);

  function clearSession() {
    ++sessionGeneration.current;
    session.current = null;
    sessionBusy.current = false;
    setSessionPending(false);
    setConnection(null);
    setInstances([]);
    setSelected("");
    setEditor(null);
  }

  async function disconnect() {
    const current = session.current;
    clearSession();
    const request = sessionGeneration.current;
    if (!current) { return; }
    try {
      await closeSession(current.sessionId);
    } catch (cause) {
      if (request === sessionGeneration.current) {
        setError(`Disconnect failed; local binding cleared. ${String(cause)}`);
      }
    }
  }

  async function connect(endpoint: string) {
    if (!desktop || sessionBusy.current || session.current || confirmation) { return; }
    const request = ++sessionGeneration.current;
    sessionBusy.current = true;
    setSessionPending(true);
    setError("");
    setMessage("");
    let opened: Connection | null = null;
    try {
      opened = await invoke<Connection>("connect_approved_mcp_endpoint", { endpoint });
      if (request !== sessionGeneration.current) {
        await closeSession(opened.sessionId);
        return;
      }
      session.current = opened;
      setConnection(opened);
      const found = await invoke<Instance[]>("list_connected_unity_instances", { sessionId: opened.sessionId });
      if (request !== sessionGeneration.current) { return; }
      setInstances(found);
    } catch (cause) {
      if (request !== sessionGeneration.current) { return; }
      clearSession();
      setError(`Connection failed. ${String(cause)}`);
      if (opened) { await closeSession(opened.sessionId).catch(() => {}); }
    } finally {
      if (request === sessionGeneration.current) {
        sessionBusy.current = false;
        setSessionPending(false);
      }
    }
  }

  async function inspect(bind: boolean) {
    const current = session.current;
    if (!desktop || !current || sessionBusy.current || (bind && !selected)) { return; }
    const request = ++sessionGeneration.current;
    sessionBusy.current = true;
    setSessionPending(true);
    setEditor(null);
    setError("");
    try {
      const state = await invoke<EditorState>(bind ? "bind_unity_instance" : "get_bound_unity_state", {
        sessionId: current.sessionId, ...(bind ? { exactId: selected } : {}),
      });
      if (request !== sessionGeneration.current) { return; }
      setEditor(state);
    } catch (cause) {
      if (request !== sessionGeneration.current) { return; }
      clearSession();
      setError(`Editor inspection failed; binding cleared. Reconnect explicitly to retry. ${String(cause)}`);
      await closeSession(current.sessionId).catch(() => {});
    } finally {
      if (request === sessionGeneration.current) {
        sessionBusy.current = false;
        setSessionPending(false);
      }
    }
  }

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
    if (action.revoke && session.current?.endpointUrl === action.endpoint) {
      const current = session.current;
      clearSession();
      void closeSession(current.sessionId).catch(() => {});
    }
    try {
      const saved = await invoke<GrantResult>(action.revoke ? "revoke_mcp_endpoint" : "approve_mcp_endpoint", { endpoint: action.endpoint });
      if (request !== generation.current) { return; }
      setGrants(saved.approvedEndpoints);
      setConfirmation(null);
      setMessage(`${saved.canonicalUrl}: first-connection grant ${saved.isApproved ? "approved" : "revoked"}.`);
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
      <OfficialUnityRelay />
      <h3>CoplayDev Unity MCP (HTTP)</h3>
      <p>Discover local endpoints, approve an exact address, then explicitly connect and choose a Unity editor. Discovery checks HTTP availability only.</p>
      <p className="mcp-boundary">Local inspection only. These controls do not expose Unity to chat or AI tools. Project verification comes from the native approved-root check, not an editor name.</p>
      <section className="mcp-session" aria-labelledby="mcp-session-title">
        <div className="mcp-actions"><h3 id="mcp-session-title">Editor session</h3>{connection && <button type="button" onClick={disconnect}>Disconnect</button>}</div>
        {!connection && <p>No editor connected. Choose Connect approved endpoint from a saved grant.</p>}
        {sessionPending && <p role="status">Waiting for local MCP response...</p>}
        {connection && <>
          <code>{connection.endpointUrl}</code>
          <p className="mcp-session-meta">Protocol {connection.protocolVersion}{connection.serverName ? ` / ${connection.serverName}` : ""}{connection.serverVersion ? ` ${connection.serverVersion}` : ""} (server-reported)</p>
          <label htmlFor="mcp-instance">Exact Unity instance</label>
          <div className="mcp-actions">
            <select id="mcp-instance" value={selected} disabled={sessionPending || !!confirmation} onChange={(event) => setSelected(event.target.value)}>
              <option value="">Choose an editor...</option>
              {instances.map((instance) => <option key={instance.id} value={instance.id}>{instance.id}{instance.isActive ? " (server active)" : ""}</option>)}
            </select>
            <button type="button" disabled={sessionPending || !selected || !!confirmation} onClick={() => inspect(true)}>Bind selected editor</button>
          </div>
          {!sessionPending && instances.length === 0 && <p>No Unity instances returned. Disconnect and reconnect to reload the instance list.</p>}
          {editor && <div className="mcp-editor-state">
            <div className="mcp-actions"><h4>Local editor state</h4><button type="button" disabled={sessionPending || !!confirmation} onClick={() => inspect(false)}>Refresh editor state</button></div>
            <dl>
              <dt>Bound instance</dt><dd>{editor.boundInstanceId}</dd>
              <dt>Unity version</dt><dd>{editor.unityVersion || "Not reported"}</dd>
              <dt>Editor</dt><dd>{editor.isPlaying ? "Playing" : "Not playing"} / {editor.isPaused ? "Paused" : "Not paused"} / {editor.isCompiling ? "Compiling" : "Not compiling"}</dd>
              <dt>Project verification</dt><dd>{editor.projectVerification.type === "verified" ? "Verified approved project" : editor.projectVerification.type === "mismatched" ? "Project mismatch" : "Project unverifiable"}</dd>
              {editor.projectVerification.type === "verified" ? <>
                <dt>Canonical root</dt><dd>{editor.projectVerification.canonical_root}</dd>
                <dt>Approved root</dt><dd>{editor.projectVerification.matched_approved_root}</dd>
              </> : <><dt>Reason</dt><dd>{editor.projectVerification.reason}</dd>{editor.projectVerification.type === "mismatched" && <><dt>Reported root</dt><dd>{editor.projectVerification.reported_root}</dd></>}</>}
            </dl>
            {editor.warnings?.map((warning, index) => <p className="mcp-warning" key={index}>{warning}</p>)}
          </div>}
        </>}
      </section>
      {!desktop && <p role="status">Browser preview is inert. Open the desktop app to list, discover, or approve local endpoints.</p>}
      <div className="mcp-actions">
        <button type="button" onClick={discover} disabled={!desktop || !!busy || sessionPending || !!confirmation}>Discover local endpoints</button>
        <span>Explicitly sends bounded HTTP GET probes to supported loopback candidates.</span>
      </div>
      {busy && <p role="status">{busy}</p>}
      {error && <div className="mcp-error" role="alert"><p>{error}</p><button type="button" onClick={() => setError("")}>Dismiss error</button></div>}
      {message && <p role="status">{message}</p>}
      {confirmation && (
        <div className="mcp-confirmation" role="group" aria-labelledby="mcp-confirm-title">
          <h3 id="mcp-confirm-title">{confirmation.revoke ? "Revoke" : "Approve"} this endpoint's first-connection grant?</h3>
          <code>{confirmation.endpoint}</code>
          <p>This changes only a saved local permission for this exact URL, port and path. Approval does not open a session or select an editor. Revocation invalidates sessions for this endpoint, even if saving the permission change fails.</p>
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
              {candidate.supported && candidate.endpointUrl && <button type="button" disabled={!desktop || !!busy || sessionPending || !!confirmation} onClick={() => requestChange(candidate.endpointUrl!, approved)}>{approved ? "Revoke endpoint approval" : "Review endpoint approval"}</button>}
            </li>;
          })}
        </ul>
        {result.warnings?.map((warning, i) => <p key={i} className="mcp-warning">{warning}</p>)}
      </>}
      {grants.length > 0 && <>
        <h3>Saved first-connection grants</h3>
        <p>Saved permissions remain visible here even when a configuration no longer lists the endpoint.</p>
        <ul className="mcp-grants">{grants.map((grant) => <li key={grant.canonicalUrl}><code>{grant.canonicalUrl}</code><span>Approved at {grant.approvedAt}</span><div className="mcp-actions"><button type="button" disabled={!desktop || !!busy || sessionPending || !!connection || !!confirmation} onClick={() => connect(grant.canonicalUrl)}>Connect approved endpoint</button><button type="button" disabled={!desktop || !!busy || sessionPending || !!confirmation} onClick={() => requestChange(grant.canonicalUrl, true)}>Revoke saved grant</button></div></li>)}</ul>
      </>}
    </section>
  );
}
type ExecutableGrant = { canonicalPath: string; approvedAt: string };
type ExecutableResult = { canonicalPath: string; isApproved: boolean; approvedExecutables: ExecutableGrant[] };
type StdioConnection = { sessionId: string; canonicalPath: string; protocolVersion: string; serverName?: string; serverVersion?: string };
type StdioInventory = { sessionId: string; tools: { name: string }[]; resources: { name: string }[]; editorBindingStatus: { type: "pendingOrUndocumented"; details: string }; warnings?: string[] };

function OfficialUnityRelay() {
  const desktop = isTauri();
  const [path, setPath] = useState("");
  const [grants, setGrants] = useState<ExecutableGrant[]>([]);
  const [review, setReview] = useState<{ path: string; revoke: boolean } | null>(null);
  const [busy, setBusy] = useState("");
  const [error, setError] = useState("");
  const [connection, setConnection] = useState<StdioConnection | null>(null);
  const [inventory, setInventory] = useState<StdioInventory | null>(null);
  const mounted = useRef(false);
  const epoch = useRef(0);
  const locked = useRef(false);
  const active = useRef<StdioConnection | null>(null);
  const confirm = useRef<HTMLButtonElement>(null);
  const absolute = /^(?:[a-zA-Z]:\\|\\\\[^\\]+\\[^\\]+\\|\/)/.test(path.trim());

  useEffect(() => {
    mounted.current = true;
    const request = ++epoch.current;
    if (desktop) {
      setBusy("Reading saved executable grants...");
      invoke<ExecutableGrant[]>("get_mcp_executable_grants").then((saved) => {
        if (request !== epoch.current) { return; }
        if (!Array.isArray(saved)) { throw new Error("Executable grants could not be read."); }
        setGrants(saved);
      }).catch((cause: unknown) => {
        if (request === epoch.current) { setError(String(cause)); }
      }).finally(() => {
        if (request === epoch.current) { setBusy(""); }
      });
    }
    return () => {
      mounted.current = false;
      ++epoch.current;
      const current = active.current;
      active.current = null;
      if (current) { void closeSession(current.sessionId).catch(() => {}); }
    };
  }, [desktop]);
  useEffect(() => { if (review) { confirm.current?.focus(); } }, [review]);

  async function disconnect() {
    ++epoch.current;
    const current = active.current;
    active.current = null;
    setConnection(null);
    setInventory(null);
    // Keep an in-flight launch locked until its late result has been closed.
    if (!current) { return; }
    locked.current = true;
    setBusy("Disconnecting relay...");
    try { await closeSession(current.sessionId); }
    catch (cause) { if (mounted.current) { setError(`Disconnect failed; local session cleared. ${String(cause)}`); } }
    finally { locked.current = false; if (mounted.current) { setBusy(""); } }
  }

  async function changeGrant() {
    if (!desktop || !review || locked.current) { return; }
    const action = review;
    locked.current = true;
    const request = ++epoch.current;
    setBusy("Saving executable permission...");
    setError("");
    if (action.revoke && active.current?.canonicalPath === action.path) {
      const current = active.current;
      active.current = null;
      setConnection(null);
      setInventory(null);
      void closeSession(current.sessionId).catch(() => {});
    }
    try {
      const saved = await invoke<ExecutableResult>(action.revoke ? "revoke_mcp_executable" : "approve_mcp_executable", { path: action.path });
      if (request !== epoch.current) { return; }
      if (!Array.isArray(saved.approvedExecutables)) { throw new Error("Invalid executable grant response."); }
      setGrants(saved.approvedExecutables);
      setReview(null);
    } catch (cause) {
      if (request === epoch.current) { setError(`Permission change failed; saved grants unchanged. ${String(cause)}`); }
    } finally {
      locked.current = false;
      if (mounted.current) { setBusy(""); }
    }
  }

  async function connect(executablePath: string) {
    if (!desktop || locked.current || active.current || review) { return; }
    locked.current = true;
    const request = ++epoch.current;
    setBusy("Starting approved relay; accept Yuki in Unity Pending Connections if prompted...");
    setError("");
    setInventory(null);
    try {
      const opened = await invoke<StdioConnection>("connect_approved_mcp_stdio", { executablePath });
      if (request !== epoch.current) { await closeSession(opened.sessionId); return; }
      active.current = opened;
      setConnection(opened);
    } catch (cause) {
      if (mounted.current) { setError(`Relay connection or cleanup failed. ${String(cause)}`); }
    } finally {
      locked.current = false;
      if (mounted.current) { setBusy(""); }
    }
  }

  async function inspect() {
    const current = active.current;
    if (!desktop || !current || locked.current || review) { return; }
    locked.current = true;
    const request = ++epoch.current;
    setBusy("Reading bounded capability inventory...");
    setError("");
    setInventory(null);
    try {
      const found = await invoke<StdioInventory>("list_mcp_stdio_capabilities", { sessionId: current.sessionId });
      if (request !== epoch.current) { return; }
      if (found.sessionId !== current.sessionId || !Array.isArray(found.tools) || !Array.isArray(found.resources)) { throw new Error("Invalid relay inventory."); }
      setInventory(found);
    } catch (cause) {
      if (request === epoch.current) {
        active.current = null;
        setConnection(null);
        setError(`Capability inventory failed; local session cleared. Reconnect explicitly. ${String(cause)}`);
        await closeSession(current.sessionId).catch(() => {});
      }
    } finally { locked.current = false; if (mounted.current) { setBusy(""); } }
  }

  return <section className="mcp-relay" aria-labelledby="mcp-relay-title">
    <h3 id="mcp-relay-title">Official Unity relay (stdio)</h3>
    <p>Separate from CoplayDev HTTP. Choose an absolute executable path yourself; Yuki does not search your filesystem for a relay.</p>
    <label htmlFor="mcp-relay-path">Relay executable absolute path</label>
    <input id="mcp-relay-path" value={path} placeholder={'C:\\Users\\...\\.unity\\relay\\relay_win.exe'} disabled={!desktop || !!busy || !!review} onChange={(event) => setPath(event.target.value)} spellCheck={false} />
    <button type="button" disabled={!desktop || !absolute || !!busy || !!review} onClick={() => { setError(""); setReview({ path: path.trim(), revoke: false }); }}>Review executable approval</button>
    <p className="mcp-session-meta">Launch command is fixed: selected executable + --mcp. No shell or custom arguments. No provider-sharing permission is granted.</p>
    {!desktop && <p>Browser preview cannot approve or launch a relay.</p>}
    {busy && <p role="status">{busy}</p>}
    {error && <div className="mcp-error" role="alert"><p>{error}</p><button type="button" onClick={() => setError("")}>Dismiss relay error</button></div>}
    {review && <div className="mcp-confirmation" role="group" aria-labelledby="mcp-relay-confirm">
      <h4 id="mcp-relay-confirm">{review.revoke ? "Revoke executable launch permission?" : "Approve this exact executable?"}</h4>
      <code>"{review.path}" --mcp</code>
      <p>{review.revoke ? "Revocation clears sessions for this executable, even if saving fails." : "Approval saves permission only. A separate Connect approved relay action starts this local process with the fixed --mcp argument. Only approve an executable you trust."}</p>
      <p>Unity may require accepting Yuki under Project Settings &gt; AI &gt; Unity MCP &gt; Pending Connections. This does not verify server identity, bind an editor, or grant provider sharing.</p>
      <div className="mcp-actions"><button ref={confirm} type="button" disabled={!!busy} onClick={changeGrant}>{review.revoke ? "Confirm executable revoke" : "Confirm executable approval"}</button><button type="button" disabled={!!busy} onClick={() => setReview(null)}>Cancel executable review</button></div>
    </div>}
    <h4>Saved executable grants</h4>
    {!grants.length && <p>No saved relay launch grants.</p>}
    <ul className="mcp-grants">{grants.map((grant) => <li key={grant.canonicalPath}><code>{grant.canonicalPath}</code><span>Approved {grant.approvedAt}</span><div className="mcp-actions">
      <button type="button" disabled={!desktop || !!busy || !!connection || !!review} onClick={() => connect(grant.canonicalPath)}>Connect approved relay</button>
      <button type="button" disabled={!desktop || !!busy || !!review} onClick={() => setReview({ path: grant.canonicalPath, revoke: true })}>Revoke executable grant</button>
    </div></li>)}</ul>
    {(connection || busy.startsWith("Starting")) && <button type="button" disabled={!!review || (!!busy && !busy.startsWith("Starting"))} onClick={disconnect}>Disconnect relay</button>}
    {connection && <div className="mcp-editor-state">
      <code>{connection.canonicalPath}</code>
      <p>Server-reported: {connection.serverName || "Name not reported"} / {connection.serverVersion || "Version not reported"}. Protocol {connection.protocolVersion}. Identity unverified.</p>
      <p>Editor binding pending or undocumented. Accepting Pending Connections does not establish verified project identity here.</p>
      <button type="button" disabled={!!busy || !!review} onClick={inspect}>Inspect relay capabilities</button>
    </div>}
    {inventory && <div className="mcp-relay-inventory">
      <p>Untrusted server metadata, displayed locally only. Inventory does not authorize tool execution or provider sharing.</p>
      <p>Tools ({inventory.tools.length})</p><ul>{inventory.tools.map((tool, index) => <li key={index}>{tool.name}</li>)}</ul>
      <p>Resources ({inventory.resources.length})</p><ul>{inventory.resources.map((resource, index) => <li key={index}>{resource.name}</li>)}</ul>
      <p>Binding pending or undocumented: {inventory.editorBindingStatus.details}</p>
      {inventory.warnings?.map((warning, index) => <p className="mcp-warning" key={index}>{warning}</p>)}
    </div>}
  </section>;
}
