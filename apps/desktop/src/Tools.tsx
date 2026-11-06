import { useEffect, useState } from "react";
import { Agent, McpServer, Plugin, rpc } from "./rpc";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemTitle } from "@/components/ui/item";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Section } from "@/components/settings";
import { Disclosure, ErrorNote, ScanWarnings } from "@/components/panel";
import { useMcp } from "./useMcp";

/** MCP servers for this agent. stdio servers run inside its sandbox; HTTP servers must be on its allowed domains. */
export default function Tools({ agent }: { agent: Agent }) {
  const { list, load, error, setError, warnings, setWarnings } = useMcp(agent);
  const [plugins, setPlugins] = useState<Plugin[]>([]);
  const [keys, setKeys] = useState<Record<string, string>>({});
  useEffect(() => void rpc<Plugin[]>("plugins.catalog").then(setPlugins, (e) => setError(String(e))), []);
  const attach = async (pl: Plugin) => {
    setError("");
    try {
      const r = await rpc<{ warnings: string[] }>("plugins.attach", { agent_id: agent.id, id: pl.id, value: keys[pl.id] });
      setWarnings(r.warnings);
      setKeys({ ...keys, [pl.id]: "" });
      load();
    } catch (e) {
      setError(String(e));
    }
  };
  const [f, setF] = useState({ name: "", transport: "stdio" as McpServer["transport"], command: "", url: "", secrets: "" });

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

  const fail = (e: unknown) => setError(String(e));
  return (
    <>
      <Section>
        <ErrorNote msg={error} />
        <ScanWarnings warnings={warnings} />
        <ItemGroup className="gap-2 py-2">
          {!list.some((s) => s.name === "browser") && (
            <Item variant="outline" size="sm" className="bg-card">
              <ItemContent>
                <ItemTitle>Web browser</ItemTitle>
                <ItemDescription>Headless Chromium in this agent’s sandbox, limited to its allowed domains. It asks you to handle logins.</ItemDescription>
              </ItemContent>
              <ItemActions><Button size="sm" variant="outline" onClick={() => rpc("mcp.add_browser", { agent_id: agent.id }).then(load, fail)}>Add browser</Button></ItemActions>
            </Item>
          )}
          {plugins.filter((pl) => !list.some((s) => s.name === pl.id)).map((pl) => (
            <Item key={pl.id} variant="outline" size="sm" className="bg-card">
              <ItemContent className="min-w-0">
                <ItemTitle>{pl.name}</ItemTitle>
                <ItemDescription>{pl.description}</ItemDescription>
              </ItemContent>
              <ItemActions>
                {pl.auth === "key" && pl.secret && (
                  <Input aria-label={pl.secret.hint} type="password" className="w-48" placeholder={pl.secret.hint}
                    value={keys[pl.id] ?? ""} onChange={(e) => setKeys({ ...keys, [pl.id]: e.target.value })} />
                )}
                <Button size="sm" variant="outline" onClick={() => attach(pl)}>{pl.auth === "oauth" ? "Sign in and add" : "Add"}</Button>
              </ItemActions>
            </Item>
          ))}
          <Item variant="outline" size="sm" className="bg-card">
            <ItemContent>
              <ItemTitle>Email (IMAP)</ItemTitle>
              <ItemDescription>Search, read, draft and send mail. Set up under Settings → Email, then grant it to this agent.</ItemDescription>
            </ItemContent>
          </Item>
        </ItemGroup>
      </Section>

      <Section title="Installed">
        {list.length === 0 && <p className="py-3 text-muted-foreground">No MCP servers yet.</p>}
        <ItemGroup className="gap-2 py-2">
          {list.map((s) => (
            <Item key={s.name} variant="outline" size="sm" className="bg-card">
              <ItemContent className="min-w-0">
                <ItemTitle>{s.name}<span className="text-xs font-normal text-muted-foreground">{s.transport}</span></ItemTitle>
                <ItemDescription className="truncate font-mono text-xs">{s.transport === "stdio" ? s.command.join(" ") : s.url}</ItemDescription>
              </ItemContent>
              <ItemActions><Button size="sm" variant="ghost" className="hover:text-destructive" onClick={() => remove(s.name)}>Remove</Button></ItemActions>
            </Item>
          ))}
        </ItemGroup>
      </Section>

      <Disclosure title="Add MCP server" summary="Your own command or HTTP server">
        <form onSubmit={add} className="space-y-2">
          <div className="flex gap-2">
            <Input aria-label="Name" required placeholder="Name, e.g. github" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} />
            <Select value={f.transport} onValueChange={(v) => setF({ ...f, transport: v as McpServer["transport"] })}>
              <SelectTrigger aria-label="Transport" className="w-52 shrink-0"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="stdio">Command (in sandbox)</SelectItem>
                <SelectItem value="http">HTTP URL</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {f.transport === "stdio" ? (
            <Input aria-label="Command" required placeholder="npx -y @modelcontextprotocol/server-everything"
              value={f.command} onChange={(e) => setF({ ...f, command: e.target.value })} />
          ) : (
            <>
              <Input aria-label="URL" required placeholder="https://mcp.example.com/mcp" value={f.url} onChange={(e) => setF({ ...f, url: e.target.value })} />
              <Input aria-label="Secrets" placeholder="Secret names sent as headers (comma-separated, optional)"
                value={f.secrets} onChange={(e) => setF({ ...f, secrets: e.target.value })} />
            </>
          )}
          {f.transport === "stdio" && <p className="text-xs text-muted-foreground">npx and uvx install through the sandbox proxy, so allow the package registries in Permissions.</p>}
          <Button size="sm">Add</Button>
        </form>
      </Disclosure>
    </>
  );
}
