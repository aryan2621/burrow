import { useEffect, useState } from "react";
import { Agent, Note, rpc, sqlTime, ui } from "./rpc";

/** The agent's notes: search, edit and delete what it remembers. */
export default function Memory({ agent }: { agent: Agent }) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<{ id: number; text: string } | null>(null);
  const [error, setError] = useState("");
  const load = (q = query) => rpc<Note[]>("notes.list", { agent_id: agent.id, query: q }).then(setNotes, (e) => setError(String(e)));
  useEffect(() => void load(""), []);
  const call = (method: string, p: object) => rpc(method, { agent_id: agent.id, ...p }).then(() => { setEditing(null); load(); }, (e) => setError(String(e)));
  return (
    <div className="h-full max-w-2xl space-y-3 overflow-y-auto p-4">
      <input aria-label="Search notes" className={ui.input} placeholder="Search notes" value={query}
        onChange={(e) => { setQuery(e.target.value); load(e.target.value); }} />
      {error && <p role="alert" className="text-sm text-red-600">{error}</p>}
      {notes.length === 0 && <p className="text-sm text-stone-500">{query ? "No matching notes." : "This agent hasn't saved any notes."}</p>}
      <ul className="space-y-2">
        {notes.map((n) => (
          <li key={n.id} className="rounded border border-stone-200 p-2 text-sm dark:border-stone-700">
            {editing?.id === n.id ? (
              <div className="space-y-2">
                <textarea aria-label="Note text" className={ui.input} rows={4} value={editing.text}
                  onChange={(e) => setEditing({ id: n.id, text: e.target.value })} />
                <div className="flex gap-2">
                  <button className={ui.button} onClick={() => call("notes.update", { note_id: n.id, text: editing.text })}>Save</button>
                  <button className={ui.button} onClick={() => setEditing(null)}>Cancel</button>
                </div>
              </div>
            ) : (
              <>
                <p className="whitespace-pre-wrap">{n.text}</p>
                <div className="mt-1 flex items-center justify-between text-xs text-stone-500">
                  <span>{sqlTime(n.created_at).toLocaleString()}</span>
                  <span className="flex gap-2">
                    <button className={ui.button} onClick={() => setEditing({ id: n.id, text: n.text })}>Edit</button>
                    <button className={ui.button} onClick={() => confirm("Delete this note?") && call("notes.delete", { note_id: n.id })}>Delete</button>
                  </span>
                </div>
              </>
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}
