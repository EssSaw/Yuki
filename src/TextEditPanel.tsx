import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./TextEditPanel.css";

type Proposal = { id: string; path: string; conversationId: string; grantId: string; generation: number; originalBytes: number; replacementBytes: number; applyAvailable: boolean };
type Recovery = { id: string; state: string; reviewRequired: boolean };
interface Props {
  desktop: boolean; disabled: boolean; conversationId: string | null;
  taskRoot?: string; taskDescription?: string; generation?: number; grants: { id: string; canonicalPath: string }[];
  onBusy: (busy: boolean) => void;
}
const LIMIT = 1024 * 1024;

export default function TextEditPanel({ desktop, disabled, conversationId, taskRoot, taskDescription, generation, grants, onBusy }: Props) {
  const [path, setPath] = useState("");
  const [replacement, setReplacement] = useState("");
  const [proposal, setProposal] = useState<Proposal | null>(null);
  const [lastChange, setLastChange] = useState<string | null>(null);
  const [recovery, setRecovery] = useState<Recovery[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [status, setStatus] = useState("");
  const [error, setError] = useState("");
  const [stopStatus, setStopStatus] = useState("");
  const epoch = useRef(0);
  const active = useRef(false);
  const stopPending = useRef(false);
  const scope = JSON.stringify([desktop, conversationId, taskRoot, taskDescription, generation]);
  const currentScope = useRef(scope);
  currentScope.current = scope;
  const bytes = new TextEncoder().encode(replacement).length;
  const locked = !desktop || disabled || busy !== null;
  const authorized = !!conversationId && !!taskRoot && generation !== undefined && grants.length > 0;
  const valid = /^(?:[a-zA-Z]:[\\/]|\\\\[^\\]+\\[^\\]+\\|\/)/.test(path) && bytes <= LIMIT && !replacement.includes("\0");

  useEffect(() => {
    ++epoch.current;
    setPath(""); setReplacement(""); setProposal(null); setLastChange(null);
    setRecovery(null); setStatus(""); setError(""); setStopStatus("");
    return () => { ++epoch.current; };
  }, [scope]);

  async function run(kind: string, action: () => Promise<void>) {
    if (locked || active.current) { return; }
    active.current = true; setBusy(kind); onBusy(true); setError(""); setStatus(""); setStopStatus("");
    const token = epoch.current;
    try { await action(); }
    catch (reason) {
      if (epoch.current === token) { setError(`${kind} failed: ${String(reason)} Draft preserved. Review recovery before retrying a write.`); }
    } finally {
      active.current = false; setBusy(null); onBusy(false);
    }
  }
  function isCurrent(token: number, origin: string) { return epoch.current === token && currentScope.current === origin; }
  async function prepare() {
    if (!authorized || !valid) { return; }
    const token = epoch.current; const origin = scope;
    await run("Prepare", async () => {
      setProposal(null);
      const result = await invoke<Proposal>("prepare_local_text_edit", { conversationId, path, generation, replacement });
      if (!isCurrent(token, origin)) { return; }
      if (result.conversationId !== conversationId || result.generation !== generation || !grants.some((grant) => grant.id === result.grantId)) {
        throw new Error("Proposal does not match the current permission scope");
      }
      setProposal(result); setStatus("Prepared locally. No file has been changed.");
    });
  }
  async function apply() {
    if (!proposal?.applyAvailable) { return; }
    const token = epoch.current; const origin = scope;
    await run("Apply", async () => {
      const changeId = await invoke<string>("apply_local_text_edit", { proposalId: proposal.id });
      if (isCurrent(token, origin)) { setLastChange(changeId); setProposal(null); setStatus("Apply confirmed. The file was changed; Undo is available for this change."); }
    });
  }
  async function undo() {
    if (!lastChange || generation === undefined) { return; }
    const token = epoch.current; const origin = scope;
    await run("Undo", async () => {
      await invoke("undo_local_text_edit", { changeId: lastChange, generation });
      if (isCurrent(token, origin)) { setLastChange(null); setProposal(null); setStatus("Undo confirmed. The recorded original was restored. Your replacement draft is retained."); }
    });
  }
  async function stop() {
    if (!desktop || !active.current || stopPending.current) { return; }
    const token = epoch.current; const origin = scope;
    stopPending.current = true;
    try {
      await invoke("stop_local_text_edit");
      if (isCurrent(token, origin)) { setStopStatus("Stop requested. This is not confirmation of cancellation or rollback; wait for the operation result and review recovery."); }
    } catch (reason) {
      if (isCurrent(token, origin)) { setStopStatus(`Stop request failed: ${String(reason)}. The operation may still finish.`); }
    } finally { stopPending.current = false; }
  }
  async function refreshRecovery() {
    const token = epoch.current; const origin = scope;
    await run("Recovery refresh", async () => {
      const result = await invoke<Recovery[]>("list_local_edit_recovery");
      if (isCurrent(token, origin)) { setRecovery(result); }
    });
  }
  async function discard() {
    if (!proposal) { return; }
    const token = epoch.current; const origin = scope;
    await run("Discard", async () => {
      await invoke("discard_local_text_edit", { proposalId: proposal.id });
      if (isCurrent(token, origin)) { setProposal(null); setStatus("Proposal discarded. Draft retained; no file changed."); }
    });
  }

  return <details className="text-edit-panel">
    <summary>Local text edit &amp; recovery</summary>
    <p>Manual, local-only editing. Nothing here is sent to a provider. Model editing tools, commands and Unity actions are not available.</p>
    {!desktop && <p>Desktop only. Browser preview cannot read, prepare or change files.</p>}
    {!authorized && <p>Select a saved Agent conversation with an approved task root and write grant to prepare an edit.</p>}
    <p>Write scope: {grants.map((grant) => grant.canonicalPath).join(", ") || "No approved write roots"}</p>
    <fieldset disabled={locked || !authorized}>
      <legend>Existing UTF-8 file</legend>
      <label>Absolute file path<input value={path} onChange={(event) => { setPath(event.target.value); setProposal(null); }} placeholder="C:\\project\\file.txt" /></label>
      <label>Complete replacement text<textarea value={replacement} onChange={(event) => { setReplacement(event.target.value); setProposal(null); }} spellCheck={false} rows={8} aria-describedby="edit-size" /></label>
      <p id="edit-size">{bytes.toLocaleString()} / {LIMIT.toLocaleString()} UTF-8 bytes. Empty text replaces the file with an empty file. NUL bytes are unsupported.</p>
      <button type="button" disabled={!valid} onClick={() => void prepare()}>Prepare local edit</button>
    </fieldset>
    {proposal && <section className="text-edit-review" aria-label="Prepared local edit">
      <strong>Review replacement</strong><p>{proposal.path}</p>
      <p>Original: {proposal.originalBytes.toLocaleString()} bytes. Replacement: {proposal.replacementBytes.toLocaleString()} bytes.</p>
      <p>Original content is not loaded in this editor. This is a replacement preview, not a diff.</p>
      <pre aria-label="Replacement preview">{replacement || "(empty file)"}</pre>
      <p>Apply uses your standing write grant. It overwrites the existing file in place and is not crash-atomic. A recovery record is kept; interruptions can require manual review. Undo may fail if the file or permissions change.</p>
      <div className="text-edit-actions"><button type="button" disabled={locked || !proposal.applyAvailable} onClick={() => void apply()}>Apply replacement</button>
        <button type="button" disabled={locked} onClick={() => void discard()}>Discard proposal</button></div>
      {!proposal.applyAvailable && <p>Apply is unavailable for this proposal.</p>}
    </section>}
    <div className="text-edit-actions">
      <button type="button" disabled={locked || !lastChange || !authorized} onClick={() => void undo()}>Undo last confirmed change</button>
      <button type="button" disabled={locked} onClick={() => void refreshRecovery()}>Refresh recovery records</button>
      {(busy === "Apply" || busy === "Undo") && <button type="button" disabled={!desktop} onClick={() => void stop()}>Stop local operation</button>}
    </div>
    <div role="status" aria-live="polite">{busy && <p>{busy} in progress...</p>}{status && <p>{status}</p>}{stopStatus && <p>{stopStatus}</p>}</div>
    {error && <p role="alert">{error}</p>}
    {recovery !== null && <section aria-label="Local edit recovery records"><strong>Recovery records</strong>
      <p>Status only; no automatic recovery or file contents. Unknown or corrupt records require manual review.</p>
      {recovery.length === 0 ? <p>No recovery records found.</p> : <ul>{recovery.map((record) => <li key={record.id}><code>{record.id}</code>: {record.state} {record.reviewRequired || !["committed", "undone"].includes(record.state) ? "- Requires review" : ""}</li>)}</ul>}
    </section>}
  </details>;
}
