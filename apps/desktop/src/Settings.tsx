import { useEffect, useState } from "react";
import { Agent, CatalogModel, Hardware, Secret, Status, rpc, ui } from "./rpc";
import useDownloads from "./useDownloads";

const gb = (b: number) => `${(b / 2 ** 30).toFixed(1)} GB`;

export function ModelList({ hardware, onChange }: { hardware: Hardware | null; onChange?: () => void }) {
  const [models, setModels] = useState<CatalogModel[]>([]);
  const [error, setError] = useState("");
  const load = () => rpc<CatalogModel[]>("models.catalog").then(setModels, (e) => setError(String(e)));
  const { progress, errors } = useDownloads(() => { load(); onChange?.(); });
  useEffect(() => void load(), []);
  const call = (method: string, id: string) => rpc(method, { id }).then(load, (e) => setError(String(e)));
  return (
    <ul className="space-y-2">
      {models.map((m) => (
        <li key={m.id} className="flex items-center justify-between gap-3 rounded border border-stone-200 p-2 text-sm dark:border-stone-700">
          <span>
            <span className="font-semibold">{m.name}</span> <span className="text-stone-500">· {gb(m.size)} · needs {m.min_ram_gb} GB RAM</span>
            {hardware?.recommended === m.id && <span className="ml-2 rounded bg-amber-100 px-1.5 text-xs text-amber-900">Recommended</span>}
            {errors[m.id] && <span role="alert" className="block text-xs text-red-600">{errors[m.id]}</span>}
          </span>
          {m.installed ? (
            <span className="text-green-700">Installed</span>
          ) : m.downloading || progress[m.id] !== undefined ? (
            <span className="flex items-center gap-2">
              <progress max={100} value={progress[m.id] ?? 0} aria-label={`${m.name} download`} className="w-28" />
              <button className={ui.button} onClick={() => call("models.cancel", m.id)}>Cancel</button>
            </span>
          ) : (
            <button className={ui.button} onClick={() => call("models.download", m.id)}>Download</button>
          )}
        </li>
      ))}
      {error && <li role="alert" className="text-sm text-red-600">{error}</li>}
    </ul>
  );
}

function Search({ status, reload }: { status: Status; reload: () => void }) {
  const [url, setUrl] = useState("");
  const [key, setKey] = useState("");
  const [msg, setMsg] = useState("");
  const set = (p: object) => rpc("search.configure", p).then(() => { setMsg("Saved"); setUrl(""); setKey(""); reload(); }, (e) => setMsg(String(e)));
  return (
    <div className="space-y-2 text-sm">
      <p>Current: {status.search === "searxng" ? "SearXNG (local)" : status.search === "brave" ? "Brave Search API" : "off — agents don't get a web_search tool"}</p>
      <div className="flex gap-2">
        <input aria-label="SearXNG URL" className={ui.input} placeholder="SearXNG URL, e.g. http://localhost:8888" value={url} onChange={(e) => setUrl(e.target.value)} />
        <button className={ui.button} disabled={!url} onClick={() => set({ searxng_url: url })}>Save</button>
        <button className={ui.button} onClick={() => set({ searxng_url: null })}>Clear</button>
      </div>
      <div className="flex gap-2">
        <input aria-label="Brave Search API key" type="password" className={ui.input} placeholder="Brave Search API key (stored in the keychain)" value={key} onChange={(e) => setKey(e.target.value)} />
        <button className={ui.button} disabled={!key} onClick={() => set({ brave_key: key })}>Save</button>
        <button className={ui.button} onClick={() => set({ brave_key: null })}>Clear</button>
      </div>
      {msg && <p role="status">{msg}</p>}
    </div>
  );
}

function Secrets() {
  const [list, setList] = useState<Secret[]>([]);
  const [agents, setAgents] = useState<Agent[]>([]);
  const [error, setError] = useState("");
  const [f, setF] = useState({ name: "", value: "", hosts: "", header: "Authorization: Bearer {}", agent_ids: [] as number[] });
  const load = () => rpc<Secret[]>("secrets.list").then(setList, (e) => setError(String(e)));
  useEffect(() => { load(); rpc<Agent[]>("agents.list").then(setAgents); }, []);
  const add = (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    rpc("secrets.set", { ...f, hosts: f.hosts.split(",").map((h) => h.trim()).filter(Boolean) })
      .then(() => { setF({ ...f, name: "", value: "", hosts: "" }); load(); }, (e) => setError(String(e)));
  };
  const name = (id: number) => agents.find((a) => a.id === id)?.name ?? `#${id}`;
  return (
    <div className="space-y-2 text-sm">
      <p className="text-stone-500">Values go to the OS keychain. Agents never see them: the daemon adds them to requests for the listed hosts and scrubs them from output.</p>
      <ul className="space-y-1">
        {list.map((s) => (
          <li key={s.name} className="flex justify-between gap-2">
            <span><span className="font-mono">{s.name}</span> → {s.hosts.join(", ")} · {s.agent_ids.map(name).join(", ") || "no agents"}</span>
            <button className={ui.button} onClick={() => confirm(`Delete secret ${s.name}?`) && rpc("secrets.delete", { name: s.name }).then(load)}>Delete</button>
          </li>
        ))}
      </ul>
      <form onSubmit={add} className="space-y-2">
        <div className="flex gap-2">
          <input aria-label="Secret name" className={ui.input} required placeholder="name" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} />
          <input aria-label="Secret value" type="password" className={ui.input} required placeholder="value" value={f.value} onChange={(e) => setF({ ...f, value: e.target.value })} />
        </div>
        <input aria-label="Hosts" className={ui.input} required placeholder="hosts, e.g. api.github.com" value={f.hosts} onChange={(e) => setF({ ...f, hosts: e.target.value })} />
        <input aria-label="Header" className={ui.input} value={f.header} onChange={(e) => setF({ ...f, header: e.target.value })} />
        <fieldset className="flex flex-wrap gap-3">
          <legend className="sr-only">Agents that may use it</legend>
          {agents.map((a) => (
            <label key={a.id} className="flex items-center gap-1">
              <input type="checkbox" checked={f.agent_ids.includes(a.id)}
                onChange={(e) => setF({ ...f, agent_ids: e.target.checked ? [...f.agent_ids, a.id] : f.agent_ids.filter((i) => i !== a.id) })} />
              {a.name}
            </label>
          ))}
        </fieldset>
        {error && <p role="alert" className="text-red-600">{error}</p>}
        <button className={ui.primary}>Add secret</button>
      </form>
    </div>
  );
}

export function LoginToggle({ status, reload }: { status: Status; reload: () => void }) {
  const [busy, setBusy] = useState(false);
  const toggle = async () => {
    setBusy(true);
    await rpc(status.service_installed ? "service.uninstall" : "service.install").catch(() => {});
    // the daemon restarts under the login service; give it a moment before asking again
    setTimeout(() => { reload(); setBusy(false); }, 2500);
  };
  return (
    <div className="flex items-center justify-between text-sm">
      <span>{status.service_installed ? "Burrow starts at login, so schedules run even when the app is closed." : "Burrow runs only while started by the app."}</span>
      <button className={ui.button} disabled={busy} onClick={toggle}>{status.service_installed ? "Don't start at login" : "Start at login"}</button>
    </div>
  );
}

export default function Settings() {
  const [hardware, setHardware] = useState<Hardware | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const reload = () => rpc<Status>("status").then(setStatus);
  useEffect(() => { rpc<Hardware>("hardware.detect").then(setHardware); reload(); }, []);
  const section = "space-y-2 border-b border-stone-200 pb-4 dark:border-stone-700";
  return (
    <div className="h-full max-w-2xl space-y-4 overflow-y-auto p-6">
      <h2 className="text-lg font-semibold">Settings</h2>
      <section className={section}>
        <h3 className="font-semibold">Built-in models</h3>
        {hardware && <p className="text-sm text-stone-500">This machine: {hardware.ram_gb} GB RAM, {hardware.cores} cores, {hardware.gpu}.</p>}
        <ModelList hardware={hardware} />
      </section>
      {status && (
        <>
          <section className={section}><h3 className="font-semibold">Web search</h3><Search status={status} reload={reload} /></section>
          <section className={section}><h3 className="font-semibold">Secrets</h3><Secrets /></section>
          <section className={section}>
            <h3 className="font-semibold">Background</h3>
            <LoginToggle status={status} reload={reload} />
            <p className="text-sm text-stone-500">Sandbox: {status.sandbox ?? "none installed — shell disabled"} · Keys: {status.keystore}</p>
          </section>
        </>
      )}
    </div>
  );
}
