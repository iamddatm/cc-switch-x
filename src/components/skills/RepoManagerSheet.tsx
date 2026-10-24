import { useState } from "react";
import { Plus, Trash2 } from "lucide-react";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { Field } from "../ui/Field";
import { Input } from "../ui/Input";
import { Notice } from "../ui/Notice";
import { Sheet } from "../ui/Sheet";
import { api } from "../../lib/api";
import type { DiscoverableSkill, SkillRepo } from "../../types";

/** 接受 owner/repo 与 https://github.com/owner/repo[.git]。 */
export function parseRepoUrl(input: string): { owner: string; name: string } | null {
  let cleaned = input.trim();
  cleaned = cleaned.replace(/^https?:\/\/github\.com\//i, "");
  cleaned = cleaned.replace(/\.git$/i, "").replace(/\/+$/, "");
  const parts = cleaned.split("/");
  if (parts.length !== 2 || !parts[0] || !parts[1]) return null;
  return { owner: parts[0], name: parts[1] };
}

function repoKey(repo: SkillRepo): string {
  return `${repo.owner}/${repo.name}`.toLowerCase();
}

/** GitHub 坐标大小写不敏感，但展示时保留用户输入的原始大小写。 */
function repoLabel(repo: SkillRepo): string {
  return `${repo.owner}/${repo.name}`;
}

export function RepoManagerSheet({
  repos,
  skills,
  onSaved,
  onClose,
}: {
  repos: SkillRepo[];
  skills: DiscoverableSkill[];
  onSaved: (repos: SkillRepo[]) => void;
  onClose: () => void;
}) {
  const [url, setUrl] = useState("");
  const [branch, setBranch] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);

  const persist = async (next: SkillRepo[]) => {
    setSaving(true);
    setError(null);
    try {
      const saved = await api.setSkillRepos(next);
      onSaved(saved);
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const add = () => {
    const parsed = parseRepoUrl(url);
    if (!parsed) {
      setError("仓库格式应为 owner/repo 或 https://github.com/owner/repo");
      return;
    }
    const key = `${parsed.owner}/${parsed.name}`.toLowerCase();
    if (repos.some((repo) => repoKey(repo) === key)) {
      setError("该仓库已在列表里");
      return;
    }
    void persist([
      ...repos,
      {
        owner: parsed.owner,
        name: parsed.name,
        branch: branch.trim() || "main",
        enabled: true,
      },
    ]).then(() => {
      setUrl("");
      setBranch("");
    });
  };

  const toggle = (repo: SkillRepo, enabled: boolean) => {
    void persist(
      repos.map((item) => (repoKey(item) === repoKey(repo) ? { ...item, enabled } : item)),
    );
  };

  const remove = (repo: SkillRepo) => {
    void persist(repos.filter((item) => repoKey(item) !== repoKey(repo)));
  };

  const skillCount = (repo: SkillRepo) =>
    skills.filter(
      (skill) => skill.repo.toLowerCase() === repoKey(repo),
    ).length;

  return (
    <Sheet
      title="管理 Skill 仓库"
      onBack={onClose}
      backLabel="返回发现"
      footer={
        <Button variant="primary" onClick={onClose}>
          完成
        </Button>
      }
    >
      <div className="mx-auto flex max-w-2xl flex-col gap-5">
        <section className="space-y-3 rounded-panel border border-border bg-surface p-4">
          <h3 className="text-strong text-fg-1">添加仓库</h3>
          <div className="grid grid-cols-[1fr_160px] gap-3">
            <Field label="仓库地址" htmlFor="skill-repo-url">
              <Input
                id="skill-repo-url"
                value={url}
                placeholder="owner/repo 或 https://github.com/owner/repo"
                onChange={(event) => {
                  setUrl(event.target.value);
                  setError(null);
                }}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !saving) add();
                }}
              />
            </Field>
            <Field label="分支（可选）" htmlFor="skill-repo-branch" hint="留空按 main">
              <Input
                id="skill-repo-branch"
                value={branch}
                placeholder="main"
                onChange={(event) => setBranch(event.target.value)}
                onKeyDown={(event) => {
                  if (event.key === "Enter" && !saving) add();
                }}
              />
            </Field>
          </div>
          <Button onClick={add} disabled={saving || !url.trim()}>
            <Plus size={14} aria-hidden />
            添加仓库
          </Button>
        </section>

        {error && <Notice tone="danger">{error}</Notice>}

        <section className="space-y-2">
          <div className="flex items-center justify-between">
            <h3 className="text-strong text-fg-1">已配置仓库</h3>
            <span className="text-caption text-fg-3">{repos.length} 个</span>
          </div>
          {repos.length === 0 ? (
            <p className="rounded-panel border border-dashed border-border px-4 py-6 text-center text-caption text-fg-3">
              还没有仓库。添加一个包含 SKILL.md 的 GitHub 仓库开始发现。
            </p>
          ) : (
            <ul className="divide-y divide-border overflow-hidden rounded-panel border border-border bg-surface">
              {repos.map((repo) => (
                <li key={repoKey(repo)} className="flex items-center gap-3 px-3 py-2.5">
                  <Checkbox
                    aria-label={`${repo.enabled ? "停用" : "启用"} ${repoLabel(repo)}`}
                    checked={repo.enabled}
                    onChange={(event) => toggle(repo, event.target.checked)}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate font-mono text-body text-fg-1">
                        {repoLabel(repo)}
                      </span>
                      <span className="text-caption text-fg-3">@{repo.branch || "main"}</span>
                    </div>
                    <p className="text-caption text-fg-3">
                      {repo.enabled ? `已启用 · 发现 ${skillCount(repo)} 个` : "已停用"}
                    </p>
                  </div>
                  <Button
                    variant="ghost"
                    size="sm"
                    aria-label={`删除仓库 ${repoLabel(repo)}`}
                    disabled={saving}
                    onClick={() => remove(repo)}
                  >
                    <Trash2 size={14} aria-hidden />
                  </Button>
                </li>
              ))}
            </ul>
          )}
          <p className="text-caption text-fg-3">
            停用的仓库不参与发现；删除只影响发现列表，不会卸载已安装的 Skill。
          </p>
        </section>
      </div>
    </Sheet>
  );
}
