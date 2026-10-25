import { useEffect, useRef, useState } from "react";
import { Agent, Note, Room, RoomMessage, Task, onEvent, rpc, sqlTime, ui } from "./rpc";

/** Create a room: 2–6 agents, one of them the coordinator. */
export function RoomForm({ agents, onCreated }: { agents: Agent[]; onCreated: (id: number) => void }) {
  const [f, setF] = useState({ name: "", agent_ids: [] as number[], coordinator_id: 0 });
  const [error, setError] = useState("");
  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    rpc<number>("rooms.create", f).then(onCreated, (e) => setError(String(e)));
  };
  return (
    <form onSubmit={submit} className="mx-auto max-w-lg space-y-3 p-6">
      <h2 className="text-lg font-semibold">New room</h2>
      <input aria-label="Room name" className={ui.input} required placeholder="name, e.g. launch team" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} />
      <fieldset className="space-y-1">
        <legend className="text-sm">Members (2–6) and coordinator</legend>
        {agents.map((a) => (
          <div key={a.id} className="flex items-center gap-3 text-sm">
            <label className="flex items-center gap-1">
              <input type="checkbox" checked={f.agent_ids.includes(a.id)}
                onChange={(e) => setF({ ...f, agent_ids: e.target.checked ? [...f.agent_ids, a.id] : f.agent_ids.filter((i) => i !== a.id) })} />
              {a.name}
            </label>
            {f.agent_ids.includes(a.id) && (
              <label className="flex items-center gap-1 text-stone-500">
                <input type="radio" name="coordinator" checked={f.coordinator_id === a.id} onChange={() => setF({ ...f, coordinator_id: a.id })} />
                coordinator
              </label>
            )}
          </div>
        ))}
      </fieldset>
      {error && <p role="alert" className="text-sm text-red-600">{error}</p>}
      <button className={ui.primary}>Create room</button>
    </form>
  );
}

/** A room: the conversation, its task board and its shared memory. */
export default function RoomView({ room, agents, onDeleted }: { room: Room; agents: Agent[]; onDeleted: () => void }) {
  const [msgs, setMsgs] = useState<RoomMessage[]>([]);
  const [tasks, setTasks] = useState<Task[]>([]);
  const [notes, setNotes] = useState<Note[]>([]);
  const [text, setText] = useState("");
  const [task, setTask] = useState("");
  const [status, setStatus] = useState("");
  const end = useRef<HTMLDivElement>(null);
  const name = (id: number | null) => (id === null ? "You" : agents.find((a) => a.id === id)?.name ?? `agent ${id}`);
  const load = () => {
    rpc<RoomMessage[]>("rooms.messages", { room_id: room.id }).then(setMsgs);
    rpc<Task[]>("tasks.list", { room_id: room.id }).then(setTasks);
    // room memory is stored under the negative room id
    rpc<Note[]>("notes.list", { agent_id: -room.id, query: "" }).then(setNotes);
  };
  useEffect(() => {
    load();
    const un = onEvent(({ method, params }) => {
      const p = params as unknown as { room_id?: number; reason?: string };
      if (p.room_id !== room.id) return;
      if (method === "room.stopped") setStatus(p.reason ?? "");
      if (method.startsWith("room.")) load();
    });
    return () => void un.then((f) => f());
  }, [room.id]);
  useEffect(() => end.current?.scrollIntoView(), [msgs.length]);
  const send = (e: React.FormEvent) => {
    e.preventDefault();
    setStatus("");
    rpc("rooms.post", { room_id: room.id, text }).then(() => { setText(""); load(); }, (e) => setStatus(String(e)));
  };
  const saveTask = (p: object) => rpc("tasks.save", { room_id: room.id, ...p }).then(load, (e) => setStatus(String(e)));
  const members = room.agent_ids.map((id) => `@${name(id).replace(/ /g, "")}${id === room.coordinator_id ? " (coordinator)" : ""}`).join(", ");
  return (
    <div className="flex h-full">
      <div className="flex min-w-0 flex-1 flex-col">
        <header className="flex items-center justify-between border-b border-stone-200 px-4 py-2 text-sm dark:border-stone-700">
          <span><span className="font-semibold">{room.name}</span> <span className="text-stone-500">· {members}</span></span>
          <button className={ui.button} onClick={() => confirm(`Delete room ${room.name}?`) && rpc("rooms.delete", { room_id: room.id }).then(onDeleted)}>Delete</button>
        </header>
        <div className="flex-1 space-y-3 overflow-y-auto p-4">
          {msgs.length === 0 && <p className="text-sm text-stone-500">Say what you need. Mention @Name to ask a specific agent; otherwise the coordinator answers.</p>}
          {msgs.map((m) => (
            <div key={m.id} className={m.agent_id === null ? "ml-auto max-w-[80%] rounded bg-amber-50 p-2 dark:bg-stone-800" : "max-w-[80%] rounded border border-stone-200 p-2 dark:border-stone-700"}>
              <p className="text-xs text-stone-500">{name(m.agent_id)} · {sqlTime(m.created_at).toLocaleTimeString()}</p>
              <p className="whitespace-pre-wrap text-sm">{m.text}</p>
            </div>
          ))}
          <div ref={end} />
        </div>
        {status && <p role="status" className="px-4 text-sm text-amber-700">{status}</p>}
        <form onSubmit={send} className="flex gap-2 border-t border-stone-200 p-3 dark:border-stone-700">
          <input aria-label="Message the room" className={ui.input} required value={text} onChange={(e) => setText(e.target.value)} placeholder="Message the room" />
          <button className={ui.primary}>Send</button>
        </form>
      </div>
      <aside className="w-72 shrink-0 space-y-4 overflow-y-auto border-l border-stone-200 p-3 text-sm dark:border-stone-700">
        <section className="space-y-2">
          <h3 className="font-semibold">Task board</h3>
          <ul className="space-y-1">
            {tasks.map((t) => (
              <li key={t.id} className="flex items-center justify-between gap-2">
                <span className={t.status === "done" ? "text-stone-400 line-through" : ""}>{t.title}{t.assignee_id !== null && <span className="text-stone-500"> → {name(t.assignee_id)}</span>}</span>
                <select aria-label={`Status of ${t.title}`} className="rounded border border-stone-300 bg-white px-1 dark:border-stone-700 dark:bg-stone-800"
                  value={t.status} onChange={(e) => saveTask({ id: t.id, status: e.target.value })}>
                  <option value="todo">todo</option><option value="doing">doing</option><option value="done">done</option>
                </select>
              </li>
            ))}
          </ul>
          <form onSubmit={(e) => { e.preventDefault(); saveTask({ title: task }).then(() => setTask("")); }} className="flex gap-1">
            <input aria-label="New task" className={ui.input} required value={task} onChange={(e) => setTask(e.target.value)} placeholder="New task" />
            <button className={ui.button}>Add</button>
          </form>
        </section>
        <section className="space-y-1">
          <h3 className="font-semibold">Room memory</h3>
          {notes.length === 0 && <p className="text-stone-500">Nothing saved yet.</p>}
          {notes.map((n) => (
            <div key={n.id} className="flex items-start justify-between gap-1">
              <p className="whitespace-pre-wrap">{n.text}</p>
              <button className="text-xs text-stone-500" aria-label="Delete note" onClick={() => rpc("notes.delete", { note_id: n.id }).then(load)}>✕</button>
            </div>
          ))}
        </section>
      </aside>
    </div>
  );
}
