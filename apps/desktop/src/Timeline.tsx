import { useEffect, useState } from "react";
import {
  ChevronRightIcon, CpuIcon, GlobeIcon, HistoryIcon, ShieldAlertIcon,
  ShieldCheckIcon, Undo2Icon, WorkflowIcon, WrenchIcon,
} from "lucide-react";
import { Action, Agent, ModelCall, Run, confirmAction, onEvent, rpc, sqlTime } from "./rpc";
import { Dialog, DialogContent, DialogDescription, DialogFooter, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { TOOL, keyArg } from "./Chat";
import { EmptyState, ErrorNote } from "@/components/panel";
import { toast } from "sonner";
import { Button } from "@/components/ui/button";

type Item = { at: number; call?: ModelCall; action?: Action };

/** A browser screenshot the daemon saved in the agent's scratch folder. */
function Shot({ agentId, path }: { agentId: number; path: string }) {
  const [src, setSrc] = useState("");
  useEffect(() => void rpc<string>("files.image", { agent_id: agentId, path }).then(setSrc, () => {}), [path]);
  return src ? <img src={src} alt="Browser screenshot" className="mt-2 max-h-48 rounded-lg border" /> : null;
}

const triggerLabel = (t: string) => (t === "chat" ? "Chat" : t.startsWith("schedule:") ? `Schedule #${t.slice(9)}`
  : t.startsWith("routine:") ? `Routine #${t.slice(8)}` : t);
const ago = (d: Date) => {
  const s = (Date.now() - d.getTime()) / 1000;
  return s < 60 ? "just now" : s < 3600 ? `${Math.floor(s / 60)} min ago` : s < 86400 ? `${Math.floor(s / 3600)} h ago` : d.toLocaleDateString();
};
const STATUS: Record<string, string> = { done: "bg-success", running: "lamp-on" };

/** The Activity tab: every run with its model calls and actions, plus undo. The journal is append-only. */
export default function Timeline({ agent }: { agent: Agent }) {
  const [runs, setRuns] = useState<Run[]>([]);
  const [actions, setActions] = useState<Action[]>([]);
  const [calls, setCalls] = useState<ModelCall[]>([]);
  const [chain, setChain] = useState("");
  const [error, setError] = useState("");

  const load = () => {
    const fail = (e: unknown) => setError(String(e));
    rpc<Run[]>("runs.list", { agent_id: agent.id }).then(setRuns, fail);
    rpc<Action[]>("actions.list", { agent_id: agent.id }).then(setActions, fail);
    rpc<ModelCall[]>("model_calls.list", { agent_id: agent.id }).then(setCalls, fail);
    rpc<number>("journal.verify").then((n) => setChain(`Journal intact · ${n} entries`), (e) => setChain(String(e)));
  };
  useEffect(() => {
    load();
    const un = onEvent(({ method, params }) => {
      if (params?.agent_id === agent.id && (method === "chat.tool" || method === "chat.done")) load();
    });
    return () => void un.then((f) => f());
  }, [agent.id]);

  const intact = chain.startsWith("Journal intact");
  const [saving, setSaving] = useState<{ run: number; name: string } | null>(null);
  const saveRoutine = (e: React.FormEvent) => {
    e.preventDefault();
    if (!saving?.name.trim()) return;
    const name = saving.name.trim();
    rpc("routines.save", { run_id: saving.run, name }).then(() => toast.success(`Saved “${name}”. Find it under Schedules.`), (e) => setError(String(e)));
    setSaving(null);
  };
  const act = async (method: string, params: object, label: string) => {
    setError("");
    try {
      await rpc(method, params);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("force") && await confirmAction(`${msg}\n\n${label} anyway and overwrite the newer version?`, label)) {
        await rpc(method, { ...params, force: true }).catch((e) => setError(String(e)));
      } else setError(msg);
    }
    load();
  };

  return (
    <section aria-label="Activity" className="h-full overflow-y-auto">
      <div className="mx-auto max-w-3xl px-8 py-8">
        <header className="flex items-end justify-between gap-4">
          <div>
            <h2 className="title text-2xl">Activity</h2>
            <p className="mt-1 text-muted-foreground">Everything {agent.name} did, with undo for file and email changes.</p>
          </div>
          <p title="Append-only and hash-chained: the agent can't edit it."
            className={`flex shrink-0 items-center gap-1.5 text-xs ${intact ? "text-muted-foreground" : "text-destructive"}`}>
            {intact ? <ShieldCheckIcon className="size-4" /> : <ShieldAlertIcon className="size-4" />}
            {chain || "Checking the journal…"}
          </p>
        </header>
        <div className="mt-6 overflow-hidden rounded-xl border bg-card">
        {error && <div className="p-3"><ErrorNote msg={error} /></div>}
        {runs.length === 0 && (
          <EmptyState className="rounded-none border-0" icon={<HistoryIcon />} title="No activity yet">Each model call and action shows up here. File and email changes can be undone.</EmptyState>
        )}
        {runs.map((run, i) => {
          const runCalls = calls.filter((c) => c.run_id === run.id);
          const runActions = actions.filter((a) => a.run_id === run.id);
          const items: Item[] = [
            ...runCalls.map((call) => ({ at: sqlTime(call.created_at).getTime(), call })),
            ...runActions.map((action) => ({ at: action.created_at, action })),
          ].sort((a, b) => a.at - b.at);
          const tokens = runCalls.reduce((n, c) => n + (c.prompt_tokens ?? 0) + (c.completion_tokens ?? 0), 0);
          const cost = runCalls.reduce((n, c) => n + (c.cost_usd ?? 0), 0);
          const undoable = runActions.some((a) => a.undoable && !a.undone_at);
          // plugin calls and sends reach outside this machine and can't be reverted from here
          const remote = runActions.filter((a) => (a.tool.startsWith("mcp__") || a.tool === "email_send") && a.decision !== "denied" && !a.undoable).length;
                    const started = sqlTime(run.started_at);
          return (
            <details key={run.id} open={i === 0} className="group border-b last:border-b-0">
              <summary className="flex cursor-pointer list-none items-center gap-2 px-4 py-3 select-none hover:bg-accent/40">
                <ChevronRightIcon className="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-90" />
                <span className="font-medium">{triggerLabel(run.trigger)}</span>
                {run.status !== "done" && <span role="img" aria-label={run.status} title={run.status} className={`size-1.5 rounded-full ${STATUS[run.status] ?? "bg-destructive"}`} />}
                <span className="truncate text-xs text-muted-foreground" title={`${started.toLocaleString()}${tokens > 0 ? ` · ${tokens.toLocaleString()} tokens` : ""}${cost > 0 ? ` · $${cost.toFixed(4)}` : ""}`}>{ago(started)}</span>
                <span className="ml-auto shrink-0 text-xs text-muted-foreground tabular-nums">{runActions.length} action{runActions.length === 1 ? "" : "s"}</span>
              </summary>
              <ol className="px-4 pb-2 pl-10">
                {items.length === 0 && <li className="py-1 text-xs text-muted-foreground">Nothing recorded.</li>}
                {items.map(({ call, action: a }, j) =>
                  call ? (
                    <li key={`c${j}`} className="flex items-center gap-2.5 py-1 text-xs text-muted-foreground">
                      <CpuIcon className="size-3.5 shrink-0" />
                      <span className="truncate">Thought for {(call.ms / 1000).toFixed(1)} s · {call.completion_tokens ?? "?"} tokens{call.cost_usd ? ` · $${call.cost_usd.toFixed(4)}` : ""}</span>
                    </li>
                  ) : a ? (
                    <ActionRow key={a.id} a={a} agentId={agent.id} onUndo={() => act("actions.undo", { action_id: a.id }, "Undo")} />
                  ) : null,
                )}
              </ol>
              {remote > 0 && <p className="px-4 pb-2 text-xs text-muted-foreground">{remote} plugin or send action{remote === 1 ? "" : "s"} reached outside this computer and can’t be undone here. Review them above.</p>}
              {(run.status === "done" || undoable) && (
                <div className="grid grid-cols-2 gap-2 px-4 pb-3">
                  {run.status === "done" && (
                    <Button size="sm" variant="outline" onClick={() => setSaving({ run: run.id, name: "" })}><WorkflowIcon />Save as routine</Button>
                  )}
                  {undoable && <Button size="sm" variant="outline" onClick={() => act("runs.undo", { run_id: run.id }, "Undo run")}><Undo2Icon />Undo whole run</Button>}
                </div>
              )}
            </details>
          );
        })}
        </div>
      </div>
      <Dialog open={!!saving} onOpenChange={(o) => !o && setSaving(null)}>
        <DialogContent className="sm:max-w-sm">
          <form onSubmit={saveRoutine} className="space-y-4">
            <DialogTitle className="font-sans text-base font-semibold">Save as routine</DialogTitle>
            <DialogDescription>Run it again later, or from a schedule, with the same tools and approvals.</DialogDescription>
            <Input autoFocus aria-label="Routine name" placeholder="e.g. Weekly folder summary" value={saving?.name ?? ""}
              onChange={(e) => setSaving((s) => s && { ...s, name: e.target.value })} />
            <DialogFooter>
              <Button type="button" variant="ghost" onClick={() => setSaving(null)}>Cancel</Button>
              <Button disabled={!saving?.name.trim()}>Save</Button>
            </DialogFooter>
          </form>
        </DialogContent>
      </Dialog>
    </section>
  );
}

function ActionRow({ a, agentId, onUndo }: { a: Action; agentId: number; onUndo: () => void }) {
  const t = TOOL[a.tool] ?? { label: a.tool === "net" ? "Network connection" : a.tool, icon: a.tool === "net" ? GlobeIcon : WrenchIcon };
  const arg = keyArg(a.input);
  return (
    <li className="flex items-start gap-2.5 py-1">
      <t.icon className="mt-0.5 size-3.5 shrink-0 text-muted-foreground" />
      <div className="min-w-0 flex-1">
        <p className="flex items-center gap-1.5">
          <span className="truncate">{t.label}</span>
          {a.decision !== "auto" && <span className="shrink-0 text-xs text-muted-foreground">· {a.decision}</span>}
        </p>
        {arg && <code className="block truncate text-xs text-muted-foreground" title={arg}>{arg}</code>}
        {a.output && (
          <details className="group/out mt-1">
            <summary className="flex w-fit cursor-pointer list-none items-center gap-0.5 text-xs text-muted-foreground select-none hover:text-foreground">
              <ChevronRightIcon className="size-3 transition-transform group-open/out:rotate-90" />Output
            </summary>
            <pre className="mt-1 max-h-48 overflow-auto whitespace-pre-wrap rounded-md bg-muted p-2 text-xs">{a.output}</pre>
          </details>
        )}
        {[...a.output.matchAll(/\[screenshot: ([^\]]+)\]/g)].map((m) => <Shot key={m[1]} agentId={agentId} path={m[1]} />)}
      </div>
      {a.undone_at ? (
        <span className="mt-0.5 shrink-0 text-xs text-muted-foreground">Undone</span>
      ) : a.undoable ? (
        <Button size="icon-xs" variant="ghost" aria-label={`Undo: ${t.label}`} title="Undo" className="shrink-0" onClick={onUndo}><Undo2Icon /></Button>
      ) : null}
    </li>
  );
}
