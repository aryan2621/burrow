import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { ask, open, save } from "@tauri-apps/plugin-dialog";

export type Rule = "allow" | "ask" | "deny";
export const TOOLS = ["list_dir", "read_file", "write_file", "delete_file", "shell", "http_fetch",
  "notes_add", "notes_search", "web_search", "email_search", "email_read", "email_move", "email_draft", "email_send", "delegate"] as const;
export const DEFAULT_RULES: Record<string, Rule> = {
  list_dir: "allow", read_file: "allow", write_file: "allow", delete_file: "ask", shell: "ask", http_fetch: "ask",
  notes_add: "allow", notes_search: "allow", web_search: "allow",
  email_search: "allow", email_read: "allow", email_move: "allow", email_draft: "allow", email_send: "ask", delegate: "deny",
};

export type Agent = {
  id: number; name: string; role_prompt: string; model_ref: string; folders: string[];
  allowed_domains: string[]; approval_rules: Record<string, Rule>; context_budget: number; limits: Limits;
};
export type Limits = {
  max_steps: number; max_run_secs: number; max_run_tokens: number; review: boolean; monthly_usd: number; cpus: number; memory_mb: number;
};
export type Routine = { id: number; name: string; prompt: string; tools: string[]; approved: string[] };
export type Cost = { agent_id: number; total_usd: number; month_usd: number; unpriced_calls: number };
export type Note = { id: number; text: string; created_at: string };
export type Providers = Record<"anthropic" | "openai" | "gemini" | "custom", { configured: boolean; base_url?: string }>;
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
type Event = {
  method: string;
  params: { agent_id: number; text?: string; error?: string | null; trigger?: string;
    id?: string; done?: number; total?: number } & Partial<Approval>;
};
export type Run = { id: number; started_at: string; ended_at: string | null; status: string; trigger: string };
export type ModelCall = {
  run_id: number; model: string; prompt_tokens: number | null; completion_tokens: number | null; ms: number; created_at: string;
  cost_usd: number | null;
};
export type Connection = {
  name: string; kind: "email"; config: Record<string, string | number>; agent_ids: number[];
};
export type Schedule = {
  id: number; agent_id: number; kind: "cron" | "interval" | "file" | "email"; spec: string; prompt: string; enabled: boolean;
  next_run_at: number | null; last_run_at: number | null; routine_id: number | null;
};
export type CatalogModel = { id: string; name: string; size: number; min_ram_gb: number; installed: boolean; downloading: boolean };
export type Hardware = { ram_gb: number; free_disk_gb: number; vram_gb: number | null; cores: number; os: string; arch: string; gpu: string; recommended: string | null };
export type McpServer = { name: string; transport: "stdio" | "http"; command: string[]; url: string | null; secrets: string[] };
export type Plugin = { id: string; name: string; description: string; auth: "none" | "key" | "oauth"; secret: { hint: string } | null };
export type Secret = { name: string; agent_ids: number[]; hosts: string[]; header: string; expires_at?: string | null; suspended?: number[] };
export type Status = { sandbox: string | null; isolation?: "vm" | "container"; service_installed: boolean; keystore: string; search: string | null };

export const rpc = <T,>(method: string, params: object = {}) => invoke<T>("rpc", { method, params });
export const onEvent = (f: (e: Event) => void) => listen<Event>("burrow", (e) => f(e.payload));

/** SQLite CURRENT_TIMESTAMP strings are UTC without a zone. */
export const sqlTime = (s: string) => new Date(s.replace(" ", "T") + "Z");

export const pickFolder = async () => {
  const r = await open({ directory: true });
  return typeof r === "string" ? r : null;
};

export const pickFile = async () => {
  const r = await open({ directory: false });
  return typeof r === "string" ? r : null;
};

export const pickSavePath = (defaultPath: string) => save({ defaultPath });

/** A native yes/no that waits for the answer: window.confirm doesn't block in the Tauri webview. */
export const confirmAction = (message: string, okLabel = "Delete") =>
  ask(message, { title: "Burrow", kind: "warning", okLabel, cancelLabel: "Cancel" });

