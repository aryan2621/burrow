import { useEffect, useState } from "react";
import { ArrowLeftIcon, ArrowRightIcon, CheckIcon } from "lucide-react";
import { Hardware, Status, rpc } from "./rpc";
import { Mark } from "./Icons";
import { Button } from "@/components/ui/button";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Cloud, LoginToggle, ModelList } from "./Settings";
import { TEMPLATES, attachPlugins } from "./templates";

const STEPS = ["This computer", "Model", "Sandbox", "Background", "First agent"];
const SANDBOX_HINT: Record<string, string> = {
  macos: "Install Apple's container tool (macOS 26): brew install container && container system start",
  linux: "Install Podman (e.g. sudo apt install podman) or Docker.",
  windows: "Install Podman Desktop or Docker Desktop.",
};
/** First run: hardware → model → sandbox → start at login → your first agent from a template. */
export default function Onboarding({ onDone }: { onDone: (agentId: number | null) => void }) {
  const [step, setStep] = useState(0);
  const [hw, setHw] = useState<Hardware | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState("");
  const [tpl, setTpl] = useState(TEMPLATES[0]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const loadModels = () => rpc<string[]>("models.list").then((m) => { setModels(m); setModel((cur) => cur || m[0] || ""); });
  const loadStatus = () => rpc<Status>("status").then(setStatus);
  useEffect(() => { rpc<Hardware>("hardware.detect").then(setHw); loadStatus(); loadModels(); }, []);
  // a cloud key added in the model step adds cloud models
  useEffect(() => { if (step === 4) loadModels(); }, [step]);

  const finish = async (create: boolean) => {
    setError("");
    try {
      let id: number | null = null;
      if (create) {
        setBusy(true);
        id = await rpc<number>("agents.create", { name: tpl.name, role_prompt: tpl.role, model_ref: model, folders: [], allowed_domains: [] });
        try {
          await attachPlugins(id, tpl.plugins);
        } catch (e) {
          await rpc("agents.delete", { agent_id: id }).catch(() => {});
          throw e;
        }
      }
      await rpc("settings.set", { key: "onboarded", value: "1" });
      onDone(id);
    } catch (e) {
      setError(String(e));
    }
    setBusy(false);
  };

  const TITLES = [
    ["Your computer", "Burrow runs AI agents on this machine. Nothing leaves it unless an agent is allowed to reach a site."],
    ["A model", "Download one to run built in, or add a cloud key."],
    ["A sandbox", "Each agent’s shell runs in its own isolated container."],
    ["Background", "Agents can keep working on schedules when the window is closed."],
    ["Your first agent", "Start from a template. Folders and tools can be added later."],
  ];
  const last = step === STEPS.length - 1;
  const nav = (
    <div className="flex items-center justify-between border-t pt-5">
      <Button variant="ghost" disabled={step === 0 || busy} onClick={() => setStep(step - 1)}><ArrowLeftIcon />Back</Button>
      {last ? (
        <div className="flex gap-2">
          <Button variant="outline" disabled={busy} onClick={() => finish(false)}>Skip</Button>
          <Button disabled={busy || !model} onClick={() => finish(true)}>{busy ? "Creating…" : `Create ${tpl.name}`}</Button>
        </div>
      ) : (
        <div className="flex gap-2">
          {step === 1 && <Button variant="outline" onClick={() => setStep(2)}>Skip</Button>}
          <Button onClick={() => setStep(step + 1)}>Next<ArrowRightIcon /></Button>
        </div>
      )}
    </div>
  );
  return (
    <div className="grid h-full place-items-center overflow-y-auto p-6">
      <div className="w-full max-w-xl">
        <div className="mb-6 flex items-center gap-2"><Mark className="size-6" /><span className="text-lg font-semibold tracking-tight">Burrow</span></div>
        <ol className="flex gap-1.5" aria-label="Setup steps">
          {STEPS.map((s, i) => (
            <li key={s} aria-current={i === step ? "step" : undefined} aria-label={`${s}${i < step ? ", done" : ""}`} className={`h-1 flex-1 rounded-full ${i <= step ? "bg-primary" : "bg-border"}`} />
          ))}
        </ol>
        <p className="mt-3 text-xs text-muted-foreground">Step {step + 1} of {STEPS.length}</p>
        <div className="mt-2 rounded-xl border bg-card p-6">
          <h1 className="title text-2xl">{TITLES[step][0]}</h1>
          <p className="mt-1 text-muted-foreground">{TITLES[step][1]}</p>
          <div className="mt-5 space-y-4">
            {step === 0 && (
              <>
                {hw ? (
                  <dl className="grid grid-cols-2 gap-4 text-sm">
                    {([["Memory", `${hw.ram_gb} GB`], ["Cores", String(hw.cores)], ["System", `${hw.os}/${hw.arch}`], ["Graphics", hw.gpu === "metal" ? "Apple GPU" : "CPU"]] as const).map(([k, v]) => (
                      <div key={k}><dt className="text-xs text-muted-foreground">{k}</dt><dd className="mt-0.5 font-medium tabular-nums">{v}</dd></div>
                    ))}
                  </dl>
                ) : <p className="text-sm text-muted-foreground">Checking…</p>}
                {hw && !hw.recommended && <p className="text-sm text-destructive">Less than 8 GB of RAM: built-in models will be slow. A cloud key (Settings → Models) works instead.</p>}
              </>
            )}
            {step === 1 && (
              <>
                <p className="text-sm text-muted-foreground">The recommended model fits this computer’s memory.</p>
                <ModelList hardware={hw} onChange={loadModels} />
                <p className="text-sm text-muted-foreground">Built-in models handle short tasks; a cloud model is more reliable for working through an inbox.</p>
                <Cloud />
              </>
            )}
            {step === 2 && status && (status.sandbox ? (
              <p className="flex items-center gap-2 text-sm"><CheckIcon className="size-4 text-success" />Sandbox ready: <code>{status.sandbox}</code></p>
            ) : (
              <>
                <p className="text-sm">No sandbox runtime found, so agents can’t run shell commands yet. Everything else works.</p>
                <p className="rounded-lg bg-muted p-3 font-mono text-xs">{SANDBOX_HINT[hw?.os ?? ""] ?? SANDBOX_HINT.linux}</p>
                <Button variant="outline" size="sm" onClick={loadStatus}>Check again</Button>
              </>
            ))}
            {step === 3 && status && (
              <>
                <p className="text-sm text-muted-foreground">To keep them going after a restart, start Burrow at login.</p>
                <LoginToggle status={status} reload={loadStatus} />
              </>
            )}
            {step === 4 && (
              <>
                <div className="grid grid-cols-2 gap-2 sm:grid-cols-3">
                  {TEMPLATES.filter((t) => t.name !== "Inbox helper" && !("key" in t)).map((t) => (
                    <button key={t.name} type="button" aria-pressed={tpl.name === t.name} onClick={() => setTpl(t)}
                      className={`flex items-center gap-2.5 rounded-xl border p-2.5 text-left text-sm ${tpl.name === t.name ? "border-primary bg-primary/5" : "hover:border-primary/40"}`}>
                      <span className="grid size-7 shrink-0 place-items-center rounded-lg bg-muted text-muted-foreground">{t.icon}</span>{t.name}
                    </button>
                  ))}
                </div>
                <div className="space-y-1.5">
                  <label htmlFor="ob-model" className="text-sm font-medium">Model</label>
                  <Select value={model} onValueChange={setModel}>
                    <SelectTrigger id="ob-model" className="w-full font-mono text-xs"><SelectValue placeholder="No model yet: go back and download one" /></SelectTrigger>
                    <SelectContent>{models.map((m) => <SelectItem key={m} value={m} className="font-mono text-xs">{m}</SelectItem>)}</SelectContent>
                  </Select>
                </div>
                {error && <p role="alert" className="text-sm text-destructive">{error}</p>}
              </>
            )}
          </div>
          <div className="mt-6">{nav}</div>
        </div>
      </div>
    </div>
  );
}
