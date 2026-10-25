import { useEffect, useRef, useState } from "react";
import { Agent, Approval, Message, onEvent, rpc } from "./rpc";

export default function Chat({ agent, onDeleted }: { agent: Agent; onDeleted: () => void }) {
  const [msgs, setMsgs] = useState<Message[]>([]);
  const [stream, setStream] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [text, setText] = useState("");
  const [approvals, setApprovals] = useState<Approval[]>([]);
  const end = useRef<HTMLDivElement>(null);

  const load = () => rpc<Message[]>("chat.history", { agent_id: agent.id }).then(setMsgs);
  const loadApprovals = () =>
    rpc<Approval[]>("approvals.pending").then((a) => setApprovals(a.filter((x) => x.agent_id === agent.id)));

  useEffect(() => {
    setStream("");
    setError("");
    load();
    loadApprovals();
    rpc<number[]>("chat.busy").then((b) => setBusy(b.includes(agent.id)));
    const un = onEvent(({ method, params }) => {
      if (params.agent_id !== agent.id) return;
      if (method === "chat.delta") setStream((s) => s + params.text);
      if (method === "chat.tool") { setStream(""); load(); }
      if (method === "approval.requested") setApprovals((a) => [...a, params as Approval]);
      if (method === "chat.done") {
        setStream("");
        setBusy(false);
        setError(params.error ?? "");
        load();
        loadApprovals();
      }
    });
    return () => void un.then((f) => f());
  }, [agent.id]);

  useEffect(() => end.current?.scrollIntoView(), [msgs, stream]);

  const send = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!text.trim() || busy) return;
    setError("");
    try {
      await rpc("chat.send", { agent_id: agent.id, text });
      setBusy(true);
      setText("");
      load();
    } catch (e) {
      setError(String(e));
    }
  };
  const decide = async (a: Approval, allow: boolean, remember = false) => {
    setApprovals((list) => list.filter((x) => x.approval_id !== a.approval_id));
    await rpc("approvals.decide", { approval_id: a.approval_id, allow, remember }).catch((e) => setError(String(e)));
  };
  const kill = () => rpc("agents.kill", { agent_id: agent.id }).catch((e) => setError(String(e)));
  const remove = async () => {
    if (!confirm(`Delete ${agent.name} and its history?`)) return;
    await rpc("agents.delete", { agent_id: agent.id }).then(onDeleted, (e) => setError(String(e)));
  };

  return (
    <div className="flex h-full flex-col">
      <header className="flex items-center justify-between border-b border-stone-200 px-4 py-2 dark:border-stone-700">
        <div>
          <h2 className="font-semibold">{agent.name}</h2>
          <p className="text-xs text-stone-500">{agent.model_ref} · {agent.folders.join(", ")}</p>
        </div>
        <div className="flex gap-3">
          {busy && <button onClick={kill} className="text-sm font-semibold text-red-700">Kill</button>}
          <button onClick={remove} className="text-sm text-red-700">Delete</button>
        </div>
      </header>
      <div className="flex-1 space-y-3 overflow-y-auto p-4">
        {msgs.map((m, i) => <Bubble key={i} m={m} />)}
        {stream && <Bubble m={{ role: "assistant", content: stream }} />}
        {approvals.map((a) => (
          <div key={a.approval_id} role="alertdialog" aria-label="Approval needed"
            className="rounded border border-amber-600 bg-amber-50 p-3 dark:bg-amber-950">
            <p className="text-sm font-semibold">
              {a.tool === "net" ? `${agent.name}'s sandbox wants to connect to ${a.args.host}` : `${agent.name} wants to run ${a.tool}`}
            </p>
            <pre className="my-2 max-h-40 overflow-auto whitespace-pre-wrap text-xs">{JSON.stringify(a.args, null, 2)}</pre>
            <div className="flex gap-2 text-sm">
              <button onClick={() => decide(a, true)} className="rounded bg-amber-800 px-3 py-1 text-white">Allow once</button>
              <button onClick={() => decide(a, true, true)} className="rounded border border-amber-800 px-3 py-1">
                {a.tool === "http_fetch" || a.tool === "net" ? "Always allow this site" : "Always allow"}
              </button>
              <button onClick={() => decide(a, false)} className="rounded px-3 py-1 text-red-700">Deny</button>
            </div>
          </div>
        ))}
        {busy && !stream && approvals.length === 0 && <p className="text-sm text-stone-500">Working…</p>}
        {error && <p role="alert" className="text-sm text-red-600">{error}</p>}
        <div ref={end} />
      </div>
      <form onSubmit={send} className="flex gap-2 border-t border-stone-200 p-3 dark:border-stone-700">
        <input aria-label="Message" value={text} onChange={(e) => setText(e.target.value)} placeholder="Message…"
          className="flex-1 rounded border border-stone-300 bg-white px-2 py-1.5 dark:border-stone-700 dark:bg-stone-800" />
        <button disabled={busy} className="rounded bg-amber-800 px-4 text-white disabled:opacity-50">Send</button>
      </form>
    </div>
  );
}

function Bubble({ m }: { m: Message }) {
  if (m.role === "tool")
    return <pre className="max-h-40 overflow-auto rounded bg-stone-200 p-2 text-xs dark:bg-stone-800">{m.content}</pre>;
  return (
    <div className={m.role === "user" ? "ml-auto max-w-[80%] rounded bg-amber-100 p-2 dark:bg-amber-950" : "max-w-[80%]"}>
      {m.content && <p className="whitespace-pre-wrap">{m.content}</p>}
      {m.tool_calls?.map((c, i) => (
        <p key={i} className="font-mono text-xs text-stone-500">→ {c.function.name}({c.function.arguments})</p>
      ))}
    </div>
  );
}
