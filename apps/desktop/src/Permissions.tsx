import { useEffect, useRef, useState } from "react";
import { BotIcon, BoxIcon, CpuIcon, FolderIcon, GaugeIcon, PlusIcon, ShieldCheckIcon, Trash2Icon } from "lucide-react";
import FoldersField from "./FoldersField";
import { Agent, DEFAULT_RULES, Limits, Rule, TOOLS, rpc } from "./rpc";
import { Row, Section, SettingsView, SliderRow, Status, type SettingsPage } from "@/components/settings";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { Switch } from "@/components/ui/switch";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

export type Perms = { allowed_domains: string; approval_rules: Record<string, Rule> };

const LABEL: Record<string, string> = {
  list_dir: "List folders", read_file: "Read files", write_file: "Write files (undoable)",
  delete_file: "Delete files (undoable)", shell: "Run shell in sandbox", http_fetch: "Open sites not on the allowed list",
  notes_add: "Save notes (undoable)", notes_search: "Search notes", web_search: "Web search (if configured)",
  email_search: "List emails", email_read: "Read emails", email_move: "Move emails (undoable)", email_draft: "Save drafts (undoable)",
  email_send: "Send email (60 s to cancel)",
  delegate: "Hand tasks to other agents (makes this a team lead)",
};
const RULE_LABEL: Record<Rule, string> = { allow: "Allow", ask: "Ask me", deny: "Deny" };
const RULE_ON: Record<Rule, string> = {
  allow: "data-[state=on]:bg-accent data-[state=on]:font-medium",
  ask: "data-[state=on]:bg-accent data-[state=on]:font-medium",
  deny: "data-[state=on]:bg-accent data-[state=on]:font-medium data-[state=on]:text-destructive",
};
const GROUPS: [string, (typeof TOOLS)[number][]][] = [
  ["Files", ["list_dir", "read_file", "write_file", "delete_file"]],
  ["Shell and web", ["shell", "http_fetch", "web_search"]],
  ["Notes", ["notes_add", "notes_search"]],
  ["Team", ["delegate"]],
  ["Email", ["email_search", "email_read", "email_move", "email_draft", "email_send"]],
];

/** Allowed domains plus allow/ask/deny per tool. Rules equal to the default are not stored. */
export default function Permissions({ value, onChange }: { value: Perms; onChange: (p: Perms) => void }) {
  const addRegistries = async () => {
    const { package_registries } = await rpc<{ package_registries: string[] }>("domains.presets");
    const merged = [...new Set([...splitDomains(value.allowed_domains), ...package_registries])];
    onChange({ ...value, allowed_domains: merged.join(", ") });
  };
  return (
    <>
      <Section title="Network">
        <Row label="Allowed domains" detail="Comma-separated; subdomains included. Anything else asks first." stack>
          <Input aria-label="Allowed domains" placeholder="wikipedia.org, api.github.com" value={value.allowed_domains}
            onChange={(e) => onChange({ ...value, allowed_domains: e.target.value })} />
          <Button type="button" variant="outline" onClick={addRegistries}><PlusIcon />Package registries (npm, PyPI…)</Button>
        </Row>
      </Section>
      <Section title="Actions" description="What runs on its own, what asks you first, and what's never allowed.">
        <Tabs defaultValue={GROUPS[0][0]} className="mt-3">
          <TabsList className="w-full">
            {GROUPS.map(([g]) => <TabsTrigger key={g} value={g}>{g}</TabsTrigger>)}
          </TabsList>
          {GROUPS.map(([g, tools]) => (
            <TabsContent key={g} value={g} className="[&>*+*]:border-t">
              {tools.map((t) => (
                <Row key={t} label={LABEL[t]}>
                  <ToggleGroup type="single" variant="outline" size="sm" aria-label={`${LABEL[t]} rule`}
                    value={value.approval_rules[t] ?? DEFAULT_RULES[t]} onValueChange={(v) => {
                      if (!v) return; // single toggle groups emit "" when the active item is clicked again
                      const rules = { ...value.approval_rules, [t]: v as Rule };
                      if (rules[t] === DEFAULT_RULES[t]) delete rules[t];
                      onChange({ ...value, approval_rules: rules });
                    }}>
                    {(Object.keys(RULE_LABEL) as Rule[]).map((r) => (
                      <ToggleGroupItem key={r} value={r} className={`px-3 text-xs ${RULE_ON[r]}`}>{RULE_LABEL[r]}</ToggleGroupItem>
                    ))}
                  </ToggleGroup>
                </Row>
              ))}
            </TabsContent>
          ))}
        </Tabs>
      </Section>
    </>
  );
}

export const splitDomains = (s: string) => s.split(",").map((d) => d.trim()).filter(Boolean);

export type Page = "general" | "model" | "limits" | "sandbox" | "folders" | "permissions" | "danger";
const PAGES: SettingsPage<Page>[] = [
  { id: "general", label: "General", group: "Agent", icon: <BotIcon />, keywords: "name role prompt identity" },
  { id: "model", label: "Model", group: "Agent", icon: <CpuIcon />, keywords: "context budget review cloud local" },
  { id: "limits", label: "Run limits", group: "Limits", icon: <GaugeIcon />, keywords: "steps minutes tokens output monthly cost usd cap" },
  { id: "sandbox", label: "Sandbox", group: "Limits", icon: <BoxIcon />, keywords: "cpu memory ram shell browser" },
  { id: "folders", label: "Folders", group: "Access", icon: <FolderIcon />, keywords: "disk path directory desktop documents access read write" },
  { id: "permissions", label: "Permissions", group: "Access", icon: <ShieldCheckIcon />, keywords: "domains allow ask deny approval rules" },
  { id: "danger", label: "Delete agent", group: "Danger", icon: <Trash2Icon />, keywords: "remove delete" },
];

/** An agent's configuration, in the same settings window as the app's. */
export function AgentSettings({ agent, open, startPage = "general", onOpenChange, onSaved, onDelete }: {
  agent: Agent; open: boolean; startPage?: Page; onOpenChange: (o: boolean) => void; onSaved: () => void; onDelete: () => void;
}) {
  const [page, setPage] = useState<Page>(startPage);
  useEffect(() => { if (open) setPage(startPage); }, [open, startPage]);
  const [folders, setFolders] = useState(agent.folders);
  const [perms, setPerms] = useState<Perms>({ allowed_domains: agent.allowed_domains.join(", "), approval_rules: agent.approval_rules });
  const [name, setName] = useState(agent.name);
  const [role, setRole] = useState(agent.role_prompt);
  const [budget, setBudget] = useState(agent.context_budget);
  const [model, setModel] = useState(agent.model_ref);
  const [models, setModels] = useState<string[]>([agent.model_ref]);
  const [limits, setLimits] = useState<Limits>(agent.limits);
  const [msg, setMsg] = useState("");
  useEffect(() => void rpc<string[]>("models.list").then((m) => setModels(m.includes(agent.model_ref) ? m : [agent.model_ref, ...m])), []);
  const wasOpen = useRef(false);
  useEffect(() => {
    if (open && !wasOpen.current) {
      setFolders(agent.folders);
      setPerms({ allowed_domains: agent.allowed_domains.join(", "), approval_rules: agent.approval_rules });
      setName(agent.name);
      setRole(agent.role_prompt);
      setBudget(agent.context_budget);
      setModel(agent.model_ref);
      setLimits(agent.limits);
      setMsg("");
    }
    wasOpen.current = open;
  }, [open, agent]);
  const flushFolders = useRef<(() => string[]) | null>(null);
  const save = () => {
    const saved = flushFolders.current?.() ?? folders;
    return rpc("agents.update", { agent_id: agent.id, name, role_prompt: role, context_budget: budget, model_ref: model, limits, folders: saved,
      allowed_domains: splitDomains(perms.allowed_domains), approval_rules: perms.approval_rules })
      .then(() => {
        setFolders(saved);
        setMsg("Saved");
        onSaved();
      }, (e) => setMsg(String(e)));
  };
  const num = (k: keyof Limits, label: string, detail: string, max: number, scale = 1) => (
    <SliderRow label={label} detail={detail} min={1} max={max} value={(limits[k] as number) / scale}
      onChange={(v) => setLimits({ ...limits, [k]: v * scale })} />
  );
  const saveBar = (
    <div className="flex items-center gap-3 border-t pt-4">
      <Button onClick={save}>Save changes</Button>
      <Status msg={msg} />
    </div>
  );
  return (
    <SettingsView title={`${agent.name} settings`} open={open} onOpenChange={onOpenChange} pages={PAGES} page={page} onPage={setPage}>
      {page === "general" && <><Section>
            <Row label="Name">
              <Input aria-label="Name" className="w-60" value={name} onChange={(e) => setName(e.target.value)} />
            </Row>
            <Row label="Role" detail="It sees this before every message." stack>
              <Textarea aria-label="Role" rows={5} value={role} onChange={(e) => setRole(e.target.value)} />
            </Row>
          </Section>{saveBar}</>}
      {page === "model" && <><Section>
            <Row label="Model" detail={/^(anthropic|openai):/.test(model) ? "Cloud model: messages and tool results leave this machine." : "Runs on this machine."}>
              <Select value={model} onValueChange={setModel}>
                <SelectTrigger aria-label="Model" className="w-60 font-mono text-xs"><SelectValue /></SelectTrigger>
                <SelectContent>{models.map((m) => <SelectItem key={m} value={m} className="font-mono text-xs">{m}</SelectItem>)}</SelectContent>
              </Select>
            </Row>
            <SliderRow label="Context budget" detail="Tokens of history sent to the model before older messages are summarized."
              min={1000} max={1000000} step={1000} value={budget} onChange={setBudget} format={(v) => `${Math.round(v / 1000)}k`} />
            <Row label="Review risky actions" detail="A second model call checks auto-allowed changes; an objection asks you first. Cloud models only.">
              <Switch aria-label="Review risky actions" checked={limits.review} onCheckedChange={(review) => setLimits({ ...limits, review })} />
            </Row>
          </Section>{saveBar}</>}
      {page === "limits" && <><Section description="A run stops when it hits any of these.">
            {num("max_steps", "Steps", "Model turns per run.", 100)}
            {num("max_run_secs", "Minutes", "Wall-clock time per run.", 120, 60)}
            <SliderRow label="Output tokens" detail="Tokens the model may write per run."
              min={256} max={32768} step={256} value={limits.max_run_tokens} onChange={(max_run_tokens) => setLimits({ ...limits, max_run_tokens })}
              format={(v) => v.toLocaleString()} />
            <SliderRow label="Monthly cloud budget" detail="USD per month; 0 means no cap."
              min={0} max={200} step={0.5} value={limits.monthly_usd} onChange={(monthly_usd) => setLimits({ ...limits, monthly_usd })}
              format={(v) => (v === 0 ? "No cap" : `$${v}`)} />
          </Section>{saveBar}</>}
      {page === "sandbox" && <><Section description="Shell and browser resources. Applies on the next sandbox start.">
            {num("cpus", "Sandbox CPUs", "Shell and browser CPU cores. Applies on the next sandbox start.", 16)}
            {num("memory_mb", "Sandbox memory (GB)", "RAM for shell and browser.", 32, 1024)}
          </Section>{saveBar}</>}
      {page === "folders" && (
        <>
          <Section title="Disk access" description="The only places on your Mac this agent can list, read, or change files. Shell commands see them read-only; writes go through journaled file tools. Empty means scratch only.">
            <FoldersField folders={folders} onChange={setFolders} flush={flushFolders} />
          </Section>
          {saveBar}
        </>
      )}
      {page === "permissions" && <><Permissions value={perms} onChange={setPerms} />{saveBar}</>}
      {page === "danger" && <Section>
            <Row label={`Delete ${agent.name}`} detail="Removes the agent and its chat history. The action log is kept.">
              <Button variant="destructive" onClick={onDelete}><Trash2Icon />Delete</Button>
            </Row>
          </Section>}
    </SettingsView>
  );
}
