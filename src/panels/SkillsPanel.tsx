import { useEffect, useState } from "react";
import { AgentMatrix } from "../components/AgentMatrix";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { api } from "../lib/api";

export function SkillsPanel() {
  const { items, error, refresh } = useAssets("skill");
  const { agents, refresh: refreshAgents } = useAgents();
  const [selected, setSelected] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [actionError, setActionError] = useState<string | null>(null);

  useEffect(() => {
    if (!selected) {
      setContent("");
      return;
    }
    void api
      .readAsset("skill", selected)
      .then(setContent)
      .catch((e) => setActionError(String(e)));
  }, [selected]);

  return (
    <div className="grid grid-cols-[240px_1fr] gap-6">
      <section>
        <h2 className="mb-3 text-xl font-semibold">Skills</h2>
        {error && <p className="text-sm text-red-600 dark:text-red-400">{error}</p>}
        <ul className="space-y-1">
          {items.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                className="w-full rounded px-2 py-1 text-left hover:bg-slate-100 dark:hover:bg-slate-700"
                onClick={() => setSelected(item.id)}
              >
                {item.name}
              </button>
            </li>
          ))}
        </ul>
        {items.length === 0 && (
          <p className="mt-2 text-sm text-slate-500 dark:text-slate-400">
            还没有 skill。可从 cc-switch 导入，或把目录放进真源的 assets/skills/。
          </p>
        )}
      </section>
      <section>
        {selected ? (
          <>
            {actionError && (
              <p className="mb-2 text-sm text-red-600 dark:text-red-400">{actionError}</p>
            )}
            <pre className="max-h-64 overflow-auto whitespace-pre-wrap rounded border border-slate-300 p-3 text-xs dark:border-slate-600">
              {content}
            </pre>
            <AgentMatrix
              assetType="skill"
              id={selected}
              agents={agents}
              onChanged={() => {
                void refresh();
                void refreshAgents();
              }}
            />
            <p className="mt-2 text-xs text-slate-500 dark:text-slate-400">
              落地方式（symlink / junction / copy）由本机实测决定，见「同步到」的返回提示。
            </p>
          </>
        ) : (
          <p className="text-sm text-slate-500 dark:text-slate-400">选择左侧 skill 查看与分发。</p>
        )}
      </section>
    </div>
  );
}
