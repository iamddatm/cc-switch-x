import { FolderOpen, RefreshCw, Sparkles } from "lucide-react";
import { useProjectSkills } from "../hooks/useProjectSkills";
import { api } from "../lib/api";
import type { BadgeTone } from "../components/ui/Badge";
import { Badge } from "../components/ui/Badge";
import { Button } from "../components/ui/Button";
import { EmptyState } from "../components/ui/EmptyState";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import type { ProjectDirMeta, ProjectDirState, ProjectSkillView, ProjectView } from "../types";

const STATE_LABEL: Record<ProjectDirState, { label: string; tone: BadgeTone }> = {
  present: { label: "有", tone: "success" },
  absent: { label: "无", tone: "neutral" },
  different: { label: "不同", tone: "warning" },
  untracked: { label: "未跟踪", tone: "neutral" },
  unavailable: { label: "不可用", tone: "neutral" },
};

/** 未在真机验证的目录不展示「有 / 无」，避免用户把它当成可信的同步依据。 */
const UNVERIFIED = { label: "待实测", tone: "neutral" as BadgeTone };

const FALLBACK_DIRS = [".claude/skills", ".qoder/skills", ".agents/skills"];

function dirBadge(meta: ProjectDirMeta, skill: ProjectSkillView) {
  if (!meta.verified) {
    return <Badge tone={UNVERIFIED.tone} title="该目录尚未在真机验证，暂不支持同步">{UNVERIFIED.label}</Badge>;
  }
  const view = skill.dirs[meta.key];
  if (!view) return <Badge tone="neutral">无</Badge>;
  const spec = STATE_LABEL[view.state] ?? STATE_LABEL.absent;
  return <Badge tone={spec.tone}>{spec.label}</Badge>;
}

export function ProjectSkillsPanel({ project }: { project: ProjectView }) {
  const { data, loading, error, refresh } = useProjectSkills(project.id);
  const current = data?.project ?? project;
  const dirs = data?.dirs ?? [];
  const skills = data?.skills ?? [];

  return (
    <div className="flex h-full flex-col">
      <PageHeader
        icon={Sparkles}
        title="Skills"
        description={`${project.name} · ${project.path}`}
        actions={
          <>
            <Button
              size="sm"
              onClick={() =>
                void api
                  .revealProjectFolder(project.id)
                  .catch((e) => console.warn("打开项目文件夹失败：", e))
              }
            >
              <FolderOpen size={14} aria-hidden />
              打开文件夹
            </Button>
            <Button size="sm" loading={loading} onClick={() => void refresh()}>
              <RefreshCw size={14} aria-hidden />
              刷新
            </Button>
          </>
        }
      />

      <div className="min-h-0 flex-1 overflow-auto px-6 py-3">
        {!current.exists && (
          <Notice tone="danger" className="mb-3">
            项目路径不存在：{current.path}
          </Notice>
        )}
        {error && (
          <Notice tone="danger" className="mb-3">
            读取项目 Skills 失败：{error}
          </Notice>
        )}

        {skills.length === 0 ? (
          loading ? (
            <p className="px-1 py-6 text-caption text-fg-3">正在读取项目 Skills…</p>
          ) : (
            <EmptyState
              icon={Sparkles}
              title="项目里还没有 Skills"
              description={`会从这些目录读取：${(dirs.length > 0 ? dirs.map((dir) => dir.relativePath) : FALLBACK_DIRS).join("、")}`}
            />
          )
        ) : (
          <div className="min-w-[560px] rounded-panel border border-border bg-surface">
            <div className="flex items-center gap-3 border-b border-border px-3 py-2 text-badge text-fg-3">
              <span className="min-w-0 flex-1">名称</span>
              {dirs.map((dir) => (
                <span
                  key={dir.key}
                  className="w-20 shrink-0 text-center"
                  title={dir.relativePath}
                >
                  {dir.label || dir.relativePath}
                </span>
              ))}
              <span className="w-10 shrink-0 text-center">操作</span>
            </div>
            <ul className="divide-y divide-border">
              {skills.map((skill) => (
                <li key={skill.id} className="flex items-center gap-3 px-3 py-2">
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-body text-fg-1">{skill.name}</span>
                    {skill.description && (
                      <span className="block truncate text-caption text-fg-3">
                        {skill.description}
                      </span>
                    )}
                  </span>
                  {dirs.map((dir) => (
                    <span key={dir.key} className="flex w-20 shrink-0 justify-center">
                      {dirBadge(dir, skill)}
                    </span>
                  ))}
                  <span className="w-10 shrink-0 text-center text-caption text-fg-3">—</span>
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </div>
  );
}
