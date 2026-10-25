import { useEffect, useState } from "react";
import { rpc } from "./rpc";
import Permissions, { Perms, splitDomains } from "./Permissions";

export default function AgentForm({ onCreated }: { onCreated: (id: number) => void }) {
  const [models, setModels] = useState<string[]>([]);
  const [pull, setPull] = useState("qwen3:8b");
  const [pulling, setPulling] = useState(false);
  const [error, setError] = useState("");
  const [f, setF] = useState({ name: "", role_prompt: "You are a helpful assistant.", model_ref: "", folders: "" });
  const [perms, setPerms] = useState<Perms>({ allowed_domains: "", approval_rules: {} });

  const loadModels = () =>
    rpc<string[]>("models.list").then((m) => {
      setModels(m);
      setF((f) => ({ ...f, model_ref: f.model_ref || m[0] || "" }));
    });
  useEffect(() => void loadModels().catch((e) => setError(String(e))), []);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    try {
      const folders = f.folders.split(",").map((s) => s.trim()).filter(Boolean);
      const allowed_domains = splitDomains(perms.allowed_domains);
      onCreated(await rpc<number>("agents.create", { ...f, folders, allowed_domains, approval_rules: perms.approval_rules }));
    } catch (e) {
      setError(String(e));
    }
  };
  const doPull = async () => {
    setPulling(true);
    setError("");
    try {
      await rpc("models.pull", { name: pull });
      await loadModels();
    } catch (e) {
      setError(String(e));
    }
    setPulling(false);
  };

  const input = "w-full rounded border border-stone-300 bg-white px-2 py-1 dark:border-stone-700 dark:bg-stone-800";
  return (
    <form onSubmit={submit} className="mx-auto max-w-lg space-y-3 p-6">
      <h2 className="text-lg font-semibold">New agent</h2>
      <label className="block">
        <span className="text-sm">Name</span>
        <input className={input} required value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} />
      </label>
      <label className="block">
        <span className="text-sm">Role prompt</span>
        <textarea className={input} rows={4} required value={f.role_prompt}
          onChange={(e) => setF({ ...f, role_prompt: e.target.value })} />
      </label>
      <label className="block">
        <span className="text-sm">Model</span>
        <select className={input} required value={f.model_ref} onChange={(e) => setF({ ...f, model_ref: e.target.value })}>
          {models.length === 0 && <option value="">No models found — is Ollama running?</option>}
          {models.map((m) => <option key={m}>{m}</option>)}
        </select>
      </label>
      <div className="flex gap-2">
        <input className={input} aria-label="Ollama model to pull" value={pull} onChange={(e) => setPull(e.target.value)} />
        <button type="button" onClick={doPull} disabled={pulling}
          className="shrink-0 rounded border border-stone-300 px-3 dark:border-stone-700">
          {pulling ? "Pulling…" : "Pull"}
        </button>
      </div>
      <label className="block">
        <span className="text-sm">Folders the agent may use (absolute paths, comma-separated)</span>
        <input className={input} required placeholder="/Users/me/Documents/notes" value={f.folders}
          onChange={(e) => setF({ ...f, folders: e.target.value })} />
      </label>
      <Permissions value={perms} onChange={setPerms} />
      {error && <p role="alert" className="text-sm text-red-600">{error}</p>}
      <button className="rounded bg-amber-800 px-4 py-1.5 text-white">Create</button>
    </form>
  );
}
