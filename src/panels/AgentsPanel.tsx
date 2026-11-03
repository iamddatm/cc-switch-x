import { useState } from "react";
import { GripVertical, Layers, RefreshCw } from "lucide-react";
import { AgentGlyph } from "../components/agents/AgentGlyph";
import { CopyCommandMenu } from "../components/agents/CopyCommandMenu";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { FeedbackBar } from "../components/ui/FeedbackBar";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { useAgents } from "../hooks/useAgents";
import { useAgentVersionCheck } from "../hooks/useAgentVersionCheck";
import { api } from "../lib/api";
import { formatRelativeTime } from "../lib/format";
import { cn } from "../lib/cn";
import type { AgentStatus, AgentVersionStatus, AssetKind } from "../types";

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
  const versionCheck = useAgentVersionCheck();
  const [busy, setBusy] = useState<string | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [dragging, setDragging] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<string | null>(null);

  const versions = new Map(versionCheck.report?.agents.map((item) => [item.id, item]) ?? []);
  const outdated = (versionCheck.report?.agents ?? []).filter((item) => item.updateAvailable);

  const reportResult = (text: string, failed: boolean) => {
    if (failed) {
      setActionError(text);
      setMessage(null);
      return;
    }
    setActionError(null);
    setMessage(text);
  };

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
      <PageHeader
        icon={Layers}
        title="Agents"
        actions={
          <>
            {versionCheck.report && (
              <span className="text-caption text-fg-3">
                检查于 {formatRelativeTime(versionCheck.report.checkedAt)}
              </span>
            )}
            <Button
              onClick={() => void versionCheck.check()}
              loading={versionCheck.checking}
              disabled={agents.length === 0}
              title="跑一遍各 agent 的 --version 并查 npm 上游；只读，不会安装或升级任何东西"
            >
              <RefreshCw size={14} aria-hidden />
              检查更新
            </Button>
          </>
        }
      />
      {(error || actionError || versionCheck.error || message || outdated.length > 0) && (
        <FeedbackBar>
          {(error || actionError || versionCheck.error) && (
            <Notice tone="danger">{error ?? actionError ?? versionCheck.error}</Notice>
          )}
          {outdated.length > 0 && (
            <Notice tone="warning">
              {outdated
                .map((item) => `${item.displayName} ${item.installedVersion} → ${item.latestVersion}`)
                .join("；")}
              。本应用不代跑安装命令，点行右侧的复制图标拿去终端执行。
            </Notice>
          )}
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
                    "flex items-start gap-3 border-t border-border px-4 py-3 transition-colors duration-150 first:border-t-0 hover:bg-subtle",
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
                      <VersionBadge version={versions.get(agent.id)} />
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
                  <span className="flex shrink-0 items-center gap-1 pt-1">
                    <CopyCommandMenu agentId={agent.id} onResult={reportResult} />
                    <label className="flex items-center gap-2 text-caption text-fg-2">
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
                  </span>
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
            「检查更新」只读版本、不装也不升级，安装命令由你复制到终端执行；
            版本只认 PATH 上第一份命令，nvm 与 WSL 里的安装探测不到，Qoder 系没有 npm 上游。
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

/**
 * 版本徽标：只有点过「检查更新」、这一轮里有该 agent 的账时才出现。
 * 读不出版本要分清「没找到命令」和「找到了但 --version 没吐出可解析的版本」——
 * 后者是探测失败，不能跟前者一起写成「未安装」。
 */
function VersionBadge({ version }: { version?: AgentVersionStatus }) {
  if (!version) return null;
  const installed = version.installedVersion ?? (version.cliFound ? "版本读不出" : "未找到命令");
  const title = [
    `本机：${version.installedVersion ?? "读不出"}`,
    `上游：${version.latestVersion ?? "未知"}`,
    version.error ? `这一路查询失败：${version.error}` : null,
    "只探测，不代装",
  ]
    .filter(Boolean)
    .join(" · ");
  return (
    <Badge mono tone={version.updateAvailable ? "warning" : "neutral"} title={title}>
      {version.updateAvailable && version.latestVersion
        ? `${installed} → ${version.latestVersion}`
        : installed}
    </Badge>
  );
}