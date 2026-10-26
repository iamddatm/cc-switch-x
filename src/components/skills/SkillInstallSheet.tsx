import { useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { FileArchive, FolderOpen, GitBranch } from "lucide-react";
import type { SkillScope } from "./skillScope";
import type {
  ConflictPolicy,
  SkillCandidate,
  SkillInstallReport,
  SkillSourceInput,
  SkillSourcePreview,
} from "../../types";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { Field } from "../ui/Field";
import { Input } from "../ui/Input";
import { Notice } from "../ui/Notice";
import { SegmentedControl } from "../ui/SegmentedControl";
import { Select } from "../ui/Select";
import { Sheet } from "../ui/Sheet";

type SourceKind = "github" | "zip" | "directory";

const STATUS_LABEL: Record<string, string> = {
  installed: "已安装",
  overwritten: "已覆盖",
  skipped: "已跳过",
  failed: "失败",
};

export function SkillInstallSheet({
  scope,
  onClose,
  onInstalled,
}: {
  scope: SkillScope;
  onClose: () => void;
  onInstalled: () => void;
}) {
  const [kind, setKind] = useState<SourceKind>("github");
  const [repo, setRepo] = useState("");
  const [branch, setBranch] = useState("");
  const [zipPath, setZipPath] = useState("");
  const [dirPath, setDirPath] = useState("");
  const [preview, setPreview] = useState<SkillSourcePreview | null>(null);
  const [chosen, setChosen] = useState<Set<string>>(new Set());
  const [policy, setPolicy] = useState<ConflictPolicy>("skip");
  const [busy, setBusy] = useState<"scan" | "install" | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [report, setReport] = useState<SkillInstallReport | null>(null);

  const resetPreview = () => {
    setPreview(null);
    setReport(null);
    setError(null);
    setChosen(new Set());
  };

  const changeKind = (value: SourceKind) => {
    setKind(value);
    resetPreview();
  };

  const buildInput = (): SkillSourceInput | null => {
    if (kind === "github") {
      if (!repo.trim()) return null;
      return { kind: "github", repo: repo.trim(), branch: branch.trim() || null };
    }
    if (kind === "zip") {
      return zipPath ? { kind: "zip", path: zipPath } : null;
    }
    return dirPath ? { kind: "directory", path: dirPath } : null;
  };

  const pickZip = async () => {
    const selected = await open({
      multiple: false,
      directory: false,
      title: "选择 Skill ZIP 文件",
      filters: [{ name: "ZIP", extensions: ["zip"] }],
    });
    if (typeof selected === "string") {
      setZipPath(selected);
      resetPreview();
    }
  };

  const pickDirectory = async () => {
    const selected = await open({
      multiple: false,
      directory: true,
      title: "选择包含 SKILL.md 的目录",
    });
    if (typeof selected === "string") {
      setDirPath(selected);
      resetPreview();
    }
  };

  const scan = async () => {
    const input = buildInput();
    if (!input) {
      setError("请先填写仓库或选择文件/目录");
      return;
    }
    setBusy("scan");
    setError(null);
    setReport(null);
    setPreview(null);
    try {
      const result = await scope.previewSource(input);
      setPreview(result);
      setChosen(new Set(result.candidates.map((candidate) => candidate.relativePath)));
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const install = async () => {
    const input = buildInput();
    if (!input || !preview) return;
    const selections = preview.candidates
      .filter((candidate) => chosen.has(candidate.relativePath))
      .map((candidate) => ({
        relativePath: candidate.relativePath,
        id: candidate.suggestedId,
      }));
    if (selections.length === 0) {
      setError("至少选择一个候选 Skill");
      return;
    }
    setBusy("install");
    setError(null);
    try {
      const outcomes = await scope.installSource(
        input,
        preview.stagingId ?? null,
        selections,
        policy,
      );
      setReport({ outcomes });
      setPreview(null);
      onInstalled();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
    }
  };

  const toggleCandidate = (candidate: SkillCandidate, checked: boolean) => {
    setChosen((previous) => {
      const next = new Set(previous);
      if (checked) next.add(candidate.relativePath);
      else next.delete(candidate.relativePath);
      return next;
    });
  };

  const footer = report ? (
    <Button variant="primary" onClick={onClose}>
      完成
    </Button>
  ) : (
    <>
      <Button onClick={onClose} disabled={busy !== null}>
        取消
      </Button>
      {preview && (
        <Button onClick={() => void scan()} disabled={busy !== null}>
          重新扫描
        </Button>
      )}
      <Button
        variant="primary"
        loading={busy !== null}
        disabled={!preview && !buildInput()}
        onClick={() => void (preview ? install() : scan())}
      >
        {preview ? `安装选中 (${chosen.size})` : "扫描来源"}
      </Button>
    </>
  );

  return (
    <Sheet title="从来源安装 Skill" onBack={onClose} backLabel="返回 Skills" footer={footer}>
      <div className="mx-auto flex max-w-2xl flex-col gap-4">
        <SegmentedControl
          ariaLabel="Skill 来源类型"
          value={kind}
          onValueChange={changeKind}
          items={[
            { value: "github", label: "GitHub 仓库" },
            { value: "zip", label: "ZIP 文件" },
            { value: "directory", label: "本地目录" },
          ]}
        />

        {kind === "github" && (
          <div className="grid grid-cols-[1fr_180px] gap-3">
            <Field label="仓库" htmlFor="skill-repo" hint="owner/repo 或 https://github.com/owner/repo">
              <Input
                id="skill-repo"
                value={repo}
                placeholder="acme/skills"
                onChange={(event) => {
                  setRepo(event.target.value);
                  resetPreview();
                }}
              />
            </Field>
            <Field label="分支（可选）" htmlFor="skill-branch" hint="留空依次尝试 main / master">
              <Input
                id="skill-branch"
                value={branch}
                placeholder="main"
                onChange={(event) => {
                  setBranch(event.target.value);
                  resetPreview();
                }}
              />
            </Field>
          </div>
        )}

        {kind === "zip" && (
          <Field label="ZIP 文件" htmlFor="skill-zip">
            <div className="flex gap-2">
              <Input id="skill-zip" value={zipPath} readOnly placeholder="尚未选择" />
              <Button onClick={() => void pickZip()} disabled={busy !== null}>
                <FileArchive size={14} aria-hidden />
                选择 ZIP
              </Button>
            </div>
          </Field>
        )}

        {kind === "directory" && (
          <Field label="本地目录" htmlFor="skill-dir">
            <div className="flex gap-2">
              <Input id="skill-dir" value={dirPath} readOnly placeholder="尚未选择" />
              <Button onClick={() => void pickDirectory()} disabled={busy !== null}>
                <FolderOpen size={14} aria-hidden />
                选择目录
              </Button>
            </div>
          </Field>
        )}

        <div className="flex items-center gap-2 text-caption text-fg-3">
          {kind === "github" ? <GitBranch size={14} aria-hidden /> : null}
          只支持公共仓库；下载走系统代理，单仓库上限 512 MiB。{scope.installHint}
        </div>

        {error && <Notice tone="danger">{error}</Notice>}
        {preview?.warnings.map((warning) => (
          <Notice key={warning} tone="warning">
            {warning}
          </Notice>
        ))}

        {preview && (
          <div className="space-y-2">
            <div className="flex items-center justify-between">
              <span className="text-caption font-medium text-fg-2">
                找到 {preview.candidates.length} 个候选
                {preview.repo ? ` · ${preview.repo}` : ""}
                {preview.branch ? `@${preview.branch}` : ""}
              </span>
              <label className="flex items-center gap-2 text-caption text-fg-2">
                冲突处理
                <Select
                  aria-label="安装冲突处理"
                  value={policy}
                  onChange={(event) => setPolicy(event.target.value as ConflictPolicy)}
                >
                  <option value="skip">跳过已存在的</option>
                  <option value="overwrite">覆盖已存在的（先备份）</option>
                  <option value="keepBoth">两条都留（加序号）</option>
                </Select>
              </label>
            </div>
            <ul className="divide-y divide-border overflow-hidden rounded-panel border border-border">
              {preview.candidates.map((candidate) => (
                <li key={candidate.relativePath} className="flex items-start gap-3 px-3 py-2.5">
                  <Checkbox
                    aria-label={`选择候选 ${candidate.name}`}
                    checked={chosen.has(candidate.relativePath)}
                    onChange={(event) => toggleCandidate(candidate, event.target.checked)}
                  />
                  <div className="min-w-0 flex-1">
                    <div className="flex items-center gap-2">
                      <span className="truncate text-body font-medium text-fg-1">
                        {candidate.name}
                      </span>
                      {candidate.exists && <Badge tone="warning">{scope.existsLabel}</Badge>}
                    </div>
                    <p className="truncate text-caption text-fg-3" title={candidate.description ?? undefined}>
                      {candidate.relativePath} · 将安装为 <span className="font-mono">{candidate.suggestedId}</span>
                      {candidate.description ? ` · ${candidate.description}` : ""}
                    </p>
                  </div>
                </li>
              ))}
            </ul>
          </div>
        )}

        {report && (
          <div className="space-y-2">
            <p className="text-body font-medium text-fg-1">安装结果</p>
            <ul className="divide-y divide-border overflow-hidden rounded-panel border border-border">
              {(report.outcomes ?? []).map((outcome) => (
                <li key={`${outcome.id}-${outcome.name}`} className="space-y-0.5 px-3 py-2.5">
                  <div className="flex items-center gap-2">
                    <span className="font-mono text-caption text-fg-3">{outcome.id}</span>
                    <Badge
                      tone={
                        outcome.status === "failed"
                          ? "danger"
                          : outcome.status === "skipped"
                            ? "neutral"
                            : "success"
                      }
                    >
                      {STATUS_LABEL[outcome.status] ?? outcome.status}
                    </Badge>
                    <span className="truncate text-caption text-fg-2">{outcome.name}</span>
                  </div>
                  {outcome.error && (
                    <p className="text-caption text-danger-text">{outcome.error}</p>
                  )}
                  {outcome.backupPath && (
                    <p className="truncate text-caption text-fg-3" title={outcome.backupPath}>
                      旧版本备份：{outcome.backupPath}
                    </p>
                  )}
                  {outcome.warnings?.map((warning) => (
                    <p key={warning} className="text-caption text-warning-text">
                      {warning}
                    </p>
                  ))}
                </li>
              ))}
            </ul>
          </div>
        )}
      </div>
    </Sheet>
  );
}
