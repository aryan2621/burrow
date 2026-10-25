import { useEffect, useState } from "react";
import { Agent, Routine, Schedule, pickFolder, rpc, ui } from "./rpc";

const HINT = { cron: "0 9 * * 1-5  (min hour day month weekday)", interval: "3600  (seconds, at least 60)", file: "a folder or file inside the agent's folders",
  email: "email connection name (new mail in INBOX)", calendar: "calendar connection name (events starting in 15 min)",
  github: "github connection name, then owner/repo (new issues)" };

/** Cron, interval and file-change schedules; runs happen in the background, even with the window closed. */
export default function Schedules({ agent }: { agent: Agent }) {
  const [list, setList] = useState<Schedule[]>([]);
  const [error, setError] = useState("");
  const [f, setF] = useState({ kind: "cron" as Schedule["kind"], spec: "", prompt: "", routine_id: null as number | null });
  const [routines, setRoutines] = useState<Routine[]>([]);

  const load = () => {
    rpc<Schedule[]>("schedules.list", { agent_id: agent.id }).then(setList, (e) => setError(String(e)));
    rpc<Routine[]>("routines.list", { agent_id: agent.id }).then(setRoutines, (e) => setError(String(e)));
  };
  useEffect(() => void load(), [agent.id]);

  const call = (method: string, params: object) => {
    setError("");
    return rpc(method, params).then(load, (e) => { setError(String(e)); throw e; });
  };
  const add = (e: React.FormEvent) => {
    e.preventDefault();
    call("schedules.add", { agent_id: agent.id, ...f }).then(() => setF({ ...f, spec: "", prompt: "", routine_id: null }), () => {});
  };
  const when = (s: Schedule) =>
    s.kind === "file" ? "on change" : ["email", "calendar", "github"].includes(s.kind) ? "checks every minute" : s.next_run_at ? `next ${new Date(s.next_run_at * 1000).toLocaleString()}` : "";

  return (
    <div className="h-full max-w-2xl space-y-4 overflow-y-auto p-4">
      <section className="space-y-2">
        <h3 className="font-semibold">Routines</h3>
        {routines.length === 0 && <p className="text-sm text-stone-500">Save a finished run as a routine from the Timeline to reuse it here.</p>}
        {routines.map((r) => (
          <div key={r.id} className="flex items-center justify-between gap-2 rounded border border-stone-200 p-2 text-sm dark:border-stone-700">
            <span className="min-w-0">
              <span className="font-semibold">{r.name}</span>{" "}
              <span className="text-stone-500">· tools: {r.tools.join(", ") || "none"}{r.approved.length > 0 && ` · pre-approved: ${r.approved.join(", ")}`}</span>
            </span>
            <span className="flex shrink-0 gap-2">
              <button className={ui.button} onClick={() => call("routines.run", { id: r.id }).catch(() => {})}>Run now</button>
              <button className={ui.button} onClick={() => confirm(`Delete routine ${r.name}?`) && call("routines.delete", { id: r.id }).catch(() => {})}>Delete</button>
            </span>
          </div>
        ))}
      </section>
      <h3 className="font-semibold">Schedules</h3>
      {list.length === 0 && <p className="text-sm text-stone-500">No schedules yet.</p>}
      <ul className="space-y-2">
        {list.map((s) => (
          <li key={s.id} className="rounded border border-stone-200 p-2 text-sm dark:border-stone-700">
            <div className="flex items-center justify-between gap-2">
              <span><span className="font-mono">{s.kind}</span> {s.spec} <span className="text-stone-500">· {s.enabled ? when(s) : "paused"}</span>
                {s.routine_id !== null && <span className="text-stone-500"> · routine {routines.find((r) => r.id === s.routine_id)?.name ?? s.routine_id}</span>}</span>
              <span className="flex shrink-0 gap-2">
                <button className={ui.button} onClick={() => call("schedules.update", { id: s.id, enabled: !s.enabled }).catch(() => {})}>
                  {s.enabled ? "Pause" : "Resume"}
                </button>
                <button className={ui.button} onClick={() => confirm("Delete this schedule?") && call("schedules.delete", { id: s.id }).catch(() => {})}>
                  Delete
                </button>
              </span>
            </div>
            <p className="mt-1 whitespace-pre-wrap text-stone-600 dark:text-stone-400">{s.prompt}</p>
          </li>
        ))}
      </ul>
      <form onSubmit={add} className="space-y-2 border-t border-stone-200 pt-4 dark:border-stone-700">
        <h3 className="font-semibold">New schedule</h3>
        <div className="flex gap-2">
          <select aria-label="Kind" className={`${ui.input} w-36`} value={f.kind} onChange={(e) => setF({ ...f, kind: e.target.value as Schedule["kind"] })}>
            <option value="cron">Cron</option>
            <option value="interval">Every N seconds</option>
            <option value="file">When files change</option>
            <option value="email">New email</option>
            <option value="calendar">Event soon</option>
            <option value="github">New GitHub issue</option>
          </select>
          <input aria-label="When" className={ui.input} required placeholder={HINT[f.kind]} value={f.spec} onChange={(e) => setF({ ...f, spec: e.target.value })} />
          {f.kind === "file" && (
            <button type="button" className={ui.button} onClick={async () => { const p = await pickFolder(); if (p) setF({ ...f, spec: p }); }}>Choose…</button>
          )}
        </div>
        {routines.length > 0 && (
          <select aria-label="Routine" className={ui.input} value={f.routine_id ?? ""} onChange={(e) => setF({ ...f, routine_id: e.target.value ? Number(e.target.value) : null })}>
            <option value="">Write a prompt</option>
            {routines.map((r) => <option key={r.id} value={r.id}>Run routine: {r.name}</option>)}
          </select>
        )}
        {f.routine_id === null && (
          <textarea aria-label="What the agent should do" className={ui.input} rows={3} required placeholder="What should the agent do each time?"
            value={f.prompt} onChange={(e) => setF({ ...f, prompt: e.target.value })} />
        )}
        {error && <p role="alert" className="text-sm text-red-600">{error}</p>}
        <button className={ui.primary}>Add schedule</button>
      </form>
    </div>
  );
}
