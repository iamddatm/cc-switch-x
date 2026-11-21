import { useEffect, useMemo, useState } from "react";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { Dialog } from "../ui/Dialog";
import { Notice } from "../ui/Notice";
import { api } from "../../lib/api";
import { generatePromptId, parsePrompt, renderPrompt } from "../../lib/canonical";
import { switchable, type AgentStatus, type AssetSummary } from "../../types";

interface Candidate {
  agent: AgentStatus;
  path: string;
  content: string;
  /** 真源里已有的对应条目：导入时更新它，而不是另建一条。 */
  existingId: string | null;
  existingName: string | null;
}

export interface PromptImportDialogProps {
  agents: AgentStatus[];
  existing: AssetSummary[];
  onClose: () => void;
  /** 导入结束（无论是否有失败）后通知面板刷新。 */
  onDone: () => void;
}

/**
 * 把各 Agent 目标文件里已经在用的提示词复制进真源。
 * 只读目标文件；已有同名条目按其原 ID 更新正文，导入后把该 Agent 指向这条提示词。
 */
export function PromptImportDialog({
  agents,
  existing,
  onClose,
  onDone,
}: PromptImportDialogProps) {
  const [candidates, setCandidates] = useState<Candidate[] | null>(null);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);
  const [report, setReport] = useState<{ imported: number; failed: string[] } | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const eligible = useMemo(() => agents.filter((agent) => switchable(agent, "prompt")), [agents]);

  // 只在打开时扫一次：导入后面板刷新会换掉 agents/existing 引用，重扫会把结果清空。
  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const loaded = await Promise.all(
        eligible.map(async (agent) => {
          const target = await api.readPromptTarget(agent.id).catch(() => null);
          if (!target || !target.content.trim()) return null;
          const hit =
            existing.find((item) => item.id === agent.activePrompt) ??
            existing.find((item) => item.name === `${agent.displayName} 提示词`) ??
            null;
          return {
            agent,
            path: target.path,
            content: target.content,
            existingId: hit?.id ?? null,
            existingName: hit?.name ?? null,
          } satisfies Candidate;
        }),
      );
      if (cancelled) return;
      const list = loaded.filter((item): item is Candidate => item !== null);
      setCandidates(list);
      setSelected(new Set(list.map((item) => item.agent.id)));
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  /** 路径只作参考：真正打开的是后端按 agent 注册表解析出的文件。 */
  const openTarget = async (agentId: string) => {
    setOpenError(null);
    try {
      await api.openPromptTarget(agentId);
    } catch (e) {
      setOpenError(String(e));
    }
  };

  const toggle = (agentId: string) => {
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(agentId)) next.delete(agentId);
      else next.add(agentId);
      return next;
    });
  };

  const run = async () => {
    if (!candidates) return;
    setBusy(true);
    const taken = new Set(existing.map((item) => item.id));
    let imported = 0;
    const failed: string[] = [];
    for (const candidate of candidates) {
      if (!selected.has(candidate.agent.id)) continue;
      const name = `${candidate.agent.displayName} 提示词`;
      const id = candidate.existingId ?? generatePromptId(name, taken);
      try {
        let title = name;
        let description = "";
        if (candidate.existingId) {
          // 更新已有条目时保留用户写的名称和说明，只替换正文。
          const current = parsePrompt(await api.readAsset("prompt", candidate.existingId));
          title = current.title || name;
          description = current.description;
        }
        await api.writeAsset("prompt", id, renderPrompt({ title, description, body: candidate.content }));
        taken.add(id);
        // 目标文件里已经是这份内容，activate 只会更新启用矩阵，不会重复写盘。
        await api.activatePrompt(id, candidate.agent.id, { confirm: true });
        imported += 1;
      } catch (e) {
        failed.push(`${candidate.agent.displayName}：${e}`);
      }
    }
    setReport({ imported, failed });
    setBusy(false);
    onDone();
  };

  return (
    <Dialog
      open
      onClose={onClose}
      title="从 Agent 目标文件导入"
      description="把各 Agent 当前目标文件里的提示词复制进真源，目标文件不会被改动。导入后该 Agent 会指向这条提示词。"
      width={640}
      footer={
        <>
          <Button onClick={onClose} disabled={busy}>
            关闭
          </Button>
          {!report && (
            <Button
              variant="primary"
              loading={busy}
              disabled={selected.size === 0}
              onClick={() => void run()}
            >
              导入选中（{selected.size}）
            </Button>
          )}
        </>
      }
    >
      {openError && (
        <Notice tone="danger" className="mb-3">
          {openError}
        </Notice>
      )}
      {candidates === null && <p className="text-body text-fg-3">正在读取各 Agent 的目标文件…</p>}
      {candidates?.length === 0 && (
        <p className="text-body text-fg-3">没有找到可导入的目标文件内容。</p>
      )}
      {candidates && candidates.length > 0 && (
        <ul className="space-y-2">
          {candidates.map((candidate) => (
            <li
              key={candidate.agent.id}
              className="flex gap-3 rounded-control border border-border p-3"
            >
              <Checkbox
                aria-label={`导入 ${candidate.agent.displayName} 的提示词`}
                checked={selected.has(candidate.agent.id)}
                disabled={busy || report !== null}
                onChange={() => toggle(candidate.agent.id)}
              />
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="text-body font-medium text-fg-1">
                    {candidate.agent.displayName}
                  </span>
                  {candidate.existingId && (
                    <Badge tone="warning">将更新「{candidate.existingName}」</Badge>
                  )}
                  <span className="ml-auto text-caption text-fg-3">
                    {candidate.content.length} 字符
                  </span>
                </div>
                <button
                  type="button"
                  className="block max-w-full cursor-pointer truncate text-left font-mono text-caption text-fg-3 hover:text-fg-1 hover:underline"
                  title={`用默认程序打开 ${candidate.path}`}
                  onClick={() => void openTarget(candidate.agent.id)}
                >
                  {candidate.path}
                </button>
                <p className="mt-1 truncate text-caption text-fg-2" title={candidate.content}>
                  {candidate.content.trim()}
                </p>
              </div>
            </li>
          ))}
        </ul>
      )}
      {report && (
        <div className="mt-3">
          <Notice tone={report.failed.length > 0 ? "warning" : "success"}>
            已导入 {report.imported} 条。
            {report.failed.length > 0 && `失败 ${report.failed.length} 条：${report.failed.join("；")}`}
          </Notice>
        </div>
      )}
    </Dialog>
  );
}
