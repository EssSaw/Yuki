import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import "./AgentControls.css";
import TextEditPanel from "./TextEditPanel";

export type AgentContext = { mode: "chat" | "agent"; taskRoot?: string; taskDescription?: string };
export const CHAT_CONTEXT: AgentContext = { mode: "chat" };
type WriteGrant = { id: string; canonicalPath: string; projectRoot: string };
type Snapshot = {
  generation: number; taskRoot?: string; isTaskRootApproved: boolean;
  providerSharingEnabled: boolean; writeRoots: WriteGrant[];
  commandTemplates: { id: string; canonicalExecutable: string }[];
  executionAvailable: boolean; unsandboxedDisclosure: string;
};
interface Props {
  desktop: boolean; disabled: boolean; conversationId: string | null;
  value: AgentContext; onChange: (value: AgentContext) => void;
  onBusy: (busy: boolean) => void;
}

export default function AgentControls({ desktop, disabled, conversationId, value, onChange, onBusy }: Props) {
  const [draft, setDraft] = useState(value);
  const [roots, setRoots] = useState<string[]>([]);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [busy, setBusy] = useState(false);
  const [editBusy, setEditBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [review, setReview] = useState<{ kind: "approve" | "revoke"; id?: string; path: string } | null>(null);
  const epoch = useRef(0);
  const locked = !desktop || disabled || busy || editBusy;
  const projectRoot = value.taskRoot;

  useEffect(() => { setDraft(value); setReview(null); }, [value]);
  useEffect(() => {
    const token = ++epoch.current;
    setSnapshot(null); setError(null); setReview(null);
    if (!desktop) return;
    void invoke<{ projectRoots: string[] }>("get_workspace_settings").then((settings) => {
      if (epoch.current === token) setRoots(settings?.projectRoots ?? []);
    }).catch((reason) => { if (epoch.current === token) setError(String(reason)); });
    void invoke<Snapshot>("get_agent_permission_snapshot", { projectRoot: projectRoot ?? null, conversationId }).then((result) => {
      if (epoch.current === token) setSnapshot(result);
    }).catch((reason) => { if (epoch.current === token) setError(String(reason)); });
    return () => { ++epoch.current; };
  }, [desktop, projectRoot, conversationId]);

  async function applyContext() {
    if (locked || (draft.mode === "agent" && !roots.includes(draft.taskRoot ?? ""))) return;
    const token = epoch.current;
    const next = draft.mode === "chat" ? CHAT_CONTEXT : { ...draft, taskDescription: draft.taskDescription?.trim() || undefined };
    setBusy(true); onBusy(true); setError(null);
    try {
      const saved = conversationId ? await invoke<AgentContext>("set_conversation_mode", {
        conversationId, mode: next.mode, taskRoot: next.taskRoot ?? null, taskDescription: next.taskDescription ?? null,
      }) : next;
      if (epoch.current === token) onChange({ mode: saved.mode ?? "chat", taskRoot: saved.taskRoot, taskDescription: saved.taskDescription });
    } catch (reason) { if (epoch.current === token) setError(`Mode was not changed: ${String(reason)}`); }
    finally { setBusy(false); onBusy(false); }
  }

  async function changeGrant() {
    if (locked || !review || !snapshot || !projectRoot) return;
    const token = epoch.current;
    setBusy(true); onBusy(true); setError(null);
    try {
      const args = review.kind === "approve" ? { projectRoot, path: review.path, expectedGeneration: snapshot.generation }
        : { projectRoot, grantId: review.id, expectedGeneration: snapshot.generation };
      const result = await invoke<{ snapshot: Snapshot }>(review.kind === "approve" ? "approve_agent_write_root" : "revoke_agent_write_root", args);
      if (epoch.current === token) { setSnapshot(result.snapshot); setReview(null); }
    } catch (reason) {
      if (epoch.current !== token) return;
      setError(`Permission was not changed. Review the refreshed permissions before trying again. ${String(reason)}`);
      setReview(null); setSnapshot(null);
      try {
        const fresh = await invoke<Snapshot>("get_agent_permission_snapshot", { projectRoot, conversationId });
        if (epoch.current === token) setSnapshot(fresh);
      } catch { /* Keep permissions unavailable rather than reuse stale authority. */ }
    } finally { setBusy(false); onBusy(false); }
  }

  return <section className="agent-controls" aria-label="Conversation mode and permissions">
    <div className="agent-current"><strong>{value.mode === "agent" ? "Agent" : "Chat"}</strong>
      <span>Project: {value.taskRoot ?? "None selected"}</span>
      {value.mode === "agent" && <span>Task: {value.taskDescription || "No description"}</span>}
    </div>
    <details>
      <summary>Mode &amp; permissions</summary>
      <fieldset disabled={locked}>
        <legend>Conversation setup</legend>
        <label>Mode<select aria-label="Conversation mode" value={draft.mode} onChange={(e) => setDraft({ ...draft, mode: e.target.value as AgentContext["mode"] })}>
          <option value="chat">Chat</option><option value="agent">Agent (foundation)</option>
        </select></label>
        {draft.mode === "agent" && <>
          <label>Task root<select value={draft.taskRoot ?? ""} onChange={(e) => setDraft({ ...draft, taskRoot: e.target.value })}>
            <option value="">Choose an approved project</option>{roots.map((root) => <option key={root} value={root}>{root}</option>)}
          </select></label>
          <label>Task description<input value={draft.taskDescription ?? ""} onChange={(e) => setDraft({ ...draft, taskDescription: e.target.value })} maxLength={2000} /></label>
        </>}
        <button type="button" disabled={draft.mode === "agent" && !roots.includes(draft.taskRoot ?? "")} onClick={() => void applyContext()}>Apply mode &amp; task</button>
      </fieldset>
      <p>Model execution, commands and Unity actions are unavailable. Manual local text edits use a separately reviewed replacement and an approved write root.</p>
      <p>Provider sharing is separate from write permission and uses existing project sharing grants.</p>
      {snapshot ? <div className="agent-permissions">
        <p>Permission revision {snapshot.generation} · Provider sharing: {snapshot.providerSharingEnabled ? "approved" : "not approved"}</p>
        {projectRoot && <p>Task root: {snapshot.isTaskRootApproved ? "approved" : "not approved"}</p>}
        {snapshot.unsandboxedDisclosure && <p>{snapshot.unsandboxedDisclosure}</p>}
        {projectRoot && <button type="button" disabled={locked || !snapshot.isTaskRootApproved} onClick={() => setReview({ kind: "approve", path: projectRoot })}>Review write-root approval</button>}
        <ul>{snapshot.writeRoots.map((grant) => <li key={grant.id}><span>{grant.canonicalPath}</span><button type="button" disabled={locked || !projectRoot} onClick={() => setReview({ kind: "revoke", id: grant.id, path: grant.canonicalPath })}>Review revoke</button></li>)}</ul>
        {!snapshot.writeRoots.length && <p>No write roots approved.</p>}
        <p>Command template creation and execution are unavailable in this UI. {snapshot.commandTemplates.length} saved records.</p>
      </div> : <p>{desktop ? "Permission snapshot unavailable or loading." : "Desktop only. Preview cannot change permissions."}</p>}
      {review && <section className="agent-review" aria-label="Review write permission">
        <strong>{review.kind === "approve" ? "Approve write root?" : "Revoke write root?"}</strong><p>{review.path}</p>
        <p>This changes a persistent permission record only, not provider sharing. No files will be edited.</p>
        <button type="button" disabled={locked} onClick={() => setReview(null)}>Keep permissions</button>
        <button type="button" disabled={locked} onClick={() => void changeGrant()}>Confirm {review.kind === "approve" ? "approval" : "revocation"}</button>
      </section>}
      {error && <p role="alert">{error}</p>}
      <TextEditPanel desktop={desktop} disabled={disabled || busy} conversationId={value.mode === "agent" ? conversationId : null}
        taskRoot={value.mode === "agent" && snapshot?.isTaskRootApproved ? projectRoot : undefined}
        taskDescription={value.taskDescription} generation={snapshot?.generation} grants={snapshot?.writeRoots ?? []}
        onBusy={(next) => { setEditBusy(next); onBusy(next); }} />
    </details>
  </section>;
}
