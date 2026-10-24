import type { AssetSummary, SkillSourceKind } from "../../types";
import { Badge } from "../ui/Badge";

const KIND_LABELS: Record<SkillSourceKind, string> = {
  github: "GitHub 安装",
  zip: "ZIP 安装",
  local: "本地目录",
  "cc-switch": "cc-switch 导入",
};

/** `owner/repo` 形式的来源标签；分支只在悬停提示里出现，避免默认分支刷屏。 */
export function originRepoLabel(origin: AssetSummary["origin"]): string | null {
  if (!origin?.repo) return null;
  return origin.repo;
}

/** 来源徽标：有仓库信息时优先显示 owner/repo，否则显示获取方式。 */
export function SkillSourceBadge({ item }: { item: AssetSummary }) {
  const origin = item.origin;
  if (!origin) {
    return (
      <Badge mono className="max-w-[200px]" title="没有来源元数据">
        {item.source ?? "本地"}
      </Badge>
    );
  }
  const repoLabel = originRepoLabel(origin);
  if (repoLabel) {
    const title = [
      `来源：${KIND_LABELS[origin.kind]}`,
      origin.branch ? `分支 ${origin.branch}` : null,
      origin.sourceUrl,
    ]
      .filter(Boolean)
      .join(" · ");
    return (
      <Badge mono className="max-w-[220px]" title={title}>
        {repoLabel}
      </Badge>
    );
  }
  return (
    <Badge className="max-w-[180px]" title={origin.sourceUrl ?? undefined}>
      {KIND_LABELS[origin.kind]}
    </Badge>
  );
}
