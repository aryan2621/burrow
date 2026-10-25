import { useEffect, useState } from "react";
import { Agent, McpServer, rpc, ui } from "./rpc";

/** MCP servers for this agent. stdio servers run inside its sandbox; HTTP servers must be on its allowed domains. */
export default function Tools({ agent }: { agent: Agent }) {
  const [list, setList] = useState<McpServer[]>([]);
  const [error, setError] = useState("");
  const [warnings, setWarnings] = useState<string[]>([]);
  const [f, setF] = useState({ name: "", transport: "stdio" as McpServer["transport"], command: "", url: "", secrets: "" });

  const load = () => rpc<McpServer[]>("mcp.list", { agent_id: agent.id }).then(setList, (e) => setError(String(e)));
  useEffect(() => void load(), [agent.id]);

  const add = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setWarnings([]);
    try {
      const r = await rpc<{ warnings: string[] }>("mcp.add", {
        agent_id: agent.id, name: f.name, transport: f.transport,
        // words split on spaces; "double" or 'single' quotes keep spaces in one argument
        command: [...f.command.matchAll(/"([^"]*)"|'([^']*)'|(\S+)/g)].map((m) => m[1] ?? m[2] ?? m[3]), url: f.url || null,
        secrets: f.secrets.split(",").map((s) => s.trim()).filter(Boolean),
      });
      setWarnings(r.warnings);
      setF({ ...f, name: "", command: "", url: "", secrets: "" });
      load();
    } catch (e) {
      setError(String(e));
    }
  };
  const remove = (name: string) => rpc("mcp.remove", { agent_id: agent.id, name }).then(load, (e) => setError(String(e)));

  return (
    <div className="h-full max-w-2xl space-y-4 overflow-y-auto p-4">
      <p className="text-sm text-stone-500">
        MCP tools appear to the agent as <span className="font-mono">mcp__server__tool</span>. They ask before running unless the
        server marks them read-only. Changes apply from the next message.
      </p>
      {!list.some((s) => s.name === "browser") && (
        <div className="flex items-center justify-between gap-2 rounded border border-stone-200 p-2 text-sm dark:border-stone-700">
          <span>Web browser: headless Chromium in this agent's sandbox, limited to its allowed domains. It asks you to handle logins.</span>
          <button className={ui.button} onClick={() => rpc("mcp.add_browser", { agent_id: agent.id }).then(load, (e) => setError(String(e)))}>Add browser</button>
        </div>
      )}
      {list.length === 0 && <p className="text-sm text-stone-500">No MCP servers yet.</p>}
      <ul className="space-y-2">
        {list.map((s) => (
          <li key={s.name} className="flex items-center justify-between gap-2 rounded border border-stone-200 p-2 text-sm dark:border-stone-700">
            <span className="min-w-0 truncate">
              <span className="font-semibold">{s.name}</span> <span className="text-stone-500">({s.transport})</span>{" "}
              <span className="font-mono text-xs">{s.transport === "stdio" ? s.command.join(" ") : s.url}</span>
            </span>
            <button className={ui.button} onClick={() => remove(s.name)}>Remove</button>
          </li>
        ))}
      </ul>
      <form onSubmit={add} className="space-y-2 border-t border-stone-200 pt-4 dark:border-stone-700">
        <h3 className="font-semibold">Add MCP server</h3>
        <div className="flex gap-2">
          <input aria-label="Name" className={ui.input} required placeholder="name, e.g. github" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} />
          <select aria-label="Transport" className={`${ui.input} w-48`} value={f.transport} onChange={(e) => setF({ ...f, transport: e.target.value as McpServer["transport"] })}>
            <option value="stdio">Command (in sandbox)</option>
            <option value="http">HTTP URL</option>
          </select>
        </div>
        {f.transport === "stdio" ? (
          <input aria-label="Command" className={ui.input} required placeholder="npx -y @modelcontextprotocol/server-everything"
            value={f.command} onChange={(e) => setF({ ...f, command: e.target.value })} />
        ) : (
          <>
            <input aria-label="URL" className={ui.input} required placeholder="https://mcp.example.com/mcp" value={f.url} onChange={(e) => setF({ ...f, url: e.target.value })} />
            <input aria-label="Secrets" className={ui.input} placeholder="secret names sent as headers (comma-separated, optional)"
              value={f.secrets} onChange={(e) => setF({ ...f, secrets: e.target.value })} />
          </>
        )}
        {f.transport === "stdio" && <p className="text-xs text-stone-500">npx/uvx install through the sandbox proxy, so allow the package registries in Permissions.</p>}
        {error && <p role="alert" className="text-sm text-red-600">{error}</p>}
        {warnings.length > 0 && (
          <div role="status" className="text-sm text-amber-700 dark:text-amber-400">
            Added, with Antidote warnings:
            <ul className="list-disc pl-5">{warnings.map((w) => <li key={w}>{w}</li>)}</ul>
          </div>
        )}
        <button className={ui.primary}>Add</button>
      </form>
    </div>
  );
}
