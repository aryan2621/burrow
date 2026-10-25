import { useEffect, useState } from "react";
import { Agent, rpc } from "./rpc";
import AgentForm from "./AgentForm";
import Chat from "./Chat";
import Activity from "./Activity";

export default function App() {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [error, setError] = useState("");
  const [tab, setTab] = useState<"chat" | "activity">("chat");
  const [sandbox, setSandbox] = useState<string | null | undefined>(undefined);

  const load = () => rpc<Agent[]>("agents.list").then(setAgents, (e) => setError(String(e)));
  useEffect(() => {
    load();
    rpc<{ sandbox: string | null }>("status").then((s) => setSandbox(s.sandbox), () => {});
  }, []);

  const agent = agents.find((a) => a.id === selected);
  return (
    <div className="flex h-full">
      <nav className="w-56 shrink-0 border-r border-stone-200 p-2 dark:border-stone-700">
        <button onClick={() => setSelected(null)} className="mb-2 w-full rounded bg-amber-800 py-1.5 text-white">
          New agent
        </button>
        {agents.map((a) => (
          <button key={a.id} onClick={() => setSelected(a.id)}
            className={`block w-full rounded px-2 py-1.5 text-left ${a.id === selected ? "bg-stone-200 dark:bg-stone-800" : ""}`}>
            {a.name}
          </button>
        ))}
        {error && <p role="alert" className="mt-2 text-sm text-red-600">{error}</p>}
        {sandbox !== undefined && (
          <p className="mt-4 text-xs text-stone-500">
            Sandbox: {sandbox ?? "none installed — shell disabled. Install Apple container, Podman or Docker."}
          </p>
        )}
      </nav>
      <main className="min-w-0 flex-1">
        {agent ? (
          <div className="flex h-full flex-col">
            <div role="tablist" className="flex gap-4 border-b border-stone-200 px-4 text-sm dark:border-stone-700">
              {(["chat", "activity"] as const).map((t) => (
                <button key={t} role="tab" aria-selected={tab === t} onClick={() => setTab(t)}
                  className={`py-2 capitalize ${tab === t ? "border-b-2 border-amber-800 font-semibold" : "text-stone-500"}`}>
                  {t}
                </button>
              ))}
            </div>
            <div className="min-h-0 flex-1">
              {tab === "chat" ? (
                <Chat agent={agent} onDeleted={() => { setSelected(null); load(); }} />
              ) : (
                <Activity key={agent.id} agent={agent} onSaved={load} />
              )}
            </div>
          </div>
        ) : (
          <AgentForm onCreated={(id) => { load(); setSelected(id); }} />
        )}
      </main>
    </div>
  );
}
