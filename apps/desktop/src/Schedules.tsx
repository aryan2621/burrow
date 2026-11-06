import { useEffect, useState } from "react";
import { CalendarClockIcon, ClockIcon, FolderOpenIcon, FolderSyncIcon, MailIcon, PlayIcon, PlusIcon, RepeatIcon, Trash2Icon, WorkflowIcon } from "lucide-react";
import { Agent, Routine, Schedule, confirmAction, pickFolder, rpc } from "./rpc";
import { Row, Section, Status } from "@/components/settings";
import { EmptyState } from "@/components/panel";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Item, ItemActions, ItemContent, ItemDescription, ItemGroup, ItemMedia, ItemTitle } from "@/components/ui/item";
import { Switch } from "@/components/ui/switch";
import { Textarea } from "@/components/ui/textarea";
import { ToggleGroup, ToggleGroupItem } from "@/components/ui/toggle-group";

const KINDS: { kind: Schedule["kind"]; label: string; icon: React.ReactNode; hint: string }[] = [
  { kind: "cron", label: "At times", icon: <ClockIcon />, hint: "0 9 * * 1-5  (min hour day month weekday)" },
  { kind: "interval", label: "Repeat", icon: <RepeatIcon />, hint: "3600  (seconds, at least 60)" },
  { kind: "file", label: "Files change", icon: <FolderSyncIcon />, hint: "a folder or file inside the agent's folders" },
  { kind: "email", label: "New email", icon: <MailIcon />, hint: "email connection name (new mail in INBOX)" },
];
const KIND = Object.fromEntries(KINDS.map((k) => [k.kind, k])) as Record<Schedule["kind"], (typeof KINDS)[number]>;
const PRESETS: Partial<Record<Schedule["kind"], [string, string][]>> = {
  cron: [["Weekdays 9:00", "0 9 * * 1-5"], ["Daily 18:00", "0 18 * * *"], ["Hourly", "0 * * * *"], ["Mondays 8:00", "0 8 * * 1"]],
  interval: [["15 min", "900"], ["1 hour", "3600"], ["6 hours", "21600"]],
};
const PROMPT = "prompt";
const ON = "data-[state=on]:bg-accent data-[state=on]:font-medium";

function useAutomation(agent: Agent) {
  const [schedules, setSchedules] = useState<Schedule[]>([]);
  const [routines, setRoutines] = useState<Routine[]>([]);
  const [msg, setMsg] = useState("");
  const load = () => {
    rpc<Schedule[]>("schedules.list", { agent_id: agent.id }).then(setSchedules, (e) => setMsg(String(e)));
    rpc<Routine[]>("routines.list", { agent_id: agent.id }).then(setRoutines, (e) => setMsg(String(e)));
  };
  useEffect(() => void load(), [agent.id]);
  const call = (method: string, params: object, done = "") => {
    setMsg("");
    return rpc(method, params).then(() => { setMsg(done); load(); }, (e) => { setMsg(String(e)); throw e; });
  };
  return { schedules, routines, msg, call };
}

/** Cron, interval, file-change and email schedules; runs happen in the background, even with the window closed. */
export function SchedulesPage({ agent }: { agent: Agent }) {
  const { schedules, routines, msg, call } = useAutomation(agent);
  const [f, setF] = useState({ kind: "cron" as Schedule["kind"], spec: "", prompt: "", routine_id: null as number | null });
  const add = (e: React.FormEvent) => {
    e.preventDefault();
    call("schedules.add", { agent_id: agent.id, ...f }, "Schedule added").then(() => setF({ ...f, spec: "", prompt: "", routine_id: null }), () => {});
  };
  const when = (s: Schedule) =>
    s.kind === "file" ? "Runs on change" : s.kind === "email" ? "Checks every minute" : s.next_run_at ? `Next: ${new Date(s.next_run_at * 1000).toLocaleString()}` : "";

  return (
    <>
      <Section title="Active" description="They run in the background, even with the window closed, and wait while the battery is low.">
        {schedules.length === 0 && <EmptyState icon={<CalendarClockIcon />} title="No schedules yet">Add one below to have {agent.name} work on its own.</EmptyState>}
        <ItemGroup className="gap-2 py-2">
        {schedules.map((s) => (
          <Item key={s.id} variant="outline" size="sm" className={`bg-card ${s.enabled ? "" : "opacity-60"}`}>
            <ItemMedia variant="icon">{KIND[s.kind].icon}</ItemMedia>
            <ItemContent className="min-w-0">
              <ItemTitle className="flex-wrap">{KIND[s.kind].label}<code className="rounded bg-secondary px-1.5 py-0.5 text-xs font-normal">{s.spec}</code></ItemTitle>
              <ItemDescription>{s.routine_id !== null ? `Routine: ${routines.find((r) => r.id === s.routine_id)?.name ?? s.routine_id}` : s.prompt}</ItemDescription>
              <span className="text-xs text-muted-foreground">{s.enabled ? when(s) : "Paused"}</span>
            </ItemContent>
            <ItemActions>
            <Switch aria-label={s.enabled ? "Pause schedule" : "Resume schedule"} checked={s.enabled}
              onCheckedChange={(enabled) => call("schedules.update", { id: s.id, enabled }).catch(() => {})} />
            <Button size="icon-sm" variant="ghost" aria-label="Delete schedule" className="hover:text-destructive"
              onClick={async () => (await confirmAction("Delete this schedule?")) && call("schedules.delete", { id: s.id }).catch(() => {})}><Trash2Icon /></Button>
            </ItemActions>
          </Item>
        ))}
        </ItemGroup>
      </Section>

      <form onSubmit={add}>
        <Section title="New schedule">
          <Row label="Trigger" detail="What starts a run." stack>
            <ToggleGroup type="single" variant="outline" size="sm" aria-label="Trigger" className="w-full"
              value={f.kind} onValueChange={(k) => k && setF({ ...f, kind: k as Schedule["kind"], spec: "" })}>
              {KINDS.map((k) => <ToggleGroupItem key={k.kind} value={k.kind} className={`flex-1 gap-1.5 text-xs ${ON}`}>{k.icon}{k.label}</ToggleGroupItem>)}
            </ToggleGroup>
          </Row>
          <Row label="When" stack detail={PRESETS[f.kind] && (
            <span className="mt-1 flex flex-wrap gap-1.5">
              {PRESETS[f.kind]!.map(([label, spec]) => (
                <button key={spec} type="button" aria-pressed={f.spec === spec} onClick={() => setF({ ...f, spec })}
                  className={`rounded-full border px-2.5 py-0.5 text-xs hover:bg-accent ${f.spec === spec ? "border-primary bg-accent font-medium" : "bg-card text-foreground"}`}>
                  {label}
                </button>
              ))}
            </span>
          )}>
            <Input aria-label="When" required className="font-mono" placeholder={KIND[f.kind].hint} value={f.spec} onChange={(e) => setF({ ...f, spec: e.target.value })} />
            {f.kind === "file" && (
              <Button type="button" variant="outline" onClick={async () => { const p = await pickFolder(); if (p) setF({ ...f, spec: p }); }}><FolderOpenIcon />Choose…</Button>
            )}
          </Row>
          {routines.length > 0 && (
            <Row label="Do">
              <Select value={f.routine_id === null ? PROMPT : String(f.routine_id)}
                onValueChange={(v) => setF({ ...f, routine_id: v === PROMPT ? null : Number(v) })}>
                <SelectTrigger aria-label="Routine" className="w-60"><SelectValue /></SelectTrigger>
                <SelectContent>
                  <SelectItem value={PROMPT}>Write a task</SelectItem>
                  {routines.map((r) => <SelectItem key={r.id} value={String(r.id)}>Routine: {r.name}</SelectItem>)}
                </SelectContent>
              </Select>
            </Row>
          )}
          {f.routine_id === null && (
            <Row label="Task" detail="What the agent should do each time." stack>
              <Textarea aria-label="Task" rows={3} required placeholder="e.g. Summarise new files in my notes folder" value={f.prompt} onChange={(e) => setF({ ...f, prompt: e.target.value })} />
            </Row>
          )}
        </Section>
        <div className="flex items-center gap-3 border-t pt-4">
          <Button><PlusIcon />Add schedule</Button>
          <Status msg={msg} />
        </div>
      </form>
    </>
  );
}

/** Runs saved from the Activity tab, to run again or from a schedule. */
export function RoutinesPage({ agent }: { agent: Agent }) {
  const { routines, msg, call } = useAutomation(agent);
  return (
    <Section description="A routine replays a run that went well: same task, same tools, same approvals. Save one from a run in the Activity tab.">
      {routines.length === 0 && <EmptyState icon={<WorkflowIcon />} title="No routines yet">Open Activity and pick “Save as routine” on a finished run.</EmptyState>}
      <ItemGroup className="gap-2 py-2">
      {routines.map((r) => (
        <Item key={r.id} variant="outline" size="sm" className="bg-card">
          <ItemMedia variant="icon"><WorkflowIcon /></ItemMedia>
          <ItemContent className="min-w-0">
            <ItemTitle>{r.name}</ItemTitle>
            <ItemDescription>Tools: {r.tools.join(", ") || "none"}{r.approved.length > 0 && ` · pre-approved: ${r.approved.join(", ")}`}</ItemDescription>
          </ItemContent>
          <ItemActions>
          <Button size="sm" variant="outline" onClick={() => call("routines.run", { id: r.id }, `Started ${r.name}`).catch(() => {})}><PlayIcon />Run now</Button>
          <Button size="icon-sm" variant="ghost" aria-label={`Delete routine ${r.name}`} className="hover:text-destructive"
            onClick={async () => (await confirmAction(`Delete routine ${r.name}?`)) && call("routines.delete", { id: r.id }).catch(() => {})}><Trash2Icon /></Button>
          </ItemActions>
        </Item>
      ))}
      </ItemGroup>
      <Status msg={msg} />
    </Section>
  );
}
