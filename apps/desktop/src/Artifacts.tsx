import { useEffect, useState } from "react";
import { Agent, Artifact, ArtifactVersion, rpc, sqlTime, ui } from "./rpc";

/** Versioned files agents saved, who can see them, and their contents. */
export default function Artifacts({ agents }: { agents: Agent[] }) {
  const [list, setList] = useState<Artifact[]>([]);
  const [open, setOpen] = useState<Artifact | null>(null);
  const [versions, setVersions] = useState<ArtifactVersion[]>([]);
  const [body, setBody] = useState("");
  const [error, setError] = useState("");
  const name = (id: number) => agents.find((a) => a.id === id)?.name ?? `agent ${id}`;
  const load = () => rpc<Artifact[]>("artifacts.list").then((l) => { setList(l); setOpen((o) => l.find((a) => a.id === o?.id) ?? null); }, (e) => setError(String(e)));
  useEffect(() => void load(), []);
  useEffect(() => {
    setBody("");
    if (open) rpc<ArtifactVersion[]>("artifacts.versions", { id: open.id }).then((v) => { setVersions(v); if (v[0]) show(v[0].id); });
  }, [open?.id]);
  const show = (version_id: number) =>
    rpc<{ text?: string; base64?: string }>("artifacts.content", { version_id }).then((c) => setBody(c.text ?? `(binary, ${Math.floor(((c.base64 ?? "").length * 3) / 4)} bytes)`));
  const share = (agent_id: number, write: boolean | null) => open && rpc("artifacts.share", { id: open.id, agent_id, write }).then(load, (e) => setError(String(e)));
  return (
    <div className="flex h-full">
      <ul className="w-64 shrink-0 space-y-1 overflow-y-auto border-r border-stone-200 p-2 text-sm dark:border-stone-700">
        {list.length === 0 && <li className="text-stone-500">Agents haven't saved any artifacts yet.</li>}
        {list.map((a) => (
          <li key={a.id}>
            <button onClick={() => setOpen(a)} className={`w-full rounded px-2 py-1 text-left ${open?.id === a.id ? "bg-stone-200 dark:bg-stone-800" : ""}`}>
              {a.name} <span className="text-stone-500">v{a.latest} · {name(a.owner_id)}{a.room_id !== null ? " · room" : ""}</span>
            </button>
          </li>
        ))}
      </ul>
      {open ? (
        <div className="min-w-0 flex-1 space-y-3 overflow-y-auto p-4 text-sm">
          <div className="flex items-center justify-between">
            <h2 className="font-semibold">{open.name}</h2>
            <button className={ui.button} onClick={() => confirm(`Delete ${open.name} and all its versions?`) && rpc("artifacts.delete", { id: open.id }).then(() => { setOpen(null); load(); })}>Delete</button>
          </div>
          <fieldset className="flex flex-wrap gap-3">
            <legend className="mb-1 text-stone-500">Shared with (owner {name(open.owner_id)}{open.room_id !== null ? " and the room's members" : ""})</legend>
            {agents.filter((a) => a.id !== open.owner_id).map((a) => {
              const acc = open.access.find((x) => x.agent_id === a.id);
              return (
                <label key={a.id} className="flex items-center gap-1">
                  {a.name}
                  <select className="rounded border border-stone-300 bg-white px-1 dark:border-stone-700 dark:bg-stone-800"
                    value={acc ? (acc.write ? "write" : "read") : "none"} onChange={(e) => share(a.id, e.target.value === "none" ? null : e.target.value === "write")}>
                    <option value="none">no access</option><option value="read">read</option><option value="write">read + write</option>
                  </select>
                </label>
              );
            })}
          </fieldset>
          <div className="flex flex-wrap gap-2">
            {versions.map((v) => (
              <button key={v.id} className={ui.button} onClick={() => show(v.id)} title={sqlTime(v.created_at).toLocaleString()}>
                v{v.version} · {name(v.author_id)} · {v.size} B
              </button>
            ))}
          </div>
          {error && <p role="alert" className="text-red-600">{error}</p>}
          <pre className="whitespace-pre-wrap rounded border border-stone-200 p-2 dark:border-stone-700">{body}</pre>
        </div>
      ) : (
        <p className="p-4 text-sm text-stone-500">Choose an artifact.</p>
      )}
    </div>
  );
}
