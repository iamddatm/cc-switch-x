import { useState } from "react";
import { GripVertical, Layers } from "lucide-react";
import { AgentGlyph } from "../components/agents/AgentGlyph";
import { Badge } from "../components/ui/Badge";
import { FeedbackBar } from "../components/ui/FeedbackBar";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { useAgents } from "../hooks/useAgents";
import { api } from "../lib/api";
import { cn } from "../lib/cn";
import type { AgentStatus, AssetKind } from "../types";

const CAPABILITIES: { kind: AssetKind; label: string }[] = [
  { kind: "mcp", label: "MCP" },
  { kind: "skill", label: "Skills" },
  { kind: "prompt", label: "提示词" },
  { kind: "rule", label: "规则" },
  { kind: "subagent", label: "子代理" },
  { kind: "hook", label: "Hooks" },
];

export function AgentsPanel() {
  const { agents, error, refresh } = useAgents();
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  /** 拖动结束后把整张顺序表写回设置；显示顺序由后端统一排序，其他面板的列也一起跟着变。 */
  const reorder = async (fromId: string, toId: string) => {
    if (fromId === toId) return;
    const ids = agents.map((item) => item.id);
    const from = ids.indexOf(fromId);
    const to = ids.indexOf(toId);
    if (from < 0 || to < 0) return;
    ids.splice(to, 0, ...ids.splice(from, 1));
    setBusy(fromId);
    setActionError(null);
    setMessage(null);
    try {
      await api.setAgentOrder(ids);
      await refresh();
      setMessage("已调整 Agent 顺序");
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const toggle = async (agent: AgentStatus, next: boolean) => {
    setBusy(agent.id);
    setActionError(null);
    setMessage(null);
    try {
      const nextIds = agents
        .filter((item) => (item.id === agent.id ? next : item.enabled))
        .map((item) => item.id);
      await api.setEnabledAgents(nextIds);
      await refresh();
      setMessage(
        next
          ? `已启用 ${agent.displayName}`
          : `已停用 ${agent.displayName}；已部署的文件不会被删除`,
      );
    } catch (e) {
      setActionError(String(e));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader icon={Layers} title="Agents" />
      {(error || actionError || message) && (
        <FeedbackBar>
          {(error || actionError) && <Notice tone="danger">{error ?? actionError}</Notice>}
          {message && <Notice tone="success">{message}</Notice>}
        </FeedbackBar>
      )}

      <div className="min-h-0 flex-1 overflow-auto px-6 py-5">
        <div className="space-y-3">
          <div className="overflow-hidden rounded-panel border border-border bg-surface">
            <ul>
              {agents.map((agent) => (
                <li
                  key={agent.id}
                  draggable
                  onDragStart={(event) => {
                    event.dataTransfer.effectAllowed = "move";
                    event.dataTransfer.setData("text/plain", agent.id);
                    setDragging(agent.id);
                  }}
                  onDragOver={(event) => {
                    if (!dragging) return;
                    event.preventDefault();
                    event.dataTransfer.dropEffect = "move";
                    if (dropTarget !== agent.id) setDropTarget(agent.id);
                  }}
                  onDragLeave={() =>
                    setDropTarget((current) => (current === agent.id ? null : current))
                  }
                  onDrop={(event) => {
                    event.preventDefault();
                    // 只认本列表内部拖过来的行，避免外部拖入的文本被当成排序
                    const fromId = dragging ?? event.dataTransfer.getData("text/plain");
                    setDragging(null);
                    setDropTarget(null);
                    if (dragging) void reorder(fromId, agent.id);
                  }}
                  onDragEnd={() => {
                    setDragging(null);
                    setDropTarget(null);
                  }}
                  className={cn(
                    "flex items-start gap-3 border-t border-border px-4 py-3 first:border-t-0",
                    dragging === agent.id && "opacity-40",
                    dropTarget === agent.id && dragging !== agent.id && "bg-subtle",
                  )}
                >
                  <span
                    title="拖动调整顺序"
                    className="mt-1 shrink-0 cursor-grab text-fg-3 active:cursor-grabbing"
                  >
                    <GripVertical size={16} aria-hidden />
                  </span>
                  <AgentGlyph agentId={agent.id} label={agent.displayName} size={26} />
                  <div className="min-w-0 flex-1 space-y-1.5">
                    <div className="flex flex-wrap items-center gap-2">
                      <span className="text-strong text-fg-1">{agent.displayName}</span>
                      <span className="font-mono text-caption text-fg-3">{agent.id}</span>
                      <span
                        title={`在 PATH 与常见安装目录中${
                          agent.cliAvailable ? "找到" : "未找到"
                        } ${agent.cliProgram}（仅探测命令是否存在）`}
                      >
                        <Badge tone={agent.cliAvailable ? "success" : "warning"}>
                          {agent.cliAvailable ? "CLI 已安装" : "CLI 未安装"}
                        </Badge>
                      </span>
                      <Badge tone={agent.detected ? "neutral" : "warning"}>
                        {agent.detected ? "配置目录已就绪" : "配置目录不存在"}
                      </Badge>
                    </div>
                    <div className="flex flex-wrap gap-1">
                      {CAPABILITIES.filter((capability) =>
                        agent.capabilities.some(
                          (item) => item.kind === capability.kind && item.supported,
                        ),
                      ).map((capability) => {
                        const status = agent.capabilities.find(
                          (item) => item.kind === capability.kind,
                        );
                        return (
                          <Badge
                            key={capability.kind}
                            tone={status?.verified ? "neutral" : "warning"}
                            className="h-[20px]"
                          >
                            {capability.label}
                            {!status?.verified && " · 未实测"}
                          </Badge>
                        );
                      })}
                    </div>
                    <div className="space-y-0.5">
                      <PathLine label="MCP" path={agent.mcpPath} />
                      <PathLine label="提示词" path={agent.promptPath} />
                      <PathLine label="Skills" path={agent.skillsDir} />
                    </div>
                  </div>
                  <label className="flex shrink-0 items-center gap-2 pt-1 text-caption text-fg-2">
                    <input
                      type="checkbox"
                      aria-label={`启用 ${agent.displayName}`}
                      checked={agent.enabled}
                      disabled={busy === agent.id}
                      onChange={(event) => void toggle(agent, event.target.checked)}
                      className="h-4 w-4 accent-action"
                    />
                    启用
                  </label>
                </li>
              ))}
            </ul>
          </div>

          {agents.length === 0 && !error && (
            <p className="py-8 text-center text-caption text-fg-3">正在读取 Agent 列表…</p>
          )}

          <Notice tone="info">
            启用范围只影响本应用里的显示与分发入口；已部署到 agent 的文件不会被删除。
            拖动左侧手柄可调整顺序，顺序会同步到各资产面板的 agent 列。
            配置目录不存在的 agent 仍可预先启用；CLI 状态只表示 PATH 与常见安装目录里能否找到命令。
          </Notice>
        </div>
      </div>
    </div>
  );
}

function PathLine({ label, path }: { label: string; path: string }) {
  return (
    <div className="flex min-w-0 items-center gap-2">
      <span className="w-12 shrink-0 text-badge text-fg-3">{label}</span>
      <code className="truncate font-mono text-caption text-fg-3" title={path}>
        {path}
      </code>
    </div>
  );
}