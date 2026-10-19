import { useEffect, useRef, useState } from "react";

export default function ModelPicker({ model, models, disabled, busy, onLoad, onSelect }: {
  model: string; models: string[]; disabled: boolean; busy: boolean;
  onLoad: () => void; onSelect: (model: string) => Promise<void>;
}) {
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState("");
  const container = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const search = useRef<HTMLInputElement>(null);
  const choices = Array.from(new Set(model ? [model, ...models] : models));
  const filtered = choices.filter((item) => item.toLowerCase().includes(query.toLowerCase()));
  useEffect(() => {
    if (!open) return;
    search.current?.focus();
    function outside(event: PointerEvent) {
      if (!container.current?.contains(event.target as Node)) setOpen(false);
    }
    document.addEventListener("pointerdown", outside);
    return () => document.removeEventListener("pointerdown", outside);
  }, [open]);
  return <div className="model-picker" ref={container} onKeyDown={(event) => {
    if (event.key === "Escape") { event.preventDefault(); setOpen(false); trigger.current?.focus(); }
  }}>
    <button ref={trigger} className="model-trigger" type="button" disabled={disabled || busy}
      aria-expanded={open} aria-controls="model-popover" aria-label={`Choose chat model: ${model || "not selected"}`}
      onClick={() => { setOpen(!open); setQuery(""); }}>
      <span className="model-spark" aria-hidden="true">✧</span>
      <span className="model-trigger-name">{model || "Choose a model"}</span><span aria-hidden="true">⌄</span>
    </button>
    {open && <div id="model-popover" className="model-popover" role="dialog" aria-label="Choose model">
      <div className="model-popover-heading"><strong>Your models</strong><button type="button" disabled={busy || disabled} onClick={onLoad}>{busy ? "Working…" : models.length ? "Refresh" : "Load models"}</button></div>
      <input ref={search} aria-label="Search models" placeholder="Find a model…" value={query} onChange={(event) => setQuery(event.target.value)} />
      <div className="model-results" aria-label="Available models">
        {filtered.map((item) => <button type="button" key={item} disabled={busy || disabled} aria-pressed={item === model}
          onClick={() => { void onSelect(item).then(() => { setOpen(false); trigger.current?.focus(); }); }}>
          <span>{item}</span><span aria-hidden="true">{item === model ? "✓" : ""}</span>
        </button>)}
        {!filtered.length && <p>{choices.length ? "No matching models." : "Load models from your saved connection."}</p>}
      </div>
      <small>Applies to your next message</small>
    </div>}
  </div>;
}
