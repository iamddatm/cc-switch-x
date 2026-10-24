import type { AssetSummary, SkillSourceKind } from "../../types";
import { Badge } from "../ui/Badge";

const KIND_LABELS: Record<SkillSourceKind, string> = {
  github: "GitHub 安装",
  zip: "ZIP 安装",
  local: "本地目录",
  "cc-switch": "cc-switch 导入",
};

/** 来源徽标：有仓库信息时优先显示 owner/repo@branch，否则显示获取方式。 */
export function SkillSourceBadge({ item }: { item: AssetSummary }) {
  const origin = item.origin;
  if (!origin) {
    return (
      <Badge mono className="max-w-[200px]" title="没有来源元数据">
        {item.source ?? "本地"}
      </Badge>
    );
  }
  if (origin.repo) {
    const label = `${origin.repo}${origin.branch ? `@${origin.branch}` : ""}`;
    return (
      <Badge
        mono
        className="max-w-[220px]"
        title={origin.sourceUrl ? `来源：${KIND_LABELS[origin.kind]} · ${origin.sourceUrl}` : KIND_LABELS[origin.kind]}
      >
        {label}
      </Badge>
    );
  }
  return (
    <Badge className="max-w-[180px]" title={origin.sourceUrl ?? undefined}>
      {KIND_LABELS[origin.kind]}
    </Badge>
  );
}
