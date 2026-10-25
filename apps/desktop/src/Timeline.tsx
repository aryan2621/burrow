import { useEffect, useState } from "react";
import { Action, Agent, ModelCall, Run, onEvent, rpc, sqlTime } from "./rpc";

type Item = { at: number; call?: ModelCall; action?: Action };

/** A browser screenshot the daemon saved in the agent's scratch folder. */
function Shot({ agentId, path }: { agentId: number; path: string }) {
  const [src, setSrc] = useState("");
  useEffect(() => void rpc<string>("files.image", { agent_id: agentId, path }).then(setSrc, () => {}), [path]);
  return src ? <img src={src} alt="Browser screenshot" className="mt-1 max-h-64 rounded border border-stone-200 dark:border-stone-700" /> : null;
}

const triggerLabel = (t: string) => (t === "chat" ? "chat" : t.startsWith("schedule:") ? `schedule #${t.slice(9)}` : t);

/** Every run with its model calls and actions in time order, plus undo. The journal is append-only. */
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
    rpc<number>("journal.verify").then((n) => setChain(`Journal intact (${n} entries)`), (e) => setChain(String(e)));
  };
  useEffect(() => {
    load();
    const un = onEvent(({ method, params }) => {
      if (params.agent_id === agent.id && (method === "chat.tool" || method === "chat.done")) load();
    });
    return () => void un.then((f) => f());
  }, [agent.id]);

  const act = async (method: string, params: object, label: string) => {
    setError("");
    try {
      await rpc(method, params);
    } catch (e) {
      const msg = String(e);
      if (msg.includes("force") && confirm(`${msg}\n\n${label} anyway and overwrite the newer version?`)) {
        await rpc(method, { ...params, force: true }).catch((e) => setError(String(e)));
      } else setError(msg);
    }
    load();
  };

  return (
    <div className="h-full space-y-4 overflow-y-auto p-4">
      <p className={`text-sm ${chain.startsWith("Journal intact") ? "text-green-700" : "text-red-600"}`}>{chain}</p>
      {error && <p role="alert" className="text-sm text-red-600">{error}</p>}
      {runs.length === 0 && <p className="text-sm text-stone-500">No runs yet.</p>}
      {runs.map((run) => {
        const items: Item[] = [
          ...calls.filter((c) => c.run_id === run.id).map((call) => ({ at: sqlTime(call.created_at).getTime(), call })),
          ...actions.filter((a) => a.run_id === run.id).map((action) => ({ at: action.created_at, action })),
        ].sort((a, b) => a.at - b.at);
        const tokens = calls.filter((c) => c.run_id === run.id).reduce((n, c) => n + (c.prompt_tokens ?? 0) + (c.completion_tokens ?? 0), 0);
        const undoable = actions.some((a) => a.run_id === run.id && a.undoable && !a.undone_at);
        return (
          <section key={run.id} className="rounded border border-stone-200 dark:border-stone-700">
            <header className="flex justify-between gap-2 border-b border-stone-200 px-3 py-1.5 text-sm dark:border-stone-700">
              <span>
                Run {run.id} · {triggerLabel(run.trigger)} · {sqlTime(run.started_at).toLocaleString()} ·{" "}
                <span className={run.status === "done" ? "" : "text-red-600"}>{run.status}</span>
                {tokens > 0 && <span className="text-stone-500"> · {tokens.toLocaleString()} tokens</span>}
              </span>
              {undoable && <button onClick={() => act("runs.undo", { run_id: run.id }, "Undo run")} className="text-amber-800">Undo run</button>}
            </header>
            <ul>
              {items.map(({ call, action: a }, i) =>
                call ? (
                  <li key={`c${i}`} className="px-3 py-1 text-xs text-stone-500">
                    model {call.model} · {call.prompt_tokens ?? "?"} → {call.completion_tokens ?? "?"} tokens · {(call.ms / 1000).toFixed(1)} s
                  </li>
                ) : a ? (
                  <li key={a.id} className="flex items-start justify-between gap-3 px-3 py-1.5 text-sm">
                    <div className="min-w-0">
                      <span className="font-mono">{a.tool}</span>{" "}
                      <span className="text-stone-500">{String(a.input.path ?? a.input.url ?? a.input.command ?? a.input.host ?? a.input.query ?? "")}</span>
                      {a.decision !== "auto" && <span className="ml-2 text-xs text-stone-500">({a.decision})</span>}
                      <pre className="truncate text-xs text-stone-500">{a.output}</pre>
                      {[...a.output.matchAll(/\[screenshot: ([^\]]+)\]/g)].map((m) => <Shot key={m[1]} agentId={agent.id} path={m[1]} />)}
                    </div>
                    {a.undone_at ? (
                      <span className="shrink-0 text-xs text-stone-500">undone</span>
                    ) : a.undoable ? (
                      <button onClick={() => act("actions.undo", { action_id: a.id }, "Undo")} className="shrink-0 text-amber-800">Undo</button>
                    ) : (
                      <span className="shrink-0 text-xs text-stone-400" title="Shell, web and MCP actions can't be undone">—</span>
                    )}
                  </li>
                ) : null,
              )}
            </ul>
          </section>
        );
      })}
    </div>
  );
}
