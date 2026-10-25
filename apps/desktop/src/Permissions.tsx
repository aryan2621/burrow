import { DEFAULT_RULES, Rule, TOOLS, rpc } from "./rpc";

export type Perms = { allowed_domains: string; approval_rules: Record<string, Rule> };

const LABEL: Record<string, string> = {
  list_dir: "List folders", read_file: "Read files", write_file: "Write files (undoable)",
  delete_file: "Delete files (undoable)", shell: "Run shell in sandbox", http_fetch: "Web access to non-allowlisted sites",
  notes_add: "Save notes (undoable)", notes_search: "Search notes", web_search: "Web search (if configured)",
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
