import { useEffect, useState } from "react";
import { BrainIcon, PencilIcon, SearchIcon, Trash2Icon } from "lucide-react";
import { Agent, Note, confirmAction, rpc, sqlTime } from "./rpc";
import { Status } from "@/components/settings";
import { EmptyState } from "@/components/panel";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Item, ItemActions, ItemContent, ItemFooter, ItemGroup } from "@/components/ui/item";
import { Textarea } from "@/components/ui/textarea";

/** The agent's notes: search, edit and delete what it remembers. */
export default function Memory({ agent }: { agent: Agent }) {
  const [notes, setNotes] = useState<Note[]>([]);
  const [query, setQuery] = useState("");
  const [editing, setEditing] = useState<{ id: number; text: string } | null>(null);
  const [error, setError] = useState("");
  const load = (q = query) => rpc<Note[]>("notes.list", { agent_id: agent.id, query: q }).then(setNotes, (e) => setError(String(e)));
  useEffect(() => void load(""), []);
  const call = (method: string, p: object) => rpc(method, { agent_id: agent.id, ...p }).then(() => { setEditing(null); load(); }, (e) => setError(String(e)));
  return (
    <>
      <div className="relative">
        <SearchIcon aria-hidden="true" className="pointer-events-none absolute top-1/2 left-2.5 size-4 -translate-y-1/2 text-muted-foreground" />
        <Input aria-label="Search notes" placeholder={`Search ${notes.length} note${notes.length === 1 ? "" : "s"}`} className="pl-8" value={query}
          onChange={(e) => { setQuery(e.target.value); load(e.target.value); }} />
      </div>
      <Status msg={error} />
      <ItemGroup className="mt-4 gap-2">
        {notes.length === 0 && (
          <EmptyState icon={query ? <SearchIcon /> : <BrainIcon />} title={query ? "No matching notes" : "Nothing remembered yet"}>
            {query ? "Try other words." : `${agent.name} saves notes here when it learns something worth keeping.`}
          </EmptyState>
        )}
        {notes.map((n) => (
          <Item key={n.id} variant="outline" size="sm" className="bg-card">
            {editing?.id === n.id ? (
              <div className="w-full space-y-2">
                <Textarea aria-label="Note text" rows={4} value={editing.text} onChange={(e) => setEditing({ id: n.id, text: e.target.value })} />
                <div className="flex gap-2">
                  <Button size="sm" onClick={() => call("notes.update", { note_id: n.id, text: editing.text })}>Save</Button>
                  <Button size="sm" variant="ghost" onClick={() => setEditing(null)}>Cancel</Button>
                </div>
              </div>
            ) : (
              <>
                <ItemContent className="min-w-0">
                  <p className="whitespace-pre-wrap leading-relaxed">{n.text}</p>
                </ItemContent>
                <ItemActions className="self-start opacity-50 transition-opacity group-hover/item:opacity-100 group-focus-within/item:opacity-100">
                  <Button size="icon-sm" variant="ghost" aria-label="Edit note" onClick={() => setEditing({ id: n.id, text: n.text })}><PencilIcon /></Button>
                  <Button size="icon-sm" variant="ghost" aria-label="Delete note" className="hover:text-destructive"
                    onClick={async () => (await confirmAction("Delete this note?")) && call("notes.delete", { note_id: n.id })}><Trash2Icon /></Button>
                </ItemActions>
                <ItemFooter className="text-xs text-muted-foreground">{sqlTime(n.created_at).toLocaleString()}</ItemFooter>
              </>
            )}
          </Item>
        ))}
      </ItemGroup>
    </>
  );
}
