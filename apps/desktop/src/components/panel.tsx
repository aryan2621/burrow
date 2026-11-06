import type { ReactNode } from "react";
import { ChevronDownIcon } from "lucide-react";
import { cn } from "@/lib/utils";
import { Empty, EmptyDescription, EmptyHeader, EmptyMedia, EmptyTitle } from "@/components/ui/empty";

/** What a list shows before there is anything in it (shadcn Empty). */
export function EmptyState({ icon, title, children, className }: { icon: ReactNode; title: string; children?: ReactNode; className?: string }) {
  return (
    <Empty className={cn("gap-3 rounded-xl border border-dashed p-8 md:p-8", className)}>
      <EmptyHeader>
        <EmptyMedia variant="icon" className="bg-muted text-muted-foreground">{icon}</EmptyMedia>
        <EmptyTitle className="text-sm">{title}</EmptyTitle>
        {children && <EmptyDescription>{children}</EmptyDescription>}
      </EmptyHeader>
    </Empty>
  );
}

export function ErrorNote({ msg, className }: { msg: string; className?: string }) {
  return msg ? <p role="alert" className={cn("rounded-lg bg-destructive/10 px-3 py-2 text-sm text-destructive", className)}>{msg}</p> : null;
}

/** A section that starts closed, with a one-line summary, so long pages show only what matters first. */
export function Disclosure({ title, summary, children }: { title: string; summary?: ReactNode; children: ReactNode }) {
  return (
    <details className="group rounded-lg border bg-card">
      <summary className="flex cursor-pointer list-none items-center justify-between gap-3 px-5 py-4 [&::-webkit-details-marker]:hidden">
        <span className="min-w-0">
          <span className="block font-sans text-sm font-semibold">{title}</span>
          {summary && <span className="block truncate text-xs text-muted-foreground">{summary}</span>}
        </span>
        <ChevronDownIcon className="size-4 shrink-0 text-muted-foreground transition-transform group-open:rotate-180" />
      </summary>
      <div className="space-y-4 border-t px-5 py-5">{children}</div>
    </details>
  );
}

/** What the safety scan flagged after adding an MCP server. */
export function ScanWarnings({ warnings }: { warnings: string[] }) {
  if (warnings.length === 0) return null;
  return (
    <div role="status" className="mt-2 text-warning">
      Added. The scan flagged:
      <ul className="list-disc pl-5">{warnings.map((w) => <li key={w}>{w}</li>)}</ul>
    </div>
  );
}
