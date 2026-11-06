import { useEffect, useRef, useState } from "react";
import {
  ArrowUpIcon, BrainIcon, CheckIcon, ChevronRightIcon, CopyIcon, FileTextIcon, FolderIcon, GlobeIcon, MailIcon, SearchIcon,
  ShieldAlertIcon, SquareIcon, SquareTerminalIcon, Trash2Icon, Volume2Icon, WrenchIcon,
} from "lucide-react";
import { toast } from "sonner";
import { Agent, Approval, Message, onEvent, rpc } from "./rpc";
import { Avatar, AvatarKind } from "./Avatars";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { Button } from "@/components/ui/button";
import { ErrorNote } from "@/components/panel";
import { TEMPLATES } from "./templates";

const STARTERS = ["What's in my folders? Give me a short overview.", "Research a topic and cite your sources.", "Draft a plan for today from my notes."];

export default function Chat({ agent, avatar, sandbox, onDecided }: { agent: Agent; avatar: AvatarKind; sandbox: string | null; onDecided?: () => void }) {
  const [msgs, setMsgs] = useState<Message[]>([]);
  const [stream, setStream] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [text, setText] = useState("");
  const [approvals, setApprovals] = useState<Approval[]>([]);
  const end = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);

  const load = () => rpc<Message[]>("chat.history", { agent_id: agent.id }).then(setMsgs);
  const loadApprovals = () =>
    rpc<Approval[]>("approvals.pending").then((a) => setApprovals(a.filter((x) => x.agent_id === agent.id)));

  useEffect(() => {
    setStream("");
    setError("");
    load();
    loadApprovals();
    rpc<number[]>("chat.busy").then((b) => setBusy(b.includes(agent.id)));
    const un = onEvent(({ method, params }) => {
      if (params?.agent_id !== agent.id) return;
      if (method === "chat.delta") setStream((s) => s + params.text);
      if (method === "chat.tool") { setStream(""); load(); }
      if (method === "approval.requested") setApprovals((a) => [...a, params as Approval]);
      if (method === "chat.done") {
        setStream("");
        setBusy(false);
        setError(params.error ?? "");
        load();
        loadApprovals();
      }
    });
    return () => void un.then((f) => f());
  }, [agent.id]);

  useEffect(() => end.current?.scrollIntoView({ block: "end" }), [msgs, stream, approvals]);

  const send = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!text.trim() || busy) return;
    setError("");
    try {
      await rpc("chat.send", { agent_id: agent.id, text });
      // the run saves the message itself, so reloading now could miss it; show it until the next reload
      setMsgs((m) => [...m, { role: "user", content: text }]);
      setBusy(true);
      setText("");
    } catch (e) {
      setError(String(e));
    }
  };
  const decide = async (a: Approval, allow: boolean, remember = false) => {
    setApprovals((list) => list.filter((x) => x.approval_id !== a.approval_id));
    await rpc("approvals.decide", { approval_id: a.approval_id, allow, remember }).catch((e) => setError(String(e)));
    onDecided?.();
  };
  const kill = () => rpc("agents.kill", { agent_id: agent.id }).catch((e) => setError(String(e)));
  const starters = TEMPLATES.find((t) => t.role === agent.role_prompt)?.starters ?? STARTERS;
  const empty = msgs.length === 0 && !stream && !busy;

  return (
    <div className="flex h-full flex-col">
      <div className="flex-1 overflow-y-auto">
        <div role="log" aria-label="Conversation" aria-live="polite" aria-busy={!!stream} className="mx-auto max-w-3xl space-y-5 px-6 py-6">
          {empty && (
            <div className="pt-10">
              <h2 className="title text-2xl">What should {agent.name} work on?</h2>
              <p className="mt-2 max-w-prose text-muted-foreground">
                It asks before anything risky, and every change it makes can be undone from Activity.
              </p>
              <ul className="mt-6 flex flex-col items-start gap-1">
                {starters.map((s) => (
                  <li key={s}><Button variant="ghost" size="sm" className="-ml-2.5 text-muted-foreground" onClick={() => { setText(s); input.current?.focus(); }}>{s}</Button></li>
                ))}
              </ul>
            </div>
          )}
          {msgs.map((m, i) => <Bubble key={i} m={m} avatar={avatar} />)}
          {stream && <Bubble m={{ role: "assistant", content: stream }} avatar={avatar} streaming />}
          {approvals.map((a) => (
            <div key={a.approval_id} role="alertdialog" aria-label="Approval needed" className="rounded-lg border border-brand bg-card p-4">
                <p className="flex items-center gap-2 font-medium">
                  <ShieldAlertIcon className="size-4 shrink-0 text-brand" />{ask(agent.name, a)}
                </p>
                <div className="mt-3 ml-6">
                  <Args tool={a.tool} args={a.args} />
                  <div className="mt-3 flex flex-wrap gap-2">
                    <Button size="sm" autoFocus onClick={() => decide(a, true)}>Allow once</Button>
                    <Button size="sm" variant="outline" onClick={() => decide(a, true, true)}>
                      {a.tool === "http_fetch" || a.tool === "net" ? "Always allow this site" : "Always allow"}
                    </Button>
                    <Button size="sm" variant="ghost" className="text-destructive hover:text-destructive" onClick={() => decide(a, false)}>Deny</Button>
                  </div>
                </div>
            </div>
          ))}
          {busy && !stream && approvals.length === 0 && (
            <p role="status" className="flex items-center gap-2 text-muted-foreground">
              <span className="lamp-on size-1.5 rounded-full" />{agent.name} is working
              <span className="flex gap-0.5">{[0, 1, 2].map((i) => <span key={i} className="typing size-1 rounded-full bg-current" style={{ animationDelay: `${i * 160}ms` }} />)}</span>
            </p>
          )}
          <ErrorNote msg={error} />
          <div ref={end} />
        </div>
      </div>
      <div className="mx-auto w-full max-w-3xl px-6 pb-5">
        <form onSubmit={send}
          className="rounded-xl border bg-card p-2 focus-within:border-foreground">
          <textarea ref={input} aria-label="Message" rows={1} value={text} onChange={(e) => setText(e.target.value)}
            onKeyDown={(e) => { if (e.key === "Enter" && !e.shiftKey) { e.preventDefault(); e.currentTarget.form?.requestSubmit(); } }}
            placeholder={`Ask ${agent.name}…`}
            className="block max-h-40 min-h-10 w-full resize-none bg-transparent px-2 py-1.5 text-sm [field-sizing:content] focus-visible:outline-none" />
          <div className="mt-1 flex items-center justify-between gap-3 pl-2">
            <p className="flex min-w-0 items-center gap-x-4 truncate text-xs text-muted-foreground">
              <span className={sandbox ? undefined : "text-destructive"}>{sandbox ? "Sandboxed" : "No sandbox, so shell is off"}</span>
              <span>{agent.allowed_domains.length === 0 ? "No sites allowed"
                : `${agent.allowed_domains.length} allowed site${agent.allowed_domains.length > 1 ? "s" : ""}`}</span>
              <span>Enter to send</span>
            </p>
            {busy ? (
              <Button type="button" variant="secondary" size="icon" onClick={kill} aria-label="Stop" title="Stop this run" className="size-8 shrink-0 rounded-lg">
                <SquareIcon className="fill-current" />
              </Button>
            ) : (
              <Button size="icon" aria-label="Send" disabled={!text.trim()} className="size-8 shrink-0 rounded-lg"><ArrowUpIcon /></Button>
            )}
          </div>
        </form>
      </div>
    </div>
  );
}

export const TOOL: Record<string, { label: string; icon: React.ComponentType<{ className?: string }> }> = {
  shell: { label: "Ran a command", icon: SquareTerminalIcon },
  list_dir: { label: "Listed a folder", icon: FolderIcon },
  read_file: { label: "Read a file", icon: FileTextIcon },
  write_file: { label: "Wrote a file", icon: FileTextIcon },
  delete_file: { label: "Deleted a file", icon: Trash2Icon },
  http_fetch: { label: "Opened a page", icon: GlobeIcon },
  web_search: { label: "Searched the web", icon: SearchIcon },
  notes_add: { label: "Saved a note", icon: BrainIcon },
  notes_search: { label: "Searched notes", icon: BrainIcon },
  email_search: { label: "Searched email", icon: MailIcon },
  email_read: { label: "Read an email", icon: MailIcon },
  email_move: { label: "Moved an email", icon: MailIcon },
  email_draft: { label: "Saved a draft", icon: MailIcon },
  email_send: { label: "Sent an email", icon: MailIcon },
};
const VERB: Record<string, string> = {
  shell: "run a command", write_file: "write a file", delete_file: "delete a file", http_fetch: "open a page", email_send: "send an email",
};
const ask = (name: string, a: Approval) =>
  a.tool === "net" ? `${name}'s sandbox wants to connect to ${a.args.host}` : `${name} wants to ${VERB[a.tool] ?? `use ${a.tool}`}`;

/** The one argument a person cares about (the command, path, URL or query), if there is one. */
const KEY_ARGS = ["command", "path", "url", "query", "to", "subject", "host"];
export const keyArg = (args: Record<string, unknown>) => {
  const k = KEY_ARGS.find((k) => typeof args[k] === "string");
  return k ? String(args[k]) : null;
};
const parse = (s: string): Record<string, unknown> => { try { return JSON.parse(s) ?? {}; } catch { return {}; } };

function Args({ tool, args }: { tool: string; args: Record<string, unknown> }) {
  if (tool === "shell" && typeof args.command === "string")
    return <pre className="overflow-auto whitespace-pre-wrap rounded-md bg-muted px-3 py-2">{args.command}</pre>;
  return <pre className="max-h-40 overflow-auto whitespace-pre-wrap rounded-md bg-muted px-3 py-2">{JSON.stringify(args, null, 2)}</pre>;
}

function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);
  const copy = () =>
    navigator.clipboard.writeText(text).then(
      () => {
        setCopied(true);
        toast.success("Copied to clipboard");
        setTimeout(() => setCopied(false), 1500);
      },
      () => toast.error("Could not copy"),
    );
  return (
    <Button variant="ghost" size="icon" onClick={copy} aria-label={copied ? "Copied" : "Copy reply"} title="Copy"
      className="-ml-1.5 size-6 text-muted-foreground hover:text-foreground">
      {copied ? <CheckIcon className="size-3.5" /> : <CopyIcon className="size-3.5" />}
    </Button>
  );
}

function SpeakButton({ text }: { text: string }) {
  const [speaking, setSpeaking] = useState(false);

  useEffect(() => () => window.speechSynthesis?.cancel(), []);

  const toggle = () => {
    const syn = window.speechSynthesis;
    if (!syn) {
      toast.error("Speech is not available here");
      return;
    }
    if (speaking) {
      syn.cancel();
      setSpeaking(false);
      return;
    }
    syn.cancel();
    const u = new SpeechSynthesisUtterance(text);
    u.onend = () => setSpeaking(false);
    u.onerror = () => setSpeaking(false);
    setSpeaking(true);
    syn.speak(u);
  };

  return (
    <Button variant="ghost" size="icon" onClick={toggle}
      aria-label={speaking ? "Stop speaking" : "Speak reply"} title={speaking ? "Stop" : "Speak"}
      className="size-6 text-muted-foreground hover:text-foreground">
      {speaking ? <SquareIcon className="size-3 fill-current" /> : <Volume2Icon className="size-3.5" />}
    </Button>
  );
}

function Bubble({ m, avatar, streaming }: { m: Message; avatar: AvatarKind; streaming?: boolean }) {
  if (m.role === "tool")
    return (
      <details className="group -mt-3 ml-9">
        <summary className="flex w-fit cursor-pointer list-none items-center gap-1 text-xs text-muted-foreground select-none hover:text-foreground">
          <ChevronRightIcon className="size-3.5 transition-transform group-open:rotate-90" />Output
        </summary>
        <pre className="mt-1 max-h-60 overflow-auto whitespace-pre-wrap rounded-md bg-muted px-3 py-2">{m.content}</pre>
      </details>
    );
  if (m.role === "user")
    return (
      <div className="flex justify-end">
        <p className="w-fit max-w-[80%] whitespace-pre-wrap rounded-xl rounded-br-md bg-primary/10 px-3.5 py-2 leading-relaxed">{m.content}</p>
      </div>
    );
  return (
    <div className="flex gap-3">
      {m.content ? <Avatar kind={avatar} className="size-6" /> : <span className="w-6 shrink-0" />}
      <div className="min-w-0 max-w-[80ch] flex-1 space-y-2">
        {m.content && (
          <div className="md pt-0.5">
            {/* no raw HTML: model output is untrusted */}
            <Markdown remarkPlugins={[remarkGfm]} components={{ a: ({ href, children }) => <a href={href} target="_blank" rel="noreferrer">{children}</a> }}>
              {m.content}
            </Markdown>
          </div>
        )}
        {m.content && !streaming && (
          <div className="flex items-center">
            <CopyButton text={m.content} />
            <SpeakButton text={m.content} />
          </div>
        )}
        {m.tool_calls?.map((c, i) => {
          const t = TOOL[c.function.name] ?? { label: c.function.name, icon: WrenchIcon };
          const arg = keyArg(parse(c.function.arguments));
          return (
            <p key={i} className="flex max-w-full items-center gap-2 text-muted-foreground">
              <t.icon className="size-4 shrink-0" />
              <span className="shrink-0">{t.label}</span>
              <code className="truncate" title={c.function.arguments}>{arg ?? c.function.arguments}</code>
            </p>
          );
        })}
      </div>
    </div>
  );
}
