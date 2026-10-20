import { useEffect, useRef, useState, type MutableRefObject } from "react";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import "./NotesPanel.css";

type Summary = { id: string; title: string; createdAt: string; updatedAt: string; versionToken: string };
type Note = Summary & { schemaVersion: number; body: string };
type Draft = { title: string; body: string };
export type NotesGuard = ((leave: () => void) => void) | null;

export default function NotesPanel({ guard }: { guard: MutableRefObject<NotesGuard> }) {
  const native = isTauri();
  const [notes, setNotes] = useState<Summary[]>([]);
  const [record, setRecord] = useState<Note | null>(null);
  const [draft, setDraft] = useState<Draft | null>(null);
  const [busy, setBusy] = useState(false);
  const [ready, setReady] = useState(false);
  const [error, setError] = useState("");
  const [pending, setPending] = useState<(() => void) | null>(null);
  const alive = useRef(false);
  const locked = useRef(false);
  const closeListener = useRef<(() => void) | null>(null);
  const current = useRef({ draft, record, pending });
  current.current = { draft, record, pending };
  const dirty = draft !== null && (record === null || draft.title !== record.title || draft.body !== record.body);
  const dirtyRef = useRef(dirty);
  dirtyRef.current = dirty;
  const saveRef = useRef<() => Promise<boolean>>(async () => false);

  function request(leave: () => void) {
    if (locked.current || current.current.pending) { return; }
    if (dirtyRef.current) { setPending(() => leave); } else { leave(); }
  }
  const requestRef = useRef(request);
  requestRef.current = request;

  useEffect(() => {
    alive.current = true;
    guard.current = (leave) => requestRef.current(leave);
    let unlisten: (() => void) | undefined;
    let disposed = false;
    if (native) {
      void (async () => {
        try {
          const stop = await getCurrentWindow().onCloseRequested((event) => {
            event.preventDefault();
            requestRef.current(() => {
              // Remove interception before the confirmed close. Tauri's automatic
              // destroy path needs a separate permission and can reject unhandled.
              setReady(false);
              void (async () => {
                try {
                  await closeListener.current?.();
                  closeListener.current = null;
                  await getCurrentWindow().close();
                } catch {
                  if (alive.current) { setError("Window could not close. Your buffer is kept. Copy it before reopening Notes to restore editing."); }
                }
              })();
            });
          });
          if (disposed) { stop(); return; }
          let stopped = false;
          unlisten = () => { if (!stopped) { stopped = true; stop(); } };
          closeListener.current = unlisten;
          setReady(true);
        } catch {
          if (!disposed) { setError("Close protection could not start. Editing is disabled; reopen Notes to retry."); }
        }
      })();
      void invoke<Summary[]>("list_notes").then((rows) => {
        if (!disposed) { setNotes((existing) => [...existing, ...rows.filter((row) => !existing.some((note) => note.id === row.id))]); }
      }).catch(() => {
        if (!disposed) { setError("Notes could not be listed. Check the local data folder in Settings, then reopen Notes."); }
      });
    }
    const keydown = (event: KeyboardEvent) => {
      if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === "s") {
        event.preventDefault();
        if (native) { void saveRef.current(); }
      }
    };
    window.addEventListener("keydown", keydown);
    return () => {
      disposed = true;
      alive.current = false;
      guard.current = null;
      unlisten?.();
      window.removeEventListener("keydown", keydown);
    };
  }, [guard, native]);

  async function open(id: string) {
    locked.current = true;
    setBusy(true);
    setError("");
    try {
      const loaded = await invoke<Note>("load_note", { id });
      if (alive.current) { setRecord(loaded); setDraft({ title: loaded.title, body: loaded.body }); }
    } catch {
      if (alive.current) { setError("Note could not be opened. The current buffer has been kept."); }
    } finally {
      locked.current = false;
      if (alive.current) { setBusy(false); }
    }
  }

  async function save(): Promise<boolean> {
    const snapshot = current.current;
    if (!native || !ready || locked.current || !snapshot.draft) { return false; }
    if (!dirtyRef.current) { return true; }
    const { title, body } = snapshot.draft;
    if (Array.from(title).length > 200 || new TextEncoder().encode(body).length > 4 * 1024 * 1024) {
      setError("Save failed: title must be at most 200 characters and body at most 4 MiB. Your buffer is unchanged.");
      return false;
    }
    locked.current = true;
    setBusy(true);
    setError("");
    try {
      const saved = await invoke<Note>(snapshot.record ? "save_note" : "create_note", snapshot.record
        ? { id: snapshot.record.id, title, body, expectedVersion: snapshot.record.versionToken }
        : { title, body });
      if (!alive.current) { return false; }
      setRecord(saved);
      setNotes((rows) => [saved, ...rows.filter((row) => row.id !== saved.id)]);
      // Keep the live buffer: a save acknowledgment never replaces subsequent edits.
      return current.current.draft === snapshot.draft;
    } catch (failure) {
      if (alive.current) {
        const conflict = String(failure).toLowerCase().includes("conflict");
        setError(conflict
          ? "Save failed: this note changed on disk. Your draft is kept; copy it before discarding and reopening the note. No overwrite was attempted."
          : "Save failed. Your draft is kept. Check the local data folder and size limits, then retry. JSON escaping can exceed the serialized record limit.");
      }
      return false;
    } finally {
      locked.current = false;
      if (alive.current) { setBusy(false); }
    }
  }
  saveRef.current = save;

  async function confirmSave() {
    const leave = pending;
    if (await save()) {
      if (alive.current) { setPending(null); leave?.(); }
    }
  }

  return <section className="notes-panel" aria-label="Local notes">
    <header className="notes-heading"><div><p className="eyebrow">YOUR LOCAL SKETCHBOOK</p><h1>Notes &amp; canvas</h1><p>Markdown notes, kept on this device. Canvas is not available yet.</p></div>
      <button type="button" disabled={!native || !ready || busy || !!pending} onClick={() => request(() => { setRecord(null); setDraft({ title: "Untitled note", body: "" }); setError(""); })}>New note</button>
    </header>
    {!native && <p role="status">Browser preview has no filesystem access. Open the desktop app to use notes.</p>}
    <p className="notes-storage">Stored as internal JSON records in your selected data folder, not standalone .md files. No AI sharing or sync.</p>
    <div className="notes-layout">
      <nav className="notes-list" aria-label="Saved notes"><h2>Saved notes</h2>{notes.length === 0 && <p>No notes to show.</p>}{notes.map((note) => <button type="button" key={note.id} aria-current={record?.id === note.id ? "page" : undefined} disabled={!ready || busy || !!pending} onClick={() => { if (record?.id !== note.id) { request(() => void open(note.id)); } }}>{note.title || "Untitled note"}</button>)}</nav>
      <div className="notes-editor">
        {draft ? <>
          <div className="notes-save"><span role="status">{busy ? "Working..." : error ? "Failed" : dirty ? "Unsaved" : "Saved"}</span><button type="button" disabled={!ready || busy || !dirty || !!pending} onClick={() => void save()}>Save</button><span>Ctrl+S</span></div>
          <label htmlFor="note-title">Title</label><input id="note-title" value={draft.title} disabled={!ready || busy || !!pending} onChange={(event) => setDraft({ ...draft, title: event.target.value })} aria-describedby="note-limits" />
          <label htmlFor="note-body">Markdown body</label><textarea id="note-body" value={draft.body} disabled={!ready || busy || !!pending} onChange={(event) => setDraft({ ...draft, body: event.target.value })} spellCheck={false} aria-describedby="note-limits" />
        </> : <div className="notes-empty"><h2>A place for a thought.</h2><p>Open a saved note or start a new one. Nothing is saved until you choose Save.</p></div>}
        <p id="note-limits" className="notes-limits">Up to 200 title characters and 4 MiB of Markdown. The serialized JSON record must also fit within 4 MiB + 64 KiB; escaping counts toward that limit. Oversized drafts stay here until corrected.</p>
        {error && <p className="notes-error" role="alert">{error}</p>}
      </div>
    </div>
    {pending && <div className="notes-dialog-backdrop"><div role="alertdialog" aria-modal="true" aria-labelledby="notes-leave-title" aria-describedby="notes-leave-description" className="notes-dialog" onKeyDown={(event) => {
      if (event.key === "Escape" && !busy) { setPending(null); }
      if (event.key === "Tab") {
        const buttons = Array.from(event.currentTarget.querySelectorAll<HTMLButtonElement>("button:not(:disabled)"));
        const first = buttons[0]; const last = buttons[buttons.length - 1];
        if (event.shiftKey && document.activeElement === first) { event.preventDefault(); last?.focus(); }
        if (!event.shiftKey && document.activeElement === last) { event.preventDefault(); first?.focus(); }
      }
    }}><h2 id="notes-leave-title">Keep your changes?</h2><p id="notes-leave-description">Save this note before leaving, discard the unsaved changes, or stay here.</p>{error && <p role="alert">{error}</p>}<div className="notes-dialog-actions"><button type="button" disabled={busy} onClick={() => void confirmSave()}>Save changes</button><button type="button" disabled={busy} onClick={() => { const leave = pending; setPending(null); setDraft(record ? { title: record.title, body: record.body } : null); leave(); }}>Discard</button><button type="button" autoFocus disabled={busy} onClick={() => setPending(null)}>Cancel</button></div></div></div>}
  </section>;
}
