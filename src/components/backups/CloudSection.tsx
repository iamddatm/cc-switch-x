import { useEffect, useRef, useState } from "react";
import { Cloud, Eye, EyeOff, RefreshCw } from "lucide-react";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Field } from "../ui/Field";
import { IconButton } from "../ui/IconButton";
import { Input } from "../ui/Input";
import { Notice } from "../ui/Notice";
import { formatBytes, formatRelativeTime } from "../../lib/format";
import type { CloudSettingsView, CloudStatus } from "../../types";

/**
 * 云备份表单的草稿。`password` 为 null 表示「保持不变」——
 * 后端里的密码只存在 secrets.local.json，页面留空不能被当成清空。
 */
export interface CloudDraft {
  baseUrl: string;
  username: string;
  remoteDir: string;
  password: string | null;
}

function cloudState(status: CloudStatus | null): {
  label: string;
  tone: "neutral" | "success" | "warning" | "danger";
} {
  if (!status?.configured) return { label: "未配置", tone: "neutral" };
  if (status.remoteError) return { label: "连接失败", tone: "danger" };
  if (status.remote?.exists) return { label: "云端有备份", tone: "success" };
  return { label: "已配置", tone: "neutral" };
}

/**
 * 云备份（WebDAV）区。密码只在本机 `secrets.local.json` 里，输入框留空表示保持不变，
 * 所以草稿带着一个 dirty 标记，交给容器翻译成 `password: null`。
 */
export function CloudSection({
  cloud,
  cloudStatus,
  loading,
  busy,
  onRefreshStatus,
  onTest,
  onSave,
  onUpload,
  onOpenRestore,
}: {
  cloud: CloudSettingsView | null;
  cloudStatus: CloudStatus | null;
  loading: boolean;
  busy: string | null;
  onRefreshStatus: () => void;
  onTest: (draft: CloudDraft) => void;
  onSave: (draft: CloudDraft) => Promise<CloudSettingsView | null>;
  onUpload: () => void;
  onOpenRestore: () => void;
}) {
  // 连接草稿归本区：容器只负责存到后端并回报结果，不再一路传 state + setter。
  const [baseUrl, setBaseUrl] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [passwordDirty, setPasswordDirty] = useState(false);
  const [showPassword, setShowPassword] = useState(false);
  const [remoteDir, setRemoteDir] = useState("ccswitchx");
  const seeded = useRef(false);
  useEffect(() => {
    if (seeded.current || !cloud) return;
    setBaseUrl(cloud.baseUrl);
    setUsername(cloud.username);
    setRemoteDir(cloud.remoteDir);
    seeded.current = true;
  }, [cloud]);

  const draft = (): CloudDraft => ({
    baseUrl,
    username,
    remoteDir,
    password: passwordDirty ? password : null,
  });

  const applySaved = (saved: CloudSettingsView) => {
    setBaseUrl(saved.baseUrl);
    setUsername(saved.username);
    setRemoteDir(saved.remoteDir);
    setPassword("");
    setPasswordDirty(false);
    setShowPassword(false);
  };

  const save = async () => {
    const saved = await onSave(draft());
    if (saved) applySaved(saved);
  };

  const remotePath = baseUrl.trim()
    ? `${baseUrl.trim().replace(/\/+$/, "")}/${remoteDir.trim().replace(/^\/+/, "")}/ccswitchx-backup.zip`
    : "—";
  const badge = cloudState(cloudStatus);

  return (
    <section className="rounded-panel border border-border bg-surface">
      <div className="flex items-center gap-2 border-b border-border px-4 py-3">
        <Cloud size={15} className="text-fg-3" aria-hidden />
        <h3 className="text-strong text-fg-1">云备份（WebDAV）</h3>
        <Badge tone={badge.tone}>{badge.label}</Badge>
        <span className="ml-auto flex gap-1.5">
          <IconButton label="刷新云状态" disabled={loading} onClick={onRefreshStatus}>
            <RefreshCw size={13} aria-hidden />
          </IconButton>
        </span>
      </div>

      {!cloud ? (
        <p className="px-4 py-3 text-caption text-fg-3">正在读取云备份设置…</p>
      ) : (
        <>
          <div className="space-y-3 border-b border-border px-4 py-3">
            {baseUrl.startsWith("http://") && (
              <Notice tone="warning">
                HTTP 会明文传输凭据，仅建议在内网使用。
              </Notice>
            )}
            <div className="flex flex-wrap items-end gap-2">
              <Field
                label="WebDAV 地址"
                htmlFor="cloud-base-url"
                className="min-w-[280px] flex-1"
              >
                <Input
                  id="cloud-base-url"
                  className="font-mono"
                  placeholder="https://dav.example.com/dav"
                  value={baseUrl}
                  onChange={(event) => setBaseUrl(event.target.value)}
                />
              </Field>
              <Field label="用户名" htmlFor="cloud-username" className="w-40">
                <Input
                  id="cloud-username"
                  value={username}
                  onChange={(event) => setUsername(event.target.value)}
                />
              </Field>
            </div>
            <div className="flex flex-wrap items-end gap-2">
              <Field label="密码" htmlFor="cloud-password" className="w-56">
                <div className="flex items-center gap-1">
                  <Input
                    id="cloud-password"
                    type={showPassword ? "text" : "password"}
                    placeholder={
                      cloud.hasPassword && !passwordDirty ? "已保存，留空保持不变" : ""
                    }
                    value={password}
                    onChange={(event) => {
                      setPassword(event.target.value);
                      setPasswordDirty(true);
                    }}
                  />
                  <IconButton
                    label={showPassword ? "隐藏密码" : "显示密码"}
                    onClick={() => setShowPassword((value) => !value)}
                  >
                    {showPassword ? <EyeOff size={14} aria-hidden /> : <Eye size={14} aria-hidden />}
                  </IconButton>
                </div>
              </Field>
              <Field label="远端目录" htmlFor="cloud-remote-dir" className="w-40">
                <Input
                  id="cloud-remote-dir"
                  className="font-mono"
                  value={remoteDir}
                  onChange={(event) => setRemoteDir(event.target.value)}
                />
              </Field>
            </div>
            <p className="text-caption text-fg-3">
              在页面顶部选择「WebDAV 云备份」后，每次自动快照都会上传；与 Git 备份互斥。
            </p>
            <div className="flex flex-wrap gap-2">
              <Button
                onClick={() => onTest(draft())}
                loading={busy === "cloud-test"}
                disabled={!baseUrl.trim() || (!!busy && busy !== "cloud-test")}
              >
                测试连接
              </Button>
              <Button
                onClick={() => void save()}
                loading={busy === "cloud-save"}
                disabled={!!busy && busy !== "cloud-save"}
              >
                保存云备份
              </Button>
              <Button
                onClick={onUpload}
                loading={busy === "cloud-upload"}
                disabled={!cloudStatus?.configured || (!!busy && busy !== "cloud-upload")}
              >
                立即上传
              </Button>
              <Button variant="danger" disabled={!cloudStatus?.configured || !!busy} onClick={onOpenRestore}>
                从云端恢复
              </Button>
            </div>
            <p className="text-caption text-fg-3">
              上传会把当前真源打包并覆盖云端唯一一份备份；恢复前会自动创建本地安全快照。
            </p>
          </div>

          <div className="grid grid-cols-2 gap-3 px-4 py-3 sm:grid-cols-3">
            <div className="min-w-0">
              <p className="text-caption text-fg-3">远端对象</p>
              <p className="truncate font-mono text-caption text-fg-1" title={remotePath}>
                {remotePath}
              </p>
            </div>
            <div className="min-w-0">
              <p className="text-caption text-fg-3">上传来源</p>
              <p
                className="truncate text-body text-fg-1"
                title={
                  cloudStatus?.manifest
                    ? `${cloudStatus.manifest.deviceName} · v${cloudStatus.manifest.appVersion} · ${
                        cloudStatus.manifest.files
                      } 个文件 · ${formatRelativeTime(cloudStatus.manifest.createdAt)}上传`
                    : "旧版上传的备份没有 manifest 描述文件"
                }
              >
                {cloudStatus?.manifest
                  ? `${cloudStatus.manifest.deviceName} · v${cloudStatus.manifest.appVersion}`
                  : "—"}
              </p>
            </div>
            <div>
              <p className="text-caption text-fg-3">远端大小</p>
              <p className="text-body text-fg-1">
                {cloudStatus?.remote?.sizeBytes != null
                  ? formatBytes(cloudStatus.remote.sizeBytes)
                  : "—"}
              </p>
            </div>
            <div>
              <p className="text-caption text-fg-3">远端修改时间</p>
              <p className="text-body text-fg-1">
                {cloudStatus?.remote?.modifiedAt
                  ? formatRelativeTime(cloudStatus.remote.modifiedAt)
                  : "—"}
              </p>
            </div>
            <div>
              <p className="text-caption text-fg-3">上次上传</p>
              <p className="text-body text-fg-1">
                {cloudStatus?.lastUploadAt ? formatRelativeTime(cloudStatus.lastUploadAt) : "—"}
              </p>
            </div>
          </div>

          {cloudStatus?.remoteError && (
            <div className="px-4 pb-3">
              <Notice tone="danger">{cloudStatus.remoteError}</Notice>
            </div>
          )}
          {cloudStatus?.manifest && !cloudStatus.manifest.compatible && (
            <div className="px-4 pb-3">
              <Notice tone="danger">
                云端备份由不兼容的版本上传（manifest schema {cloudStatus.manifest.schema}），
                请升级 CC Switch X 后再恢复。
              </Notice>
            </div>
          )}
          {cloudStatus?.manifestError && (
            <div className="px-4 pb-3">
              <Notice tone="warning">
                云端 manifest 无法解析：{cloudStatus.manifestError}
              </Notice>
            </div>
          )}
          {cloudStatus?.remote?.exists &&
            !cloudStatus.manifest &&
            !cloudStatus.manifestError && (
              <div className="px-4 pb-3">
                <Notice tone="info">
                  云端备份是旧版上传的裸包，没有 manifest：来源与完整性无法核对。
                  重新上传一次即可补齐。
                </Notice>
              </div>
            )}
          {cloudStatus?.lastError && (
            <div className="px-4 pb-3">
              <Notice tone="warning">
                上次上传失败：{cloudStatus.lastError}
              </Notice>
            </div>
          )}
        </>
      )}
    </section>
  );
}
