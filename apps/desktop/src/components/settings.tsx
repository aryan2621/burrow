import { useEffect, useState, type ReactNode } from "react";
import { ArrowLeftIcon, SearchIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Slider } from "@/components/ui/slider";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";
import type { Agent } from "@/rpc";

export type SettingsPage<Id extends string> = { id: Id; label: string; group: string; icon: ReactNode; keywords?: string };

/** Settings as a page over the main area (not a modal): search + grouped icon nav on the left, one page on the right. Esc or Back closes it. */
export function SettingsView<Id extends string>({ open, onOpenChange, title, pages, page, onPage, children }: {
  open: boolean; onOpenChange: (o: boolean) => void; title: string;
  pages: SettingsPage<Id>[]; page: Id; onPage: (p: Id) => void; children: ReactNode;
}) {
  const [query, setQuery] = useState("");
  useEffect(() => {
    if (!open) return;
    const esc = (e: KeyboardEvent) => { if (e.key === "Escape" && !document.querySelector("[role=dialog],[role=alertdialog]")) onOpenChange(false); };
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [open, onOpenChange]);
  if (!open) return null;
  const q = query.trim().toLowerCase();
  const matches = pages.filter((p) => `${p.label} ${p.keywords ?? ""}`.toLowerCase().includes(q));
  const groups = [...new Set(matches.map((p) => p.group))];
  const current = pages.find((p) => p.id === page);
  return (
    <div className="fixed inset-0 z-20 flex bg-background">
      <nav aria-label={`${title} sections`} className="flex w-56 shrink-0 flex-col overflow-y-auto border-r bg-muted/50 p-3">
        <Button variant="ghost" size="sm" className="-ml-1 mb-3 w-fit text-muted-foreground" onClick={() => onOpenChange(false)}>
          <ArrowLeftIcon />Back
        </Button>
        <div className="relative mb-4">
          <SearchIcon aria-hidden="true" className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
          <Input aria-label={`Search ${title.toLowerCase()}`} placeholder="Search" value={query} onChange={(e) => setQuery(e.target.value)}
            className="h-8 bg-card pl-8" />
        </div>
        {groups.map((g) => (
          <div key={g} className="mb-5 flex flex-col gap-0.5">
            <div className="px-2.5 pb-1.5 font-mono text-xs uppercase tracking-[1.2px] text-muted-foreground">{g}</div>
            {matches.filter((p) => p.group === g).map((p) => (
              <button key={p.id} onClick={() => onPage(p.id)} aria-current={p.id === page ? "page" : undefined}
                className={cn("flex h-9 items-center gap-3 rounded-md px-2.5 text-left text-sm [&_svg]:size-4 [&_svg]:shrink-0",
                  p.id === page ? "bg-accent [&_svg]:text-foreground" : "hover:bg-accent [&_svg]:text-muted-foreground")}>
                {p.icon}{p.label}
              </button>
            ))}
          </div>
        ))}
        {matches.length === 0 && <p className="px-2.5 text-xs text-muted-foreground">Nothing matches “{query}”.</p>}
      </nav>
      <div className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-10 py-8">
          <h2 className="title mb-6 text-2xl">{current?.label}</h2>
          <div>{children}</div>
        </div>
      </div>
    </div>
  );
}

/** A titled group of setting rows. */
export function Section({ title, description, children }: { title?: string; description?: ReactNode; children: ReactNode }) {
  return (
    <section className="mt-8 first:mt-0">
      {title && <h3 className="font-sans text-sm font-semibold">{title}</h3>}
      {description && <p className="mt-0.5 text-muted-foreground">{description}</p>}
      <div className={cn(title || description ? "mt-3" : "", "rounded-lg border bg-card px-4 py-2 [&>*+*]:border-t")}>{children}</div>
    </section>
  );
}

/** Label and detail on the left, the control on the right. */
export function Row({ label, detail, children, stack }: { label: ReactNode; detail?: ReactNode; children?: ReactNode; stack?: boolean }) {
  return (
    <div className={cn("flex gap-x-6 gap-y-2 py-3.5", stack ? "flex-col" : "items-center justify-between")}>
      <div className="min-w-0">
        <div className="text-sm">{label}</div>
        {detail && <div className="mt-0.5 text-muted-foreground">{detail}</div>}
      </div>
      {children && <div className={cn("flex shrink-0 items-center gap-2", stack && "w-full")}>{children}</div>}
    </div>
  );
}

/** A number as a slider; the value is shown on the thumb. `max` grows if the saved value is above it. */
export function SliderRow({ label, detail, value, onChange, min = 0, max, step = 1, format }: {
  label: ReactNode; detail?: ReactNode; value: number; onChange: (v: number) => void;
  min?: number; max: number; step?: number; format?: (v: number) => string;
}) {
  return (
    <Row label={label} detail={detail} stack>
      <Slider aria-label={typeof label === "string" ? label : undefined} min={min} max={Math.max(max, value)} step={step} value={[value]}
        onValueChange={([v]) => onChange(v)} format={format} className="mb-1" />
    </Row>
  );
}

/** Tick which agents may use something (a connection, a secret). */
export function AgentPicker({ agents, value, onChange }: { agents: Agent[]; value: number[]; onChange: (ids: number[]) => void }) {
  if (agents.length === 0) return <p className="text-muted-foreground">No agents yet.</p>;
  return (
    <fieldset className="flex flex-wrap gap-x-5 gap-y-2">
      <legend className="sr-only">Agents that may use it</legend>
      {agents.map((a) => (
        <div key={a.id} className="flex items-center gap-2">
          <Checkbox id={`agent-pick-${a.id}`} checked={value.includes(a.id)}
            onCheckedChange={(c) => onChange(c ? [...value, a.id] : value.filter((i) => i !== a.id))} />
          <Label htmlFor={`agent-pick-${a.id}`} className="font-normal">{a.name}</Label>
        </div>
      ))}
    </fieldset>
  );
}

/** Inline result of a save, in the product's own words. */
export function Status({ msg }: { msg: string }) {
  if (!msg) return null;
  const bad = /error|fail|invalid|denied|must|can't|cannot/i.test(msg);
  return <p role={bad ? "alert" : "status"} className={cn("mt-2", bad ? "text-destructive" : "text-muted-foreground")}>{msg}</p>;
}
