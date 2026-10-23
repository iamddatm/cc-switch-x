import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type { AgentStatus } from "../types";

export function useAgents() {
  const [agents, setAgents] = useState<AgentStatus[]>([]);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    try {
      setAgents(await api.listAgents());
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);
  useEffect(() => {
    void refresh();
  }, [refresh]);
  return { agents, enabledAgents: agents.filter((agent) => agent.enabled), error, refresh };
}
