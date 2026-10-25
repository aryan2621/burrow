import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { open } from "@tauri-apps/plugin-dialog";

export type Rule = "allow" | "ask" | "deny";
export const TOOLS = ["list_dir", "read_file", "write_file", "delete_file", "shell", "http_fetch",
  "notes_add", "notes_search", "web_search"] as const;
export const DEFAULT_RULES: Record<string, Rule> = {
  list_dir: "allow", read_file: "allow", write_file: "allow", delete_file: "ask", shell: "ask", http_fetch: "ask",
  notes_add: "allow", notes_search: "allow", web_search: "allow",
};

export type Agent = {
  id: number; name: string; role_prompt: string; model_ref: string; folders: string[];
  allowed_domains: string[]; approval_rules: Record<string, Rule>;
};
export type Message = {
  role: "user" | "assistant" | "tool" | "system";
  content: string;
  tool_calls?: { function: { name: string; arguments: string } }[];
};
export type Approval = { approval_id: number; agent_id: number; tool: string; args: Record<string, unknown> };
export type Action = {
  id: number; run_id: number; tool: string; input: Record<string, unknown>; output: string;
  undoable: boolean; decision: string; created_at: number; undone_at: string | null;
};
export type Event = {
  method: string;
  params: { agent_id: number; text?: string; error?: string | null; trigger?: string;
    id?: string; done?: number; total?: number } & Partial<Approval>;
};
export type Run = { id: number; started_at: string; ended_at: string | null; status: string; trigger: string };
export type ModelCall = {
  run_id: number; model: string; prompt_tokens: number | null; completion_tokens: number | null; ms: number; created_at: string;
};
export type Schedule = {
  id: number; agent_id: number; kind: "cron" | "interval" | "file"; spec: string; prompt: string; enabled: boolean;
  next_run_at: number | null; last_run_at: number | null;
};
export type CatalogModel = { id: string; name: string; size: number; min_ram_gb: number; installed: boolean; downloading: boolean };
export type Hardware = { ram_gb: number; cores: number; os: string; arch: string; gpu: string; recommended: string | null };
export type McpServer = { name: string; transport: "stdio" | "http"; command: string[]; url: string | null; secrets: string[] };
export type Secret = { name: string; agent_ids: number[]; hosts: string[]; header: string };
export type Status = { sandbox: string | null; service_installed: boolean; keystore: string; search: string | null };

export const rpc = <T,>(method: string, params: object = {}) => invoke<T>("rpc", { method, params });
export const onEvent = (f: (e: Event) => void) => listen<Event>("burrow", (e) => f(e.payload));

/** SQLite CURRENT_TIMESTAMP strings are UTC without a zone. */
export const sqlTime = (s: string) => new Date(s.replace(" ", "T") + "Z");

export const pickFolder = async () => {
  const r = await open({ directory: true });
  return typeof r === "string" ? r : null;
};

export const ui = {
  input: "w-full rounded border border-stone-300 bg-white px-2 py-1 dark:border-stone-700 dark:bg-stone-800",
  button: "rounded border border-stone-300 px-3 py-1 text-sm hover:bg-stone-100 dark:border-stone-700 dark:hover:bg-stone-800",
  primary: "rounded bg-amber-800 px-4 py-1.5 text-white disabled:opacity-50",
};
