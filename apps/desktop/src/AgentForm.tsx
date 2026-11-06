import { useEffect, useRef, useState } from "react";
import { onEvent, rpc } from "./rpc";
import { TEMPLATES, attachPlugins } from "./templates";
import FoldersField from "./FoldersField";
import Permissions, { Perms, splitDomains } from "./Permissions";
import { Avatar, AvatarKind, AvatarPicker } from "./Avatars";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Disclosure, ErrorNote } from "@/components/panel";
import { Textarea } from "@/components/ui/textarea";

export default function AgentForm({ onCreated }: { onCreated: (id: number, avatar: AvatarKind) => void }) {
  const [models, setModels] = useState<string[]>([]);
  const [error, setError] = useState("");
  const [f, setF] = useState({ name: "", role_prompt: "", model_ref: "" });
  const [foldersList, setFolders] = useState<string[]>([]);
  const flushFolders = useRef<(() => string[]) | null>(null);
  const [plugins, setPlugins] = useState<string[]>([]);
  const [keyed, setKeyed] = useState<{ plugin: string; hint: string } | null>(null);
  const [keyValue, setKeyValue] = useState("");
  const [avatar, setAvatar] = useState<AvatarKind>("pebble");
  const [perms, setPerms] = useState<Perms>({ allowed_domains: "", approval_rules: {} });

  const loadModels = () =>
    rpc<string[]>("models.list").then((m) => {
      setModels(m);
      setF((f) => ({ ...f, model_ref: f.model_ref || m[0] || "" }));
    });
  useEffect(() => void loadModels().catch((e) => setError(String(e))), []);
  // a download finishing in Settings (open over this form) adds a model
  useEffect(() => {
    const un = onEvent(({ method }) => { if (method === "models.downloaded") void loadModels(); });
    return () => void un.then((f) => f());
  }, []);

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    setError("");
    try {
      const folders = flushFolders.current?.() ?? foldersList;
      const allowed_domains = splitDomains(perms.allowed_domains);
      const id = await rpc<number>("agents.create", { ...f, folders, allowed_domains, approval_rules: perms.approval_rules });
      await attachPlugins(id, plugins, keyed ? { plugin: keyed.plugin, value: keyValue } : undefined);
      onCreated(id, avatar);
    } catch (e) {
      setError(String(e));
    }
  };

  const missing = !f.name.trim() ? "Give it a name" : !f.model_ref ? "Pick a model" : "";

  const pickTemplate = (t: (typeof TEMPLATES)[number]) => {
    setF((f) => ({ ...f, name: t.name, role_prompt: t.role }));
    setAvatar(t.avatar); setPlugins(t.plugins); setKeyed(("key" in t && t.key) || null); setKeyValue("");
  };

  return (
    <form onSubmit={submit} className="h-full overflow-y-auto">
      <div className="mx-auto max-w-5xl px-8 py-8">
        <header>
          <h1 className="title text-2xl">New agent</h1>
          <p className="mt-1 text-muted-foreground">It gets its own sandbox, only the folders and sites you give it, and asks before anything risky.</p>
        </header>
        <ErrorNote msg={error} className="mt-4" />

        <div className="mt-6 grid items-start gap-8 lg:grid-cols-[1fr_272px]">
          <div className="space-y-8">
            <section aria-label="Start from">
              <h2 className="font-sans text-sm font-semibold">Start from</h2>
              <div className="mt-3 grid grid-cols-2 gap-2 sm:grid-cols-3">
                {TEMPLATES.map((t) => (
                  <button key={t.name} type="button" aria-pressed={f.name === t.name} onClick={() => pickTemplate(t)}
                    className={`flex items-center gap-2.5 rounded-xl border p-2.5 text-left text-sm transition-colors ${f.name === t.name ? "border-primary bg-primary/5" : "bg-card hover:border-primary/40"}`}>
                    <span className="grid size-7 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">{t.icon}</span>
                    <span className="truncate">{t.name}</span>
                  </button>
                ))}
              </div>
            </section>

            <Card title="Identity">
              <div className="flex items-center gap-4">
                <Avatar kind={avatar} className="size-14" />
                <div className="flex-1 space-y-1.5">
                  <Label htmlFor="agent-name">Name</Label>
                  <Input id="agent-name" required placeholder="e.g. Researcher" value={f.name} onChange={(e) => setF({ ...f, name: e.target.value })} />
                </div>
              </div>
              <div className="space-y-1.5">
                <Label>Avatar</Label>
                <AvatarPicker value={avatar} onChange={setAvatar} />
              </div>
              <div className="space-y-1.5">
                <Label htmlFor="agent-role">Role</Label>
                <Textarea id="agent-role" rows={5} required placeholder="e.g. You summarise my notes folder each morning." value={f.role_prompt} onChange={(e) => setF({ ...f, role_prompt: e.target.value })} />
                <p className="text-xs text-muted-foreground">It sees this before every message.</p>
              </div>
              {keyed && (
                <div className="space-y-1.5">
                  <Label htmlFor="plugin-key">{keyed.hint} (optional)</Label>
                  <Input id="plugin-key" type="password" placeholder="Add it later from the Tools tab if you skip this" value={keyValue} onChange={(e) => setKeyValue(e.target.value)} />
                </div>
              )}
            </Card>

            <Card title="Model" description={models.length === 0 ? "No models yet: download one in Settings → Models, or add a cloud key there." : "Local models keep everything on this machine."}>
              <Select value={f.model_ref} onValueChange={(model_ref) => setF({ ...f, model_ref })} required>
                <SelectTrigger aria-label="Model" className="w-full font-mono text-xs"><SelectValue placeholder="Pick a model" /></SelectTrigger>
                <SelectContent>{models.map((m) => <SelectItem key={m} value={m} className="font-mono text-xs">{m}</SelectItem>)}</SelectContent>
              </Select>
            </Card>

            <Disclosure title="Folders" summary={foldersList.length ? `${foldersList.length} folder${foldersList.length > 1 ? "s" : ""}` : "None: only its private scratch folder"}>
              <p className="text-sm text-muted-foreground">Optional. The only places on your disk it can read or change.</p>
              <FoldersField folders={foldersList} onChange={setFolders} flush={flushFolders} />
            </Disclosure>

            <Disclosure title="Permissions" summary={`${splitDomains(perms.allowed_domains).length} allowed sites · ${Object.keys(perms.approval_rules).length} custom rules`}>
              <Permissions value={perms} onChange={setPerms} />
            </Disclosure>
          </div>

          <aside aria-label="Summary" className="rounded-xl border bg-card p-5 lg:sticky lg:top-0">
            <div className="flex items-center gap-3">
              <Avatar kind={avatar} className="size-10" />
              <div className="min-w-0">
                <p className="truncate font-medium">{f.name.trim() || "Unnamed agent"}</p>
                <p className="truncate font-mono text-xs text-muted-foreground">{f.model_ref || "no model"}</p>
              </div>
            </div>
            <dl className="mt-4 space-y-2 border-t pt-4">
              {([["Folders", foldersList.length ? `${foldersList.length} chosen` : "Scratch only"],
                ["Sites", perms.allowed_domains.trim() ? `${splitDomains(perms.allowed_domains).length} allowed` : "None"],
                ["Tools", plugins.length ? `${plugins.length} attached` : "None yet"]] as const).map(([k, v]) => (
                <div key={k} className="flex justify-between gap-3"><dt className="text-muted-foreground">{k}</dt><dd>{v}</dd></div>
              ))}
            </dl>
            <Button size="lg" className="mt-5 w-full" disabled={!!missing}>Create agent</Button>
            {missing && <p className="mt-2 text-center text-xs text-muted-foreground">{missing}</p>}
          </aside>
        </div>
      </div>
    </form>
  );
}

export function Card({ title, description, children }: { title: string; description?: string; children: React.ReactNode }) {
  return (
    <section className="rounded-xl border bg-card p-5">
      <h2 className="font-sans text-sm font-semibold">{title}</h2>
      {description && <p className="mt-0.5 text-muted-foreground">{description}</p>}
      <div className="mt-4 space-y-4">{children}</div>
    </section>
  );
}
