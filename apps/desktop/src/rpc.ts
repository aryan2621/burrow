import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export type Rule = "allow" | "ask" | "deny";
export const TOOLS = ["list_dir", "read_file", "write_file", "delete_file", "shell", "http_fetch"] as const;
export const DEFAULT_RULES: Record<string, Rule> = {
  list_dir: "allow", read_file: "allow", write_file: "allow", delete_file: "ask", shell: "ask", http_fetch: "ask",
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
  params: { agent_id: number; text?: string; error?: string | null } & Partial<Approval>;
};

export const rpc = <T,>(method: string, params: object = {}) => invoke<T>("rpc", { method, params });
export const onEvent = (f: (e: Event) => void) => listen<Event>("burrow", (e) => f(e.payload));
