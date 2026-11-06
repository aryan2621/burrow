import { useState } from "react";
import { Agent, rpc } from "./rpc";
import { Card } from "./AgentForm";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { ErrorNote } from "@/components/panel";
import { Avatar } from "./Avatars";

type Member = { agent: string | null; name: string; role: string; why: string };

/** Describe a problem, get a proposed team, approve it; the lead agent then splits the work with `delegate`. */
export default function Team({ agents, onStarted }: { agents: Agent[]; onStarted: (leadId: number) => void }) {
  const [lead, setLead] = useState(agents[0]?.id ?? 0);
  const [problem, setProblem] = useState("");
  const [members, setMembers] = useState<(Member & { on: boolean })[] | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const leadAgent = agents.find((a) => a.id === lead);

  const run = async (f: () => Promise<void>) => {
    setError("");
    setBusy(true);
    try { await f(); } catch (e) { setError(String(e)); }
    setBusy(false);
  };
  const propose = () => run(async () => {
    const r = await rpc<{ members: Member[] }>("team.propose", { lead_id: lead, model_ref: leadAgent!.model_ref, problem });
    setMembers(r.members.map((m) => ({ ...m, on: true })));
  });
  const start = () => run(async () => {
    for (const m of members!.filter((m) => m.on && !m.agent && !agents.some((a) => a.name.toLowerCase() === m.name.toLowerCase()))) {
      await rpc("agents.create", { name: m.name, role_prompt: m.role || "You are a specialist. Do the task you are given and reply with the result.",
        model_ref: leadAgent!.model_ref, folders: [], allowed_domains: [] });
    }
    await rpc("team.start", { lead_id: lead, problem });
    onStarted(lead);
  });

  if (agents.length === 0) return (
    <div className="flex h-full flex-col items-center justify-center gap-5 p-6 text-center">
      <div className="flex items-end gap-2" aria-hidden>
        {(["pebble", "cloud", "pebble"] as const).map((k, i) => (
          <div key={i} className="bob" style={{ animationDelay: `${i * 0.5}s` }}><Avatar kind={k} className={i === 1 ? "size-16" : "size-12"} /></div>
        ))}
      </div>
      <p className="text-sm text-muted-foreground">Create an agent first; it will lead the team.</p>
    </div>
  );
  return (
    <div className="h-full overflow-y-auto">
      <div className="mx-auto max-w-3xl px-8 py-8">
        <header>
          <h1 className="title text-2xl">New team</h1>
          <p className="mt-1 text-muted-foreground">A lead agent splits the problem between specialists, each in its own sandbox, and combines their answers.</p>
        </header>
        <ErrorNote msg={error} className="mt-4" />

        <div className="mt-6 space-y-6">
          <Card title="Lead" description="Needs a cloud model, or a built-in one of 9B or more.">
            <Select value={String(lead)} onValueChange={(v) => { setLead(Number(v)); setMembers(null); }}>
              <SelectTrigger aria-label="Lead agent" className="w-full"><SelectValue /></SelectTrigger>
              <SelectContent>
                {agents.map((a) => <SelectItem key={a.id} value={String(a.id)}>{a.name} <span className="font-mono text-xs text-muted-foreground">{a.model_ref}</span></SelectItem>)}
              </SelectContent>
            </Select>
          </Card>

          <Card title="Problem" description="What should the team solve?">
            <Textarea aria-label="Problem" rows={5} placeholder="e.g. Research competitors and draft a one-page comparison" value={problem}
              onChange={(e) => { setProblem(e.target.value); setMembers(null); }} />
            {!members && (
              <Button disabled={busy || !problem.trim()} onClick={propose}>{busy ? "Proposing…" : "Propose a team"}</Button>
            )}
          </Card>

          {members && (
            <Card title="Proposed team" description="Untick anyone you don't want. New agents are created with the lead's model and no folder access.">
              <ul className="divide-y">
                {members.map((m, i) => (
                  <li key={i}>
                    <Label className="flex cursor-pointer items-start gap-3 p-3 font-normal">
                      <Checkbox checked={m.on} className="mt-0.5" onCheckedChange={(on) => setMembers(members.map((x, j) => (j === i ? { ...x, on: on === true } : x)))} />
                      <span className="text-sm">
                        <span className="font-medium">{m.name}</span>
                        <span className="ml-2 text-xs text-muted-foreground">{m.agent ? "existing agent" : "new agent"}</span>
                        <span className="mt-0.5 block text-muted-foreground">{m.why}</span>
                      </span>
                    </Label>
                  </li>
                ))}
              </ul>
              <div className="flex gap-2">
                <Button disabled={busy || !members.some((m) => m.on)} onClick={start}>{busy ? "Starting…" : "Approve and start"}</Button>
                <Button variant="outline" disabled={busy} onClick={() => setMembers(null)}>Propose again</Button>
              </div>
            </Card>
          )}
        </div>
      </div>
    </div>
  );
}
