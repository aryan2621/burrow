import { useEffect, useState } from "react";
import { Hardware, Status, pickFolder, rpc, ui } from "./rpc";
import { LoginToggle, ModelList } from "./Settings";

const STEPS = ["This computer", "Model", "Sandbox", "Background", "First agent"];
const SANDBOX_HINT: Record<string, string> = {
  macos: "Install Apple's container tool (macOS 26): brew install container && container system start",
  linux: "Install Podman (e.g. sudo apt install podman) or Docker.",
  windows: "Install Podman Desktop or Docker Desktop.",
};
const TEMPLATE = `You are a research and documents assistant. You search the web and read the user's documents, then write clear, \
well-structured reports into the user's folder, citing the URLs you used. Save key facts and decisions as notes so you can find \
them later. Ask before doing anything you can't undo.`;

/** First run: hardware → model → sandbox → start at login → a template agent. */
export default function Onboarding({ onDone }: { onDone: (agentId: number | null) => void }) {
  const [step, setStep] = useState(0);
  const [hw, setHw] = useState<Hardware | null>(null);
  const [status, setStatus] = useState<Status | null>(null);
  const [models, setModels] = useState<string[]>([]);
  const [model, setModel] = useState("");
  const [folder, setFolder] = useState("");
  const [error, setError] = useState("");

  const loadModels = () => rpc<string[]>("models.list").then((m) => { setModels(m); setModel((cur) => cur || m[0] || ""); });
  const loadStatus = () => rpc<Status>("status").then(setStatus);
  useEffect(() => { rpc<Hardware>("hardware.detect").then(setHw); loadStatus(); loadModels(); }, []);

  const finish = async (create: boolean) => {
    setError("");
    try {
      let id: number | null = null;
      if (create) {
        const { package_registries } = await rpc<{ package_registries: string[] }>("domains.presets");
        id = await rpc<number>("agents.create", {
          name: "Research assistant", role_prompt: TEMPLATE, model_ref: model, folders: [folder], allowed_domains: package_registries,
        });
      }
      await rpc("settings.set", { key: "onboarded", value: "1" });
      onDone(id);
    } catch (e) {
      setError(String(e));
    }
  };

  const next = <button className={ui.primary} onClick={() => setStep(step + 1)}>Next</button>;
  return (
    <div className="mx-auto max-w-xl space-y-4 p-6">
      <h2 className="text-lg font-semibold">Welcome to Burrow</h2>
      <ol className="flex gap-2 text-xs" aria-label="Setup steps">
        {STEPS.map((s, i) => (
          <li key={s} aria-current={i === step ? "step" : undefined} className={`rounded px-2 py-0.5 ${i === step ? "bg-amber-800 text-white" : "bg-stone-200 dark:bg-stone-800"}`}>{s}</li>
        ))}
      </ol>

      {step === 0 && (
        <section className="space-y-3">
          <p>Burrow runs AI agents entirely on this computer. Nothing leaves it unless an agent is allowed to reach a site.</p>
          {hw && <p className="text-sm">{hw.ram_gb} GB RAM · {hw.cores} cores · {hw.os}/{hw.arch} · {hw.gpu === "metal" ? "Apple GPU" : "CPU"}</p>}
          {hw && !hw.recommended && <p className="text-sm text-red-600">Less than 8 GB of RAM: built-in models will be slow. An Ollama server elsewhere works too.</p>}
          {next}
        </section>
      )}

      {step === 1 && (
        <section className="space-y-3">
          <p className="text-sm">Download a model to run built in. The recommended one fits this computer's memory.</p>
          <ModelList hardware={hw} onChange={loadModels} />
          {models.some((m) => m.startsWith("ollama:")) && <p className="text-sm text-stone-500">Ollama models were found too; you can pick one at the last step.</p>}
          <div className="flex gap-2">{next}<button className={ui.button} onClick={() => setStep(2)}>Skip</button></div>
        </section>
      )}

      {step === 2 && status && (
        <section className="space-y-3">
          {status.sandbox ? (
            <p>Sandbox ready: <span className="font-mono">{status.sandbox}</span>. Each agent's shell runs in its own isolated container.</p>
          ) : (
            <>
              <p>No sandbox runtime found, so agents can't run shell commands yet (everything else works).</p>
              <p className="rounded bg-stone-100 p-2 font-mono text-xs dark:bg-stone-800">{SANDBOX_HINT[hw?.os ?? ""] ?? SANDBOX_HINT.linux}</p>
              <button className={ui.button} onClick={loadStatus}>Check again</button>
            </>
          )}
          <div>{next}</div>
        </section>
      )}

      {step === 3 && status && (
        <section className="space-y-3">
          <p className="text-sm">Agents can keep working on schedules when the window is closed. To keep them going after a restart, start Burrow at login.</p>
          <LoginToggle status={status} reload={loadStatus} />
          <div>{next}</div>
        </section>
      )}

      {step === 4 && (
        <section className="space-y-3">
          <p className="text-sm">Create a research and documents assistant. It can read and write files only in the folder you choose; every change can be undone.</p>
          <label className="block text-sm">
            Model
            <select className={ui.input} value={model} onChange={(e) => setModel(e.target.value)}>
              {models.length === 0 && <option value="">No model yet — download one in step 2</option>}
              {models.map((m) => <option key={m}>{m}</option>)}
            </select>
          </label>
          <div className="flex gap-2">
            <input aria-label="Folder" className={ui.input} readOnly placeholder="Choose the folder it works in" value={folder} />
            <button className={ui.button} onClick={async () => setFolder((await pickFolder()) ?? folder)}>Choose…</button>
          </div>
          {error && <p role="alert" className="text-sm text-red-600">{error}</p>}
          <div className="flex gap-2">
            <button className={ui.primary} disabled={!model || !folder} onClick={() => finish(true)}>Create agent</button>
            <button className={ui.button} onClick={() => finish(false)}>Skip</button>
          </div>
        </section>
      )}
    </div>
  );
}
