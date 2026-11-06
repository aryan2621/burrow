import { useState, type RefObject } from "react";
import { FolderIcon, FolderOpenIcon, XIcon } from "lucide-react";
import { pickFolder } from "./rpc";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

type FoldersChange = (next: string[] | ((prev: string[]) => string[])) => void;

/** Host folders an agent may read or write via file tools (and read-only in the sandbox). */
export default function FoldersField({ folders, onChange, flush }: { folders: string[]; onChange: FoldersChange; flush?: RefObject<(() => string[]) | null> }) {
  const [draft, setDraft] = useState("");
  // lets the parent's Save include a path that was typed but not yet added
  if (flush) flush.current = () => { const d = draft.trim(); setDraft(""); return d ? [...new Set([...folders, d])] : folders; };
  const setUnique = (list: string[]) => onChange([...new Set(list)]);
  const add = (path: string) => {
    const t = path.trim();
    if (t) onChange((prev) => [...new Set([...prev, t])]);
  };
  const commitDraft = () => {
    if (draft.trim()) {
      add(draft);
      setDraft("");
    }
  };

  return (
    <div className="space-y-3">
      {folders.length > 0 && (
        <ul className="flex flex-wrap gap-2">
          {folders.map((p) => (
            <li key={p} className="inline-flex max-w-full items-center gap-1.5 rounded-lg border bg-secondary py-1 pr-1 pl-2.5 font-mono text-xs">
              <FolderIcon className="size-3.5 shrink-0 text-muted-foreground" />
              <span className="truncate">{p}</span>
              <button type="button" aria-label={`Remove ${p}`} onClick={() => setUnique(folders.filter((x) => x !== p))}
                className="grid size-6 place-items-center rounded hover:bg-accent"><XIcon className="size-3.5" /></button>
            </li>
          ))}
        </ul>
      )}
      <div className="flex gap-2">
        <Input aria-label="Add a folder path" placeholder="/Users/me/Documents/notes" value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); commitDraft(); } }}
          onBlur={commitDraft} />
        <Button type="button" variant="outline" onClick={async () => { const p = await pickFolder(); if (p) add(p); }}>
          <FolderOpenIcon />Choose…
        </Button>
      </div>
    </div>
  );
}
