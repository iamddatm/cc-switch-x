import { useCallback, useEffect, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { api } from "../lib/api";
import type { ProjectView } from "../types";

export const PROJECT_ID_STORAGE_KEY = "ccswitchx.projectId";

/** 最近打开的项目排在前面；时间戳是 RFC 3339，可直接按字符串比较。 */
export function byLastOpenedDesc(a: ProjectView, b: ProjectView): number {
  return b.lastOpenedAt.localeCompare(a.lastOpenedAt);
}

export function useProjects() {
  const [projects, setProjects] = useState<ProjectView[]>([]);
  const [activeId, setActiveId] = useState<string | null>(() =>
    localStorage.getItem(PROJECT_ID_STORAGE_KEY),
  );
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setProjects(await api.listProjects());
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // 记住的项目可能已被移除或换了机器，回退到最近打开的一个。
  useEffect(() => {
    if (loading) return;
    if (projects.length === 0) {
      if (activeId !== null) setActiveId(null);
      localStorage.removeItem(PROJECT_ID_STORAGE_KEY);
      return;
    }
    if (projects.some((project) => project.id === activeId)) return;
    const next = [...projects].sort(byLastOpenedDesc)[0].id;
    setActiveId(next);
    localStorage.setItem(PROJECT_ID_STORAGE_KEY, next);
  }, [activeId, loading, projects]);

  const select = useCallback(async (id: string) => {
    setActiveId(id);
    localStorage.setItem(PROJECT_ID_STORAGE_KEY, id);
    try {
      const touched = await api.touchProject(id);
      setProjects((prev) => prev.map((item) => (item.id === id ? touched : item)));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const add = useCallback(async () => {
    const selected = await open({ directory: true, multiple: false });
    if (typeof selected !== "string") return null;
    try {
      const project = await api.addProject(selected);
      setProjects((prev) =>
        prev.some((item) => item.id === project.id)
          ? prev.map((item) => (item.id === project.id ? project : item))
          : [...prev, project],
      );
      setActiveId(project.id);
      localStorage.setItem(PROJECT_ID_STORAGE_KEY, project.id);
      setError(null);
      return project;
    } catch (e) {
      setError(String(e));
      return null;
    }
  }, []);

  const remove = useCallback(async (id: string) => {
    try {
      await api.removeProject(id);
      setProjects((prev) => prev.filter((project) => project.id !== id));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const setAutoSync = useCallback(async (id: string, enabled: boolean) => {
    try {
      const updated = await api.setProjectAutoSync(id, enabled);
      setProjects((prev) => prev.map((item) => (item.id === id ? updated : item)));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const active = projects.find((project) => project.id === activeId) ?? null;

  return {
    projects,
    active,
    activeId,
    loading,
    error,
    refresh,
    add,
    remove,
    select,
    setAutoSync,
  };
}
