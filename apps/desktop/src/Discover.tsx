import { useState } from "react";
import { Agent, rpc } from "./rpc";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemTitle } from "@/components/ui/item";
import { Section } from "@/components/settings";
import { ErrorNote, ScanWarnings } from "@/components/panel";
import { useMcp } from "./useMcp";

type Found = { title: string; name: string; description: string; transport: "stdio" | "http"; command?: string[]; url?: string };

/** Shown before any search: well-known servers that need no key. Run through npx/uvx in the sandbox like any registry result. */
const SUGGESTED: Found[] = [
  { title: "Memory", name: "memory", description: "A knowledge graph the agent can write to and read back across chats.", transport: "stdio", command: ["npx", "-y", "@modelcontextprotocol/server-memory"] },
  { title: "Sequential Thinking", name: "thinking", description: "Step-by-step reasoning for multi-part problems.", transport: "stdio", command: ["npx", "-y", "@modelcontextprotocol/server-sequential-thinking"] },
  { title: "Context7", name: "context7", description: "Up-to-date library documentation and code examples.", transport: "stdio", command: ["npx", "-y", "@upstash/context7-mcp"] },
  { title: "Time", name: "time", description: "Current time and time zone conversion.", transport: "stdio", command: ["uvx", "mcp-server-time"] },
];

/** Find MCP servers in the official registry (community-made, not reviewed by Burrow) and add them to this agent. */
export default function Discover({ agent }: { agent: Agent }) {
  const { list, load, error, setError, warnings, setWarnings } = useMcp(agent);
  const [q, setQ] = useState("");
  const [found, setFound] = useState<Found[] | null>(null);
  const [busy, setBusy] = useState(false);
  const search = (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    setBusy(true);
    rpc<Found[]>("plugins.search", { query: q }).then(setFound, (e) => setError(String(e))).finally(() => setBusy(false));
  };
  const add = (r: Found) => {
    setWarnings([]);
    return rpc<{ warnings: string[] }>("plugins.add_community", { agent_id: agent.id, name: r.name, transport: r.transport, command: r.command ?? [], url: r.url ?? null, secrets: [] })
      .then((x) => { setWarnings(x.warnings); load(); }, (e) => setError(String(e)));
  };
  const shown = (found ?? SUGGESTED).filter((r) => !list.some((x) => x.name === r.name));
  return (
    <>
      <Section description="Community servers, not reviewed by Burrow. Command servers run in the sandbox and are scanned for risky behaviour.">
        <form onSubmit={search} className="flex gap-2 py-2">
          <Input aria-label="Search" placeholder="e.g. postgres, jira, calendar" value={q} onChange={(e) => setQ(e.target.value)} />
          <Button size="sm" className="h-9" disabled={!q.trim() || busy}>{busy ? "Searching…" : "Search"}</Button>
        </form>
        <ErrorNote msg={error} />
        <ScanWarnings warnings={warnings} />
      </Section>
      <Section title={found ? "Results" : "Suggested, no key needed"}>
        {shown.length === 0 && <p className="py-3 text-muted-foreground">{found ? "Nothing found." : "Everything suggested is already added."}</p>}
        <ItemGroup className="gap-2 py-2">
          {shown.map((r) => (
            <Item key={r.title} variant="outline" size="sm" className="bg-card">
              <ItemContent className="min-w-0">
                <ItemTitle>{r.title}<span className="text-xs font-normal text-muted-foreground">{r.transport}</span></ItemTitle>
                <ItemDescription className="line-clamp-2">{r.description}</ItemDescription>
              </ItemContent>
              <ItemActions><Button type="button" size="sm" variant="outline" onClick={() => add(r)}>Add</Button></ItemActions>
            </Item>
          ))}
        </ItemGroup>
      </Section>
    </>
  );
}
