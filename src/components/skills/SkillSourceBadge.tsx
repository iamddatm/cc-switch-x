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

/**
 * 已安装 Skill 可在浏览器打开的来源地址。
 *
 * 优先 SKILL.md 的 blob 链接（最精确），其次仓库页；ZIP / 本地目录的来源是路径，不在此列。
 * 只认 GitHub 的 https 链接，与后端 `open_external_url` 的校验保持一致。
 */
export function skillSourceUrl(
  origin: AssetSummary["origin"] | null | undefined,
): string | null {
  if (!origin) return null;
  for (const candidate of [origin.readmeUrl, origin.sourceUrl]) {
    if (isOpenableGithubUrl(candidate)) return candidate;
  }
  if (origin.repo) return `https://github.com/${origin.repo}`;
  return null;
}

function isOpenableGithubUrl(url: string | null | undefined): url is string {
  if (!url) return false;
  try {
    const parsed = new URL(url);
    return (
      parsed.protocol === "https:" &&
      (parsed.hostname === "github.com" || parsed.hostname === "www.github.com")
    );
  } catch {
    return false;
  }
}

/** 来源徽标：有仓库信息时优先显示 owner/repo，否则显示获取方式。 */
export function SkillSourceBadge({
  item,
  onOpen,
}: {
  item: AssetSummary;
  /** 提供时徽标变成按钮，点击在默认浏览器打开来源。 */
  onOpen?: () => void;
}) {
  const origin = item.origin;
  if (!origin) {
    return (
      <Badge mono className="max-w-[200px]" title="没有来源元数据">
        {item.source ?? "本地"}
      </Badge>
    );
  }
  const repoLabel = originRepoLabel(origin);
  const title = [
    `来源：${KIND_LABELS[origin.kind]}`,
    origin.branch ? `分支 ${origin.branch}` : null,
    origin.sourceUrl,
  ]
    .filter(Boolean)
    .join(" · ");
  const hint = onOpen ? `在浏览器打开来源 · ${title}` : title;

  const badge = repoLabel ? (
    <Badge mono className="max-w-[220px]" title={hint}>
      {repoLabel}
    </Badge>
  ) : (
    <Badge className="max-w-[180px]" title={onOpen ? hint : (origin.sourceUrl ?? undefined)}>
      {KIND_LABELS[origin.kind]}
    </Badge>
  );

  if (!onOpen) return badge;

  return (
    <button
      type="button"
      aria-label={`在浏览器打开 ${repoLabel ?? item.name} 的来源`}
      title={hint}
      onClick={(event) => {
        // 整行是透明的「查看」按钮，这次点击只该打开来源，不应再冒泡成打开详情。
        event.stopPropagation();
        onOpen();
      }}
      className="pointer-events-auto rounded-full transition-opacity duration-150 hover:opacity-80 focus-visible:outline-offset-2"
    >
      {badge}
    </button>
  );
}
