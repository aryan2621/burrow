import { useEffect, useState } from "react";
import { Agent, McpServer, rpc } from "./rpc";

/** An agent's MCP servers plus the error/warning state both the Tools and Discover tabs show. */
export function useMcp(agent: Agent) {
  const [list, setList] = useState<McpServer[]>([]);
  const [error, setError] = useState("");
  const [warnings, setWarnings] = useState<string[]>([]);
  const load = () => rpc<McpServer[]>("mcp.list", { agent_id: agent.id }).then(setList, (e) => setError(String(e)));
  useEffect(() => void load(), [agent.id]);
  return { list, load, error, setError, warnings, setWarnings };
}
