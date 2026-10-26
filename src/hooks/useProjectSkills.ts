import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type { ProjectSkillsView } from "../types";

export function useProjectSkills(projectId: string) {
  const [data, setData] = useState<ProjectSkillsView | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setData(await api.listProjectSkills(projectId));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [projectId]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  return { data, loading, error, refresh };
}
