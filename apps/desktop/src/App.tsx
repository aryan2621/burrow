import { useEffect, useState } from "react";
import { Agent, Room, rpc } from "./rpc";
import AgentForm from "./AgentForm";
import Chat from "./Chat";
import Timeline from "./Timeline";
import Schedules from "./Schedules";
import Tools from "./Tools";
import Settings from "./Settings";
import Onboarding from "./Onboarding";
import { PermissionsTab } from "./Permissions";
import Memory from "./Memory";
import RoomView, { RoomForm } from "./Rooms";
import Artifacts from "./Artifacts";

const TABS = ["chat", "timeline", "schedules", "tools", "memory", "permissions"] as const;

export default function App() {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [error, setError] = useState("");
  const [tab, setTab] = useState<(typeof TABS)[number]>("chat");
  const [view, setView] = useState<"agents" | "rooms" | "artifacts" | "settings" | "onboarding" | null>(null);
  const [rooms, setRooms] = useState<Room[]>([]);
  const [room, setRoom] = useState<number | null>(null);
  const loadRooms = () => rpc<Room[]>("rooms.list").then(setRooms, () => {});
  useEffect(() => void loadRooms(), []);
  const [sandbox, setSandbox] = useState<string | null | undefined>(undefined);

  const load = () => rpc<Agent[]>("agents.list").then(setAgents, (e) => setError(String(e)));
  useEffect(() => {
    rpc<{ sandbox: string | null }>("status").then((s) => setSandbox(s.sandbox), () => {});
    // first run: no agents and never onboarded; existing installs skip it
    Promise.all([rpc<Agent[]>("agents.list"), rpc<string | null>("settings.get", { key: "onboarded" })]).then(
      ([a, done]) => { setAgents(a); setView(!done && a.length === 0 ? "onboarding" : "agents"); },
      (e) => { setError(String(e)); setView("agents"); },
    );
  }, []);

  if (view === null) return null;
  if (view === "onboarding")
    return <Onboarding onDone={(id) => { load(); setSelected(id); setView("agents"); }} />;

  const agent = agents.find((a) => a.id === selected);
  return (
    <div className="flex h-full">
      <nav className="w-56 shrink-0 border-r border-stone-200 p-2 dark:border-stone-700">
        <button onClick={() => { setSelected(null); setView("agents"); }} className="mb-2 w-full rounded bg-amber-800 py-1.5 text-white">
          New agent
        </button>
        {agents.map((a) => (
          <button key={a.id} onClick={() => { setSelected(a.id); setView("agents"); }}
            className={`block w-full rounded px-2 py-1.5 text-left ${a.id === selected && view === "agents" ? "bg-stone-200 dark:bg-stone-800" : ""}`}>
            {a.name}
          </button>
        ))}
        <p className="mt-4 px-2 text-xs uppercase text-stone-500">Rooms</p>
        {rooms.map((r) => (
          <button key={r.id} onClick={() => { setRoom(r.id); setView("rooms"); }}
            className={`block w-full rounded px-2 py-1.5 text-left ${r.id === room && view === "rooms" ? "bg-stone-200 dark:bg-stone-800" : ""}`}>
            # {r.name}
          </button>
        ))}
        <button onClick={() => { setRoom(null); setView("rooms"); }} className="block w-full rounded px-2 py-1 text-left text-sm text-stone-500">+ New room</button>
        <button onClick={() => setView("artifacts")}
          className={`mt-4 block w-full rounded px-2 py-1.5 text-left text-sm ${view === "artifacts" ? "bg-stone-200 dark:bg-stone-800" : ""}`}>
          Artifacts
        </button>
        <button onClick={() => setView("settings")}
          className={`mt-4 block w-full rounded px-2 py-1.5 text-left text-sm ${view === "settings" ? "bg-stone-200 dark:bg-stone-800" : ""}`}>
          Settings
        </button>
        {error && <p role="alert" className="mt-2 text-sm text-red-600">{error}</p>}
        {sandbox !== undefined && (
          <p className="mt-4 text-xs text-stone-500">
            Sandbox: {sandbox ?? "none installed — shell disabled. Install Apple container, Podman or Docker."}
          </p>
        )}
      </nav>
      <main className="min-w-0 flex-1">
        {view === "settings" ? (
          <Settings />
        ) : view === "artifacts" ? (
          <Artifacts agents={agents} />
        ) : view === "rooms" ? (
          rooms.find((r) => r.id === room) ? (
            <RoomView key={room} room={rooms.find((r) => r.id === room)!} agents={agents} onDeleted={() => { setRoom(null); loadRooms(); }} />
          ) : (
            <RoomForm agents={agents} onCreated={(id) => { loadRooms(); setRoom(id); }} />
          )
        ) : agent ? (
          <div className="flex h-full flex-col">
            <div role="tablist" className="flex gap-4 border-b border-stone-200 px-4 text-sm dark:border-stone-700">
              {TABS.map((t) => (
                <button key={t} role="tab" aria-selected={tab === t} onClick={() => setTab(t)}
                  className={`py-2 capitalize ${tab === t ? "border-b-2 border-amber-800 font-semibold" : "text-stone-500"}`}>
                  {t}
                </button>
              ))}
            </div>
            <div className="min-h-0 flex-1">
              {tab === "chat" && <Chat agent={agent} onDeleted={() => { setSelected(null); load(); }} />}
              {tab === "timeline" && <Timeline agent={agent} />}
              {tab === "schedules" && <Schedules agent={agent} />}
              {tab === "tools" && <Tools agent={agent} />}
              {tab === "memory" && <Memory key={agent.id} agent={agent} />}
              {tab === "permissions" && <PermissionsTab key={agent.id} agent={agent} onSaved={load} />}
            </div>
          </div>
        ) : (
          <AgentForm onCreated={(id) => { load(); setSelected(id); }} />
        )}
      </main>
    </div>
  );
}
