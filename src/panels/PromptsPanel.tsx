import { useEffect, useState } from "react";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { api } from "../lib/api";

export function PromptsPanel() {
  const { items, error, refresh } = useAssets("prompt");
  const { agents, refresh: refreshAgents } = useAgents();
  const [selected, setSelected] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [message, setMessage] = useState<string | null>(null);
  const [pending, setPending] = useState<{ promptId: string; agentId: string; path: string } | null>(
    null,
  );

  useEffect(() => {
    if (!selected) {
      setContent("");
      return;
    }
    void api
      .readAsset("prompt", selected)
      .then(setContent)
      .catch((e) => setMessage(String(e)));
  }, [selected]);

  const activate = async (
    promptId: string,
    agentId: string,
    options?: { confirm?: boolean; saveExistingAs?: string },
  ) => {
    try {
      const outcome = await api.activatePrompt(promptId, agentId, options);
      if (outcome.kind === "needsConfirmation") {
        setPending({ promptId, agentId, path: outcome.path });
        setMessage(null);
        return;
      }
      setPending(null);
      setMessage(outcome.kind === "activated" ? `已写入 ${outcome.path}` : "该 agent 已经在使用这条 prompt");
      await refresh();
      await refreshAgents();
    } catch (e) {
      setMessage(String(e));
    }
  };

  const activeBy = (agentId: string) =>
    agents.find((agent) => agent.id === agentId)?.activePrompt;

  return (
    <div className="grid grid-cols-[240px_1fr] gap-6">
      <section>
        <h2 className="mb-3 text-xl font-semibold">Prompts</h2>
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
      </section>
      <section>
        {selected ? (
          <>
            <pre className="max-h-64 overflow-auto whitespace-pre-wrap rounded border border-slate-300 p-3 text-xs dark:border-slate-600">
              {content}
            </pre>
            {message && <p className="mt-2 text-sm">{message}</p>}
            <h3 className="mt-4 mb-2 text-sm font-medium">激活到</h3>
            <ul className="space-y-1 text-sm">
              {agents
                .filter((agent) => agent.verified)
                .map((agent) => (
                  <li key={agent.id} className="flex items-center gap-2">
                    <button
                      type="button"
                      className="rounded border border-slate-300 px-2 py-1 dark:border-slate-600"
                      onClick={() => void activate(selected, agent.id)}
                    >
                      激活到 {agent.displayName}
                    </button>
                    {activeBy(agent.id) === selected && <span>当前激活</span>}
                  </li>
                ))}
            </ul>
            {pending && (
              <div
                role="alertdialog"
                aria-label="目标文件已有内容"
                className="mt-4 rounded border border-amber-400 p-3 text-sm"
              >
                <p>
                  {pending.path} 已有内容，且不是本工具写入的。整文件接管会覆盖它，请先选择处置方式。
                </p>
                <div className="mt-2 flex gap-2">
                  <button
                    type="button"
                    className="rounded border border-slate-300 px-2 py-1 dark:border-slate-600"
                    onClick={() =>
                      void activate(pending.promptId, pending.agentId, {
                        saveExistingAs: `salvaged-${Date.now()}`,
                      })
                    }
                  >
                    把现有内容另存为一条 prompt
                  </button>
                  <button
                    type="button"
                    className="rounded bg-slate-800 px-2 py-1 text-white dark:bg-slate-600"
                    onClick={() =>
                      void activate(pending.promptId, pending.agentId, { confirm: true })
                    }
                  >
                    确认覆盖
                  </button>
                </div>
              </div>
            )}
          </>
        ) : (
          <p className="text-sm text-slate-500 dark:text-slate-400">选择左侧 prompt 查看与激活。</p>
        )}
      </section>
    </div>
  );
}
