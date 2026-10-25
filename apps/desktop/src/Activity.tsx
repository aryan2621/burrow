import { useEffect, useState } from "react";
import { Action, Agent, onEvent, rpc } from "./rpc";
import Permissions, { Perms, splitDomains } from "./Permissions";

/** The agent's journal grouped by run, with undo, plus its permission settings. */
export default function Activity({ agent, onSaved }: { agent: Agent; onSaved: () => void }) {
  const [actions, setActions] = useState<Action[]>([]);
  const [chain, setChain] = useState("");
  const [error, setError] = useState("");
  const [perms, setPerms] = useState<Perms>({
    allowed_domains: agent.allowed_domains.join(", "), approval_rules: agent.approval_rules,
  });

  const load = () => {
    rpc<Action[]>("actions.list", { agent_id: agent.id }).then(setActions, (e) => setError(String(e)));
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
  const save = () =>
    rpc("agents.update", {
      agent_id: agent.id, allowed_domains: splitDomains(perms.allowed_domains), approval_rules: perms.approval_rules,
    }).then(onSaved, (e) => setError(String(e)));

  const runs = [...new Set(actions.map((a) => a.run_id))].reverse();
  return (
    <div className="h-full space-y-4 overflow-y-auto p-4">
      <p className={`text-sm ${chain.startsWith("Journal intact") ? "text-green-700" : "text-red-600"}`}>{chain}</p>
      {error && <p role="alert" className="text-sm text-red-600">{error}</p>}
      {runs.length === 0 && <p className="text-sm text-stone-500">No actions yet.</p>}
      {runs.map((run) => {
        const list = actions.filter((a) => a.run_id === run);
        const undoable = list.some((a) => a.undoable && !a.undone_at);
        return (
          <section key={run} className="rounded border border-stone-200 dark:border-stone-700">
            <header className="flex justify-between border-b border-stone-200 px-3 py-1.5 text-sm dark:border-stone-700">
              <span>Run {run} · {new Date(list[0].created_at).toLocaleString()}</span>
              {undoable && (
                <button onClick={() => act("runs.undo", { run_id: run }, "Undo run")} className="text-amber-800">Undo run</button>
              )}
            </header>
            <ul>
              {list.map((a) => (
                <li key={a.id} className="flex items-start justify-between gap-3 px-3 py-1.5 text-sm">
                  <div className="min-w-0">
                    <span className="font-mono">{a.tool}</span>{" "}
                    <span className="text-stone-500">{String(a.input.path ?? a.input.url ?? a.input.command ?? "")}</span>
                    {a.decision !== "auto" && <span className="ml-2 text-xs text-stone-500">({a.decision})</span>}
                    <pre className="truncate text-xs text-stone-500">{a.output}</pre>
                  </div>
                  {a.undone_at ? (
                    <span className="shrink-0 text-xs text-stone-500">undone</span>
                  ) : a.undoable ? (
                    <button onClick={() => act("actions.undo", { action_id: a.id }, "Undo")} className="shrink-0 text-amber-800">Undo</button>
                  ) : (
                    <span className="shrink-0 text-xs text-stone-400" title="Shell and web actions can't be undone">—</span>
                  )}
                </li>
              ))}
            </ul>
          </section>
        );
      })}
      <section className="max-w-lg space-y-2 border-t border-stone-200 pt-4 dark:border-stone-700">
        <h3 className="font-semibold">Permissions</h3>
        <Permissions value={perms} onChange={setPerms} />
        <button onClick={save} className="rounded bg-amber-800 px-4 py-1.5 text-white">Save</button>
      </section>
    </div>
  );
}
