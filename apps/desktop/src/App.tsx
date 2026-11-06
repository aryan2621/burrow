import { useEffect, useState } from "react";
import { Agent, Approval, confirmAction, onEvent, rpc } from "./rpc";
import AgentForm from "./AgentForm";
import Chat from "./Chat";
import Timeline from "./Timeline";
import Settings from "./Settings";
import Onboarding from "./Onboarding";
import Team from "./Team";
import { AgentSettings } from "./Permissions";
import Tools from "./Tools";
import Discover from "./Discover";
import Memory from "./Memory";
import { RoutinesPage, SchedulesPage } from "./Schedules";
import { BrainIcon, CalendarClockIcon, CompassIcon, HistoryIcon, MessageSquareIcon, PlusIcon, Settings2Icon, SlidersHorizontalIcon, SquareIcon, UsersIcon, WrenchIcon } from "lucide-react";
import { Avatar, AvatarKind, AvatarPicker, avatarFor } from "./Avatars";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Mark } from "./Icons";
import { Button } from "@/components/ui/button";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { ErrorNote } from "@/components/panel";
import { Toaster } from "sonner";


type Tab = "chat" | "activity" | "tools" | "discover" | "schedules" | "memory";
const TABS: { id: Tab; label: string; icon: React.ComponentType<{ className?: string }> }[] = [
  { id: "chat", label: "Chat", icon: MessageSquareIcon },
  { id: "activity", label: "Activity", icon: HistoryIcon },
  { id: "tools", label: "Tools", icon: WrenchIcon },
  { id: "discover", label: "Discover", icon: CompassIcon },
  { id: "schedules", label: "Schedules", icon: CalendarClockIcon },
  { id: "memory", label: "Memory", icon: BrainIcon },
];

const TAB_HELP: Record<Tab, string> = {
  chat: "", activity: "",
  tools: "MCP servers and plugins this agent can call. Command servers run inside its sandbox, and tools ask before running unless the server marks them read-only. Changes apply from the next message.",
  discover: "Find community MCP servers in the official registry and add them to this agent.",
  schedules: "Let it work on its own: on a timer, or when a file or email arrives. Routines are saved runs you can replay.",
  memory: "Notes this agent keeps and searches across chats. Only this agent reads them, and you can edit or delete any.",
};

export default function App() {
  const [agents, setAgents] = useState<Agent[]>([]);
  const [selected, setSelected] = useState<number | null>(null);
  const [error, setError] = useState("");
  const [tab, setTab] = useState<Tab>("chat");
  const [view, setView] = useState<"agents" | "onboarding" | "team" | null>(null);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [agentSettingsOpen, setAgentSettingsOpen] = useState(false);
  const [sandbox, setSandbox] = useState<string | null | undefined>(undefined);
  const [isolation, setIsolation] = useState("");
  const [busy, setBusy] = useState<Set<number>>(new Set());
  const [avatars, setAvatars] = useState<Record<string, string>>({});
  const [pickAvatar, setPickAvatar] = useState(false);
  const saveAvatar = (id: number, kind: AvatarKind) => {
    const next = { ...avatars, [id]: kind };
    setAvatars(next);
    rpc("settings.set", { key: "avatars", value: JSON.stringify(next) }).catch((e) => setError(String(e)));
  };

  const [pending, setPending] = useState<Record<number, number>>({});
  const refreshPending = () => rpc<Approval[]>("approvals.pending").then((l) => {
    const n: Record<number, number> = {};
    for (const a of l) n[a.agent_id] = (n[a.agent_id] ?? 0) + 1;
    setPending(n);
  }, () => {});

  const load = () => rpc<Agent[]>("agents.list").then(setAgents, (e) => setError(String(e)));
  useEffect(() => {
    rpc<{ sandbox: string | null; isolation?: string }>("status").then((s) => { setSandbox(s.sandbox); setIsolation(s.isolation ?? ""); }, () => {});
    // first run: no agents and never onboarded; existing installs skip it
    Promise.all([rpc<Agent[]>("agents.list"), rpc<string | null>("settings.get", { key: "onboarded" })]).then(
      ([a, done]) => { setAgents(a); setSelected(a[0]?.id ?? null); setView(!done && a.length === 0 ? "onboarding" : "agents"); },
      (e) => { setError(String(e)); setView("agents"); },
    );
    rpc<number[]>("chat.busy").then((b) => setBusy(new Set(b)), () => {});
    refreshPending();
    rpc<string | null>("settings.get", { key: "avatars" }).then((v) => v && setAvatars(JSON.parse(v)), () => {});
    const un = onEvent(({ method, params }) => {
      const id = params?.agent_id as number;
      if (method === "chat.delta" || method === "chat.tool" || method === "schedule.fired")
        setBusy((b) => (b.has(id) ? b : new Set(b).add(id)));
      if (method === "approval.requested" || method === "chat.done" || method === "chat.tool") refreshPending();
      if (method === "chat.done") setBusy((b) => { const n = new Set(b); n.delete(id); return n; });
    });
    return () => void un.then((f) => f());
  }, []);

  if (view === null) return null;
  if (view === "onboarding")
    return <Onboarding onDone={(id) => { load(); setSelected(id); setView("agents"); }} />;

  const agent = agents.find((a) => a.id === selected);
  const kind = agent && avatarFor(agent.id, avatars[agent.id]);
  const remove = async (a: Agent) => {
    if (!(await confirmAction(`Delete ${a.name} and its chat history? The action log is kept.`))) return;
    await rpc("agents.delete", { agent_id: a.id }).then(() => { setAgentSettingsOpen(false); setSelected(null); load(); }, (e) => setError(String(e)));
  };
  const navItem = (active: boolean) =>
    `flex h-9 w-full items-center gap-2.5 rounded-md px-2.5 text-left text-sm ${active
      ? "bg-accent text-foreground"
      : "text-muted-foreground hover:bg-accent hover:text-foreground"}`;
  const pick = (id: number) => { setSelected(id); setTab("chat"); setView("agents"); };

  return (
    <TooltipProvider delayDuration={400}>
    <Toaster richColors closeButton position="bottom-center" />
    <div className="flex h-full overflow-hidden">
      <nav aria-label="Agents" className="flex w-56 shrink-0 flex-col border-r bg-muted/50 p-3">
        <div className="flex items-center gap-2 px-1.5 pt-1">
          <Mark className="size-5" />
          <span className="font-semibold tracking-tight">Burrow</span>
        </div>
        <div className="mt-4 flex gap-1.5">
          <Button size="sm" className="flex-1" onClick={() => { setSelected(null); setView("agents"); }}><PlusIcon />New agent</Button>
          <Tooltip>
            <TooltipTrigger asChild>
              <Button size="icon-sm" variant={view === "team" ? "secondary" : "outline"} aria-label="New team" onClick={() => setView("team")}><UsersIcon /></Button>
            </TooltipTrigger>
            <TooltipContent>New team</TooltipContent>
          </Tooltip>
        </div>
        <div className="mt-5 mb-1 flex items-center justify-between px-2.5">
          <p className="text-xs font-medium text-muted-foreground">Agents</p>
          {busy.size > 0 && (
            <Tooltip>
              <TooltipTrigger asChild>
                <Button size="icon-xs" variant="ghost" aria-label="Stop all agents" onClick={() => rpc("agents.stop_all").catch((e) => setError(String(e)))}><SquareIcon className="fill-current" /></Button>
              </TooltipTrigger>
              <TooltipContent>Stop every running agent</TooltipContent>
            </Tooltip>
          )}
        </div>
        <ul className="flex-1 space-y-0.5 overflow-y-auto">
          {agents.map((a) => (
            <li key={a.id}>
              <button onClick={() => pick(a.id)} aria-current={a.id === selected && view === "agents" ? "page" : undefined}
                className={navItem(a.id === selected && view === "agents")}>
                <Avatar kind={avatarFor(a.id, avatars[a.id])} className="size-5" />
                <span className="min-w-0 flex-1 truncate">{a.name}</span>
                {pending[a.id] ? <span role="status" aria-label={`${pending[a.id]} waiting for your approval`} className="grid min-w-4 place-items-center rounded-full bg-brand px-1 text-xs text-stone-950">{pending[a.id]}</span>
                  : busy.has(a.id) && <span role="status" aria-label="working" className="lamp-on size-2 shrink-0 rounded-full" />}
              </button>
            </li>
          ))}
          {agents.length === 0 && <li className="px-2.5 text-muted-foreground">No agents yet.</li>}
        </ul>
        <ErrorNote msg={error} className="mb-2 px-2.5 py-2 text-xs" />
        <button onClick={() => setSettingsOpen(true)} className={navItem(false)}>
          <SlidersHorizontalIcon className="size-4" /> Settings
        </button>
        {sandbox !== undefined && (
          <p className="mt-2 flex items-center gap-2 border-t px-2.5 pt-3 text-xs text-muted-foreground"
            title={isolation === "container" ? "Shared-kernel container: weaker isolation than a VM" : undefined}>
            {sandbox ? `Sandbox: ${sandbox}` : <span className="text-destructive">No sandbox, so shell is off</span>}
          </p>
        )}
      </nav>
      <main className="relative min-w-0 flex-1">
        <Settings open={settingsOpen} onOpenChange={setSettingsOpen} />
        {view === "team" ? (
          <Team agents={agents} onStarted={(id) => { load(); pick(id); }} />
        ) : agent ? (
          <div className="flex h-full min-h-0 flex-col">
            <header className="border-b px-6 pt-4">
              <div className="flex items-center justify-between gap-4">
                <div className="flex min-w-0 items-center gap-3">
                  <Tooltip>
                    <TooltipTrigger asChild>
                      <button onClick={() => setPickAvatar(true)} aria-label="Change avatar" className="rounded-[34%]">
                        <Avatar kind={kind!} className="size-9" />
                      </button>
                    </TooltipTrigger>
                    <TooltipContent>Change avatar</TooltipContent>
                  </Tooltip>
                  <div className="min-w-0">
                    <div className="flex items-center gap-2">
                      <h1 className="truncate text-lg leading-tight">{agent.name}</h1>
                      <span className="inline-flex items-center gap-1.5 text-xs text-muted-foreground">
                        {busy.has(agent.id) && <span aria-hidden="true" className="lamp-on size-1.5 rounded-full" />}
                        {busy.has(agent.id) ? "Working" : "Idle"}
                      </span>
                    </div>
                    <p className="truncate text-xs text-muted-foreground" title={agent.folders.join(", ")}>
                      <span className="font-mono">{agent.model_ref}</span>{agent.folders.length > 0 && <> · {agent.folders.join(", ")}</>}
                    </p>
                  </div>
                </div>
                <Button variant="outline" size="sm" onClick={() => setAgentSettingsOpen(true)}>
                  <Settings2Icon />Settings
                </Button>
              </div>
              <div role="tablist" aria-label="Agent sections" className="mt-3 -mb-px flex gap-1">
                {TABS.map(({ id, label, icon: I }) => (
                  <button key={id} role="tab" aria-selected={tab === id} onClick={() => setTab(id)}
                    className={`inline-flex items-center gap-1.5 border-b-2 px-3 pb-2.5 text-sm focus-visible:outline-2 focus-visible:-outline-offset-2 focus-visible:outline-ring ${tab === id
                      ? "border-primary font-medium text-foreground"
                      : "border-transparent text-muted-foreground hover:text-foreground"}`}>
                    <I className="size-4" />{label}
                    {id === "chat" && !!pending[agent.id] && <span aria-label={`${pending[agent.id]} waiting for your approval`} className="rounded-full bg-brand px-1.5 text-xs text-stone-950">{pending[agent.id]}</span>}
                  </button>
                ))}
              </div>
            </header>
            <div className="min-h-0 flex-1">
              {tab === "chat" && <Chat agent={agent} avatar={kind!} sandbox={sandbox ?? null} onDecided={refreshPending} />}
              {tab === "activity" && <Timeline key={agent.id} agent={agent} />}
              {tab !== "chat" && tab !== "activity" && (
                <div className="h-full overflow-y-auto">
                  <div className="mx-auto max-w-3xl space-y-8 px-8 py-8">
                    <header>
                      <h2 className="title text-2xl">{TABS.find((t) => t.id === tab)!.label}</h2>
                      <p className="mt-1 text-muted-foreground">{TAB_HELP[tab]}</p>
                    </header>
                    {tab === "tools" && <Tools agent={agent} />}
                    {tab === "discover" && <Discover agent={agent} />}
                    {tab === "schedules" && <><SchedulesPage agent={agent} /><RoutinesPage agent={agent} /></>}
                    {tab === "memory" && <Memory agent={agent} />}
                  </div>
                </div>
              )}
            </div>
            <Dialog open={pickAvatar} onOpenChange={setPickAvatar}>
              <DialogContent className="sm:max-w-sm">
                <DialogTitle className="font-sans text-base font-semibold">Pick {agent.name}'s avatar</DialogTitle>
                <DialogDescription>Only shown to you.</DialogDescription>
                <AvatarPicker value={kind!} onChange={(k) => { saveAvatar(agent.id, k); setPickAvatar(false); }} />
              </DialogContent>
            </Dialog>
            <AgentSettings key={agent.id} agent={agent} open={agentSettingsOpen} onOpenChange={setAgentSettingsOpen} onSaved={load} onDelete={() => remove(agent)} />
          </div>
        ) : (
          <AgentForm onCreated={(id, k) => { saveAvatar(id, k); load(); pick(id); }} />
        )}
      </main>
    </div>
    </TooltipProvider>
  );
}
