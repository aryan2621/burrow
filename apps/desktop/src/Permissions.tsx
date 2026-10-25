import { useEffect, useState } from "react";
import { Agent, DEFAULT_RULES, Limits, Rule, TOOLS, rpc, ui } from "./rpc";

export type Perms = { allowed_domains: string; approval_rules: Record<string, Rule> };

const LABEL: Record<string, string> = {
  list_dir: "List folders", read_file: "Read files", write_file: "Write files (undoable)",
  delete_file: "Delete files (undoable)", shell: "Run shell in sandbox", http_fetch: "Web access to non-allowlisted sites",
  notes_add: "Save notes (undoable)", notes_search: "Search notes", web_search: "Web search (if configured)",
  email_search: "List emails", email_read: "Read emails", email_move: "Move emails (undoable)",
  email_send: "Send email (60 s to cancel)", calendar_list: "List events", calendar_create: "Create events (undoable)",
  calendar_update: "Change events (undoable)", calendar_delete: "Delete events (undoable)", github_issues: "Read GitHub issues",
  github_comment: "Comment on GitHub (undoable)", github_label: "Label issues (undoable)", github_close: "Close issues (undoable)",
};

/** Allowed domains plus allow/ask/deny per tool. Rules equal to the default are not stored. */
export default function Permissions({ value, onChange }: { value: Perms; onChange: (p: Perms) => void }) {
  const input = "w-full rounded border border-stone-300 bg-white px-2 py-1 dark:border-stone-700 dark:bg-stone-800";
  const addRegistries = async () => {
    const { package_registries } = await rpc<{ package_registries: string[] }>("domains.presets");
    const merged = [...new Set([...splitDomains(value.allowed_domains), ...package_registries])];
    onChange({ ...value, allowed_domains: merged.join(", ") });
  };
  return (
    <fieldset className="space-y-2">
      <label className="block">
        <span className="text-sm">Allowed domains (comma-separated, subdomains included)</span>
        <input className={input} placeholder="wikipedia.org, api.github.com" value={value.allowed_domains}
          onChange={(e) => onChange({ ...value, allowed_domains: e.target.value })} />
      </label>
      <button type="button" onClick={addRegistries}
        className="rounded border border-stone-300 px-2 py-0.5 text-sm hover:bg-stone-100 dark:border-stone-700 dark:hover:bg-stone-800">
        + Package registries (pip, npm, git)
      </button>
      <table className="w-full text-sm">
        <tbody>
          {TOOLS.map((t) => (
            <tr key={t}>
              <td className="py-0.5">{LABEL[t]}</td>
              <td className="text-right">
                <select aria-label={`${LABEL[t]} rule`} value={value.approval_rules[t] ?? DEFAULT_RULES[t]}
                  className="rounded border border-stone-300 bg-white px-1 dark:border-stone-700 dark:bg-stone-800"
                  onChange={(e) => {
                    const rules = { ...value.approval_rules, [t]: e.target.value as Rule };
                    if (rules[t] === DEFAULT_RULES[t]) delete rules[t];
                    onChange({ ...value, approval_rules: rules });
                  }}>
                  <option value="allow">Allow</option>
                  <option value="ask">Ask me</option>
                  <option value="deny">Deny</option>
                </select>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </fieldset>
  );
}

export const splitDomains = (s: string) => s.split(",").map((d) => d.trim()).filter(Boolean);

/** The agent page's Permissions tab: model, run limits, allowed domains and rules. */
export function PermissionsTab({ agent, onSaved }: { agent: Agent; onSaved: () => void }) {
  const [perms, setPerms] = useState<Perms>({ allowed_domains: agent.allowed_domains.join(", "), approval_rules: agent.approval_rules });
  const [model, setModel] = useState(agent.model_ref);
  const [models, setModels] = useState<string[]>([agent.model_ref]);
  const [limits, setLimits] = useState<Limits>(agent.limits);
  const [msg, setMsg] = useState("");
  useEffect(() => void rpc<string[]>("models.list").then((m) => setModels(m.includes(agent.model_ref) ? m : [agent.model_ref, ...m])), []);
  const save = () =>
    rpc("agents.update", { agent_id: agent.id, model_ref: model, limits,
      allowed_domains: splitDomains(perms.allowed_domains), approval_rules: perms.approval_rules })
      .then(() => { setMsg("Saved"); onSaved(); }, (e) => setMsg(String(e)));
  const num = (k: keyof Limits, label: string, scale = 1) => (
    <label className="block text-sm">
      {label}
      <input type="number" min={1} className={ui.input} value={(limits[k] as number) / scale}
        onChange={(e) => setLimits({ ...limits, [k]: Math.round(Number(e.target.value) * scale) })} />
    </label>
  );
  return (
    <div className="h-full max-w-lg space-y-3 overflow-y-auto p-4">
      <label className="block text-sm">
        Model
        <select className={ui.input} value={model} onChange={(e) => setModel(e.target.value)}>
          {models.map((m) => <option key={m}>{m}</option>)}
        </select>
        {/^(anthropic|openai):/.test(model) && <span className="text-xs text-stone-500">Cloud model: messages and tool results leave this machine.</span>}
      </label>
      <fieldset className="grid grid-cols-3 gap-2">
        <legend className="mb-1 text-sm font-semibold">Run limits</legend>
        {num("max_steps", "Steps")}
        {num("max_run_secs", "Minutes", 60)}
        {num("max_run_tokens", "Output tokens")}
      </fieldset>
      <label className="flex items-center gap-2 text-sm">
        <input type="checkbox" checked={limits.review} onChange={(e) => setLimits({ ...limits, review: e.target.checked })} />
        Review auto-allowed changes with a second model call (an objection asks you first)
      </label>
      <Permissions value={perms} onChange={setPerms} />
      <button onClick={save} className={ui.primary}>Save</button>
      {msg && <p role="status" className="text-sm">{msg}</p>}
    </div>
  );
}
