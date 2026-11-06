import { useEffect, useState } from "react";
import { ArchiveIcon, CheckIcon, DownloadIcon, XIcon, CpuIcon, KeyRoundIcon, MailIcon, ReceiptIcon, SearchIcon, Settings2Icon } from "lucide-react";
import { Agent, CatalogModel, Connection, Cost, Hardware, Providers, Secret, Status, confirmAction, pickFile, pickSavePath, rpc } from "./rpc";
import useDownloads from "./useDownloads";
import { AgentPicker, Row, Section, SettingsView, SliderRow, Status as Msg, type SettingsPage } from "@/components/settings";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Progress } from "@/components/ui/progress";
import { Switch } from "@/components/ui/switch";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";

const gb = (b: number) => `${(b / 2 ** 30).toFixed(1)} GB`;

export function ModelList({ hardware, onChange }: { hardware: Hardware | null; onChange?: () => void }) {
  const [models, setModels] = useState<CatalogModel[]>([]);
  const [error, setError] = useState("");
  const load = () => rpc<CatalogModel[]>("models.catalog").then((ms) => { setModels(ms); settle(ms.filter((m) => m.downloading).map((m) => m.id)); }, (e) => setError(String(e)));
  const { progress, errors, settle } = useDownloads(() => { load(); onChange?.(); });
  useEffect(() => void load(), []);
  const call = (method: string, id: string) => rpc(method, { id }).then(load, (e) => setError(String(e)));
  return (
    <div className="[&>*+*]:border-t">
      {/* only what this machine can run and has room for (same half-GB RAM slack as the daemon's recommendation; 1 GB spare disk), plus anything already downloaded */}
      {hardware && models.filter((m) => m.installed || (hardware.ram_gb + 0.5 >= m.min_ram_gb && (hardware.free_disk_gb ?? Infinity) >= m.size / 2 ** 30 + 1)).map((m) => (
        <Row key={m.id}
          label={<span className="flex items-center gap-2 font-medium">{m.name}
            {hardware?.recommended === m.id && <Badge variant="secondary">Recommended</Badge>}</span>}
          detail={errors[m.id] ? <span role="alert" className="text-destructive">{errors[m.id]}</span> : `${gb(m.size)} · needs ${m.min_ram_gb} GB RAM${hardware?.vram_gb ? (m.size / 2 ** 30 * 1.2 <= hardware.vram_gb ? " · fits your GPU" : " · too big for your GPU, partly on CPU (slower)") : ""}`}>
          {m.installed ? (
            <Badge variant="outline" className="gap-1 text-success"><CheckIcon />Installed</Badge>
          ) : m.downloading || progress[m.id] !== undefined ? (
            <div className="flex w-56 items-center gap-3">
              <div className="min-w-0 flex-1">
                <div className="mb-1.5 flex justify-between text-xs text-muted-foreground">
                  <span>Downloading</span><span className="tabular-nums">{Math.round(progress[m.id] ?? 0)}%</span>
                </div>
                <Progress value={progress[m.id] ?? 0} aria-label={`${m.name} download`} className="h-1.5" />
              </div>
              <Button variant="ghost" size="icon" className="size-8" aria-label={`Cancel ${m.name} download`} title="Cancel download"
                onClick={() => call("models.cancel", m.id)}><XIcon /></Button>
            </div>
          ) : (
            <Button variant="outline" size="sm" onClick={() => call("models.download", m.id)}><DownloadIcon />Download</Button>
          )}
        </Row>
      ))}
      {error && <p role="alert" className="py-2 text-sm text-destructive">{error}</p>}
    </div>
  );
}

/** A secret-ish value with Save and Clear, as one row. */
function KeyRow({ label, detail, placeholder, type = "password", onSave, onClear, clearLabel = "Clear" }: {
  label: string; detail?: string; placeholder: string; type?: string;
  onSave: (v: string) => Promise<unknown>; onClear: () => Promise<unknown>; clearLabel?: string;
}) {
  const [v, setV] = useState("");
  return (
    <Row label={label} detail={detail} stack>
      <Input aria-label={label} type={type} placeholder={placeholder} value={v} onChange={(e) => setV(e.target.value)} />
      <Button variant="outline" disabled={!v} onClick={() => onSave(v).then(() => setV(""))}>Save</Button>
      <Button variant="ghost" onClick={onClear}>{clearLabel}</Button>
    </Row>
  );
}

function Search({ status, reload }: { status: Status; reload: () => void }) {
  const [msg, setMsg] = useState("");
  const set = (p: object) => rpc("search.configure", p).then(() => { setMsg("Saved"); reload(); }, (e) => setMsg(String(e)));
  return (
    <Section description={status.search === "searxng" ? "Using SearXNG (local)." : status.search === "brave" ? "Using the Brave Search API."
      : "Off: agents don't get a web_search tool until you set one up."}>
      <KeyRow label="SearXNG URL" detail="A SearXNG instance you run yourself." type="url" placeholder="http://localhost:8888"
        onSave={(v) => set({ searxng_url: v })} onClear={() => set({ searxng_url: null })} />
      <KeyRow label="Brave Search API key" detail="Stored in the OS keychain." placeholder="API key"
        onSave={(v) => set({ brave_key: v })} onClear={() => set({ brave_key: null })} />
      <Msg msg={msg} />
    </Section>
  );
}

export function Cloud() {
  const [p, setP] = useState<Providers | null>(null);
  const [msg, setMsg] = useState("");
  const load = () => rpc<Providers>("providers.list").then(setP, (e) => setMsg(String(e)));
  useEffect(() => void load(), []);
  const set = (body: object) => rpc("providers.set", body).then(() => { setMsg("Saved"); load(); }, (e) => setMsg(String(e)));
  if (!p) return null;
  return (
    <Section title="Cloud providers" description="Off until you add a key. Only agents you switch to a cloud model use it; their messages and tool results then go to that provider.">
      <Tabs defaultValue="anthropic" className="mt-3">
        <TabsList className="w-full">
          {(["anthropic", "openai", "gemini", "custom"] as const).map((k) => (
            <TabsTrigger key={k} value={k} className="gap-2">
              {{ anthropic: "Anthropic", openai: "OpenAI", gemini: "Gemini", custom: "Other (OpenAI-compatible)" }[k]}
              {p[k].configured && <span role="img" aria-label="key saved" className="size-1.5 rounded-full bg-success" />}
            </TabsTrigger>
          ))}
        </TabsList>
        <TabsContent value="anthropic">
          <KeyRow label="API key" detail={p.anthropic.configured ? "Key saved in the keychain." : "Stored in the OS keychain."} placeholder="sk-ant-…"
            onSave={(v) => set({ provider: "anthropic", api_key: v })} onClear={() => set({ provider: "anthropic", api_key: null })} />
        </TabsContent>
        <TabsContent value="openai" className="[&>*+*]:border-t">
          <KeyRow label="API key" detail={p.openai.configured ? "Key saved in the keychain." : "Stored in the OS keychain."} placeholder="sk-…"
            onSave={(v) => set({ provider: "openai", api_key: v })} onClear={() => set({ provider: "openai", api_key: null })} />
          <KeyRow label="Base URL" detail={`Now ${p.openai.base_url ?? "https://api.openai.com/v1"}`} type="url" placeholder="https://…/v1" clearLabel="Reset"
            onSave={(v) => set({ provider: "openai", base_url: v })} onClear={() => set({ provider: "openai", base_url: null })} />
        </TabsContent>
        <TabsContent value="gemini">
          <KeyRow label="API key" detail={p.gemini.configured ? "Key saved in the keychain." : "From Google AI Studio. Stored in the OS keychain."} placeholder="AIza…"
            onSave={(v) => set({ provider: "gemini", api_key: v })} onClear={() => set({ provider: "gemini", api_key: null })} />
        </TabsContent>
        <TabsContent value="custom" className="[&>*+*]:border-t">
          <KeyRow label="Base URL" detail={p.custom.base_url ? `Now ${p.custom.base_url}` : "OpenRouter, Groq, Together, LM Studio, vLLM… any OpenAI-compatible API."} type="url" placeholder="https://…/v1" clearLabel="Reset"
            onSave={(v) => set({ provider: "custom", base_url: v })} onClear={() => set({ provider: "custom", base_url: null })} />
          <KeyRow label="API key" detail="Needed even for local servers; enter any text if yours has none. Both are needed before models show up." placeholder="key"
            onSave={(v) => set({ provider: "custom", api_key: v })} onClear={() => set({ provider: "custom", api_key: null })} />
        </TabsContent>
      </Tabs>
      <Msg msg={msg} />
    </Section>
  );
}

function Costs() {
  const [costs, setCosts] = useState<Cost[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [prices, setPrices] = useState("");
  const [msg, setMsg] = useState("");
  useEffect(() => {
    rpc<Cost[]>("costs.summary").then(setCosts);
    rpc<Agent[]>("agents.list").then(setAgents);
    rpc<string | null>("settings.get", { key: "prices" }).then((p) => setPrices(p ?? ""));
  }, []);
  const name = (id: number) => agents.find((a) => a.id === id)?.name ?? `agent ${id} (deleted)`;
  const save = () => rpc("settings.set", { key: "prices", value: prices || "{}" }).then(() => setMsg("Saved"), (e) => setMsg(String(e)));
  return (
    <>
      <Section title="Spend by agent">
        {costs.length === 0 ? <p className="py-3 text-muted-foreground">No cloud calls yet. Local models are free.</p> : (
          <table className="w-full text-sm">
            <thead><tr className="text-left text-muted-foreground"><th className="py-2 font-normal">Agent</th><th className="font-normal">This month</th><th className="font-normal">All time</th></tr></thead>
            <tbody>
              {costs.map((c) => (
                <tr key={c.agent_id} className="border-t">
                  <td className="py-2">{name(c.agent_id)}</td><td>${c.month_usd.toFixed(2)}</td>
                  <td>${c.total_usd.toFixed(2)}{c.unpriced_calls > 0 && <span className="text-muted-foreground"> + {c.unpriced_calls} unpriced</span>}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </Section>
      <Section title="Prices">
        <Row label="Other cloud models" detail="Claude prices are built in. For others, give USD per million input/output tokens." stack>
          <Input aria-label="Prices" className="font-mono" placeholder='{"openai:gpt-5": [1.25, 10]}' value={prices} onChange={(e) => setPrices(e.target.value)} />
          <Button variant="outline" onClick={save}>Save</Button>
        </Row>
        <Msg msg={msg} />
      </Section>
    </>
  );
}

function Data() {
  const [pass, setPass] = useState("");
  const [msg, setMsg] = useState("");
  const run = (f: () => Promise<unknown>, done: string) => { setMsg("Working…"); f().then(() => setMsg(done), (e) => setMsg(String(e))); };
  const exportTo = async () => {
    const path = await pickSavePath(`burrow-${new Date().toISOString().slice(0, 10)}.burrow`);
    if (path) run(() => rpc("backup.export", { path, passphrase: pass }), `Saved to ${path}`);
  };
  const importFrom = async () => {
    const path = await pickFile();
    if (path && await confirmAction("Replace all agents, history and undo snapshots with this backup?", "Replace"))
      run(() => rpc("backup.import", { path, passphrase: pass }), "Imported. Burrow is restarting.");
  };
  const wipe = async () => {
    if (await confirmAction("Wipe all data? This deletes every agent, chat, note, undo snapshot, scratch folder and stored key. It can't be undone.", "Wipe everything"))
      run(() => rpc("data.wipe", { confirm: "wipe" }), "Wiped. Burrow is restarting.");
  };
  return (
    <>
      <Section title="Backup" description="One file encrypted with your passphrase: agents, history, notes, schedules and undo snapshots. Keychain secrets and API keys aren't included.">
        <Row label="Passphrase" detail="At least 8 characters. You need it to import." stack>
          <Input aria-label="Backup passphrase" type="password" value={pass} onChange={(e) => setPass(e.target.value)} />
        </Row>
        <Row label="Export or import">
          <Button variant="outline" disabled={pass.length < 8} onClick={exportTo}>Export…</Button>
          <Button variant="outline" disabled={pass.length < 8} onClick={importFrom}>Import…</Button>
        </Row>
      </Section>
      <Section title="Danger zone">
        <Row label="Wipe all data" detail="Deletes every agent, its history and snapshots, and stored keys. Can't be undone.">
          <Button variant="destructive" onClick={wipe}>Wipe…</Button>
        </Row>
      </Section>
      <Msg msg={msg} />
    </>
  );
}

const FIELDS: Record<Connection["kind"], [string, string, string][]> = {
  email: [["imap_host", "IMAP host", "imap.fastmail.com"], ["imap_port", "IMAP port", "993"], ["smtp_host", "SMTP host", "smtp.fastmail.com"],
    ["smtp_port", "SMTP port", "465, or 587 for STARTTLS"], ["username", "Username", "you@example.com"], ["address", "From address", "if not the username"]],
};

function Connections() {
  const [list, setList] = useState<Connection[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [error, setError] = useState("");
  const blank = { name: "", kind: "email" as Connection["kind"], config: {} as Record<string, string>, secret: "", agent_ids: [] as number[] };
  const [f, setF] = useState(blank);
  const load = () => rpc<Connection[]>("connections.list").then(setList, (e) => setError(String(e)));
  useEffect(() => { load(); rpc<Agent[]>("agents.list").then(setAgents); }, []);
  const save = (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    const config: Record<string, unknown> = {};
    for (const [k, v] of Object.entries(f.config)) {
      if (!v) continue;
      config[k] = k.endsWith("_port") ? Number(v) : v;
    }
    rpc("connections.set", { ...f, config, secret: f.secret || null }).then(() => { setF(blank); load(); }, (e) => setError(String(e)));
  };
  const edit = (c: Connection) => setF({ name: c.name, kind: c.kind, secret: "", agent_ids: c.agent_ids,
    config: Object.fromEntries(Object.entries(c.config).map(([k, v]) => [k, String(v)])) });
  const name = (id: number) => agents.find((a) => a.id === id)?.name ?? `#${id}`;
  const editing = list.some((c) => c.name === f.name);
  return (
    <>
      <Section description="Use your email provider's app password; it goes to the OS keychain. Changes agents make can be undone from Activity, and emails wait 60 s before sending.">
        {list.length === 0 && <p className="py-3 text-muted-foreground">No connections yet.</p>}
        {list.map((c) => (
          <Row key={c.name} label={<span className="font-mono">{c.name}</span>} detail={`${c.kind} · ${c.agent_ids.map(name).join(", ") || "no agents"}`}>
            <Button variant="outline" size="sm" onClick={() => edit(c)}>Edit</Button>
            <Button variant="ghost" size="sm" className="text-destructive" onClick={async () => (await confirmAction(`Delete connection ${c.name}?`)) && rpc("connections.delete", { name: c.name }).then(load)}>Delete</Button>
          </Row>
        ))}
      </Section>
      <Section title={editing ? `Edit ${f.name}` : "Add an email account"}>
        <form onSubmit={save} className="grid grid-cols-2 gap-3 pt-3">
          <Input aria-label="Connection name" className="col-span-2" required placeholder="Name, e.g. work-mail" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} />
          {FIELDS[f.kind].map(([k, label, hint]) => (
            <Input key={k} aria-label={label} placeholder={`${label} (${hint})`} value={f.config[k] ?? ""}
              onChange={(e) => setF({ ...f, config: { ...f.config, [k]: e.target.value } })} />
          ))}
          <Input aria-label="App password" type="password" className="col-span-2" value={f.secret}
            placeholder={editing ? "App password (leave empty to keep)" : "App password"} onChange={(e) => setF({ ...f, secret: e.target.value })} />
          <div className="col-span-2 py-1"><AgentPicker agents={agents} value={f.agent_ids} onChange={(agent_ids) => setF({ ...f, agent_ids })} /></div>
          {error && <p role="alert" className="col-span-2 text-destructive">{error}</p>}
          <div className="col-span-2"><Button>{editing ? "Save changes" : "Add connection"}</Button></div>
        </form>
      </Section>
    </>
  );
}

function Secrets() {
  const [list, setList] = useState<Secret[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [error, setError] = useState("");
  const [f, setF] = useState({ name: "", value: "", hosts: "", header: "Authorization: Bearer {}", expires: "", agent_ids: [] as number[] });
  const load = () => rpc<Secret[]>("secrets.list").then(setList, (e) => setError(String(e)));
  useEffect(() => { load(); rpc<Agent[]>("agents.list").then(setAgents); }, []);
  const add = (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    rpc("secrets.set", { ...f, expires_in_hours: Number(f.expires) || undefined, hosts: f.hosts.split(",").map((h) => h.trim()).filter(Boolean) })
      .then(() => { setF({ ...f, name: "", value: "", hosts: "", expires: "" }); load(); }, (e) => setError(String(e)));
  };
  const name = (id: number) => agents.find((a) => a.id === id)?.name ?? `#${id}`;
  return (
    <>
      <Section description="Values go to the OS keychain. Agents never see them: Burrow adds them to requests for the listed hosts and scrubs them from output.">
        {list.length === 0 && <p className="py-3 text-muted-foreground">No secrets yet.</p>}
        {list.map((s) => (
          <Row key={s.name} label={<span className="font-mono">{s.name}</span>} detail={`${s.hosts.join(", ")} · ${s.agent_ids.map(name).join(", ") || "no agents"}${s.expires_at ? ` · expires ${s.expires_at} UTC` : ""}${s.suspended?.length ? ` · stopped for ${s.suspended.map(name).join(", ")}` : ""}`}>
            {s.suspended?.map((id) => (
              <Button key={id} variant="outline" size="sm" onClick={() => rpc("agents.restore_access", { agent_id: id }).then(load, (e) => setError(String(e)))}>Restore {name(id)}</Button>
            ))}
            <Button variant="ghost" size="sm" className="text-destructive" onClick={async () => (await confirmAction(`Delete secret ${s.name}?`)) && rpc("secrets.delete", { name: s.name }).then(load)}>Delete</Button>
          </Row>
        ))}
      </Section>
      <Section title="Add a secret">
        <form onSubmit={add} className="grid grid-cols-2 gap-3 pt-3">
          <Input aria-label="Secret name" required placeholder="Name" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} />
          <Input aria-label="Secret value" type="password" required placeholder="Value" value={f.value} onChange={(e) => setF({ ...f, value: e.target.value })} />
          <Input aria-label="Hosts" className="col-span-2" required placeholder="Hosts, e.g. api.github.com" value={f.hosts} onChange={(e) => setF({ ...f, hosts: e.target.value })} />
          <Input aria-label="Header" className="col-span-2 font-mono" value={f.header} onChange={(e) => setF({ ...f, header: e.target.value })} />
          <div className="col-span-2">
            <SliderRow label="Expires after" min={0} max={720} value={Number(f.expires) || 0} onChange={(v) => setF({ ...f, expires: v ? String(v) : "" })}
              format={(v) => (v === 0 ? "Never" : v >= 48 ? `${Math.round(v / 24)} d` : `${v} h`)} />
          </div>
          <div className="col-span-2 py-1"><AgentPicker agents={agents} value={f.agent_ids} onChange={(agent_ids) => setF({ ...f, agent_ids })} /></div>
          {error && <p role="alert" className="col-span-2 text-destructive">{error}</p>}
          <div className="col-span-2"><Button>Add secret</Button></div>
        </form>
      </Section>
    </>
  );
}

export function LoginToggle({ status, reload }: { status: Status; reload: () => void }) {
  const [busy, setBusy] = useState(false);
  const toggle = async () => {
    setBusy(true);
    await rpc(status.service_installed ? "service.uninstall" : "service.install").catch((e) => window.alert(String(e)));
    // the daemon restarts under the login service; give it a moment before asking again
    setTimeout(() => { reload(); setBusy(false); }, 2500);
  };
  return (
    <Row label="Start at login"
      detail={status.service_installed ? "Burrow starts at login, so schedules run even when the app is closed." : "Burrow runs only while started by the app."}>
      <Switch aria-label="Start at login" disabled={busy} checked={status.service_installed} onCheckedChange={toggle} />
    </Row>
  );
}

type Page = "general" | "models" | "search" | "email" | "secrets" | "costs" | "data";
const PAGES: SettingsPage<Page>[] = [
  { id: "general", label: "General", group: "Burrow", icon: <Settings2Icon />, keywords: "login startup background sandbox keychain" },
  { id: "models", label: "Models", group: "Burrow", icon: <CpuIcon />, keywords: "download qwen llama hardware ram gpu cloud anthropic claude openai api key base url" },
  { id: "search", label: "Web search", group: "Burrow", icon: <SearchIcon />, keywords: "searxng brave" },
  { id: "email", label: "Email", group: "Access", icon: <MailIcon />, keywords: "email imap smtp app password connections" },
  { id: "secrets", label: "Secrets", group: "Access", icon: <KeyRoundIcon />, keywords: "token header keychain host" },
  { id: "costs", label: "Costs", group: "Data", icon: <ReceiptIcon />, keywords: "spend budget prices usd" },
  { id: "data", label: "Backup and data", group: "Data", icon: <ArchiveIcon />, keywords: "export import wipe passphrase" },
];

export default function Settings({ open, onOpenChange }: { open: boolean; onOpenChange: (o: boolean) => void }) {
  const [page, setPage] = useState<Page>("general");
  const [hardware, setHardware] = useState<Hardware | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const reload = () => rpc<Status>("status").then(setStatus);
  useEffect(() => { if (open) { rpc<Hardware>("hardware.detect").then(setHardware); reload(); } }, [open]);
  return (
    <SettingsView title="Settings" open={open} onOpenChange={onOpenChange} pages={PAGES} page={page} onPage={setPage}>
      {status && page === "general" && (
        <Section>
          <LoginToggle status={status} reload={reload} />
          <Row label="Sandbox" detail={status.sandbox ? `Each agent runs in its own ${status.sandbox} sandbox.` : "None installed, so shell is off. Install Apple container, Podman or Docker."}>
            <Badge variant="outline" className={status.sandbox ? "gap-1 text-success" : "text-destructive"}>
              {status.sandbox ? <><CheckIcon />{status.sandbox}</> : "Off"}
            </Badge>
          </Row>
          <Row label="Key storage" detail={status.keystore} />
        </Section>
      )}
      {page === "models" && (
        <>
          <Section title="On this machine" description={hardware && `${hardware.ram_gb} GB RAM, ${hardware.cores} cores, ${hardware.gpu}. Nothing leaves your computer.`}>
            <ModelList hardware={hardware} />
          </Section>
          <Cloud />
        </>
      )}
      {status && page === "search" && <Search status={status} reload={reload} />}
      {page === "email" && <Connections />}
      {page === "secrets" && <Secrets />}
      {page === "costs" && <Costs />}
      {page === "data" && <Data />}
    </SettingsView>
  );
}
