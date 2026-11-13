import { useEffect, useRef, useState } from "react";
import { Cloud, Eye, EyeOff, RefreshCw } from "lucide-react";
import { Badge } from "../ui/Badge";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Checkbox";
import { Field } from "../ui/Field";
import { IconButton } from "../ui/IconButton";
import { Input } from "../ui/Input";
import { Notice } from "../ui/Notice";
import { Select } from "../ui/Select";
import { formatBytes, formatRelativeTime } from "../../lib/format";
import type {
  CloudProvider,
  CloudSettingsView,
  CloudStatus,
} from "../../types";

/**
 * 云备份表单的草稿。密码 / Secret 为 null 表示「保持不变」——
 * 后端里的密钥只存在 secrets.local.json，页面留空不能被当成清空。
 * 两个通道的字段放在同一份草稿里，切换标签页时未保存的输入不会丢。
 */
export interface CloudDraft {
  provider: CloudProvider;
  baseUrl: string;
  username: string;
  remoteDir: string;
  password: string | null;
  ossRegion: string;
  ossBucket: string;
  ossAccessKeyId: string;
  ossPrefix: string;
  ossAccessKeySecret: string | null;
}

interface ChannelDraft {
  baseUrl: string;
  username: string;
  remoteDir: string;
  password: string;
  passwordDirty: boolean;
  ossRegion: string;
  ossBucket: string;
  ossAccessKeyId: string;
  ossPrefix: string;
  ossAccessKeySecret: string;
  ossSecretDirty: boolean;
}

function emptyDraft(): ChannelDraft {
  return {
    baseUrl: "",
    username: "",
    remoteDir: "ccswitchx",
    password: "",
    passwordDirty: false,
    ossRegion: "",
    ossBucket: "",
    ossAccessKeyId: "",
    ossPrefix: "ccswitchx",
    ossAccessKeySecret: "",
    ossSecretDirty: false,
  };
}

const PROVIDER_LABEL: Record<CloudProvider, string> = {
  webdav: "WebDAV",
  oss: "阿里云 OSS",
};

function cloudState(
  status: CloudStatus | null,
  active: boolean,
): { label: string; tone: "neutral" | "success" | "warning" | "danger" } {
  if (!active) return { label: "未启用", tone: "neutral" };
  if (!status?.configured) return { label: "未配置", tone: "neutral" };
  if (status.remoteError) return { label: "连接失败", tone: "danger" };
  if (status.remote?.exists) return { label: "云端有备份", tone: "success" };
  return { label: "已配置", tone: "neutral" };
}

/**
 * 云备份区：按 `provider` 显示 WebDAV 或阿里云 OSS 的连接表单。
 * 两个通道的草稿都归本组件，切换标签页只换字段，不清草稿。
 */
export function CloudSection({
  provider,
  cloud,
  cloudStatus,
  cloudChecking,
  loading,
  busy,
  onRefreshStatus,
  onTest,
  onSave,
  onToggleChannel,
  onToggleAuto,
  onUpload,
  onOpenRestore,
}: {
  provider: CloudProvider;
  cloud: CloudSettingsView | null;
  cloudStatus: CloudStatus | null;
  cloudChecking: boolean;
  loading: boolean;
  busy: string | null;
  onRefreshStatus: () => void;
  onTest: (draft: CloudDraft, provider: CloudProvider) => void;
  onSave: (
    draft: CloudDraft,
    provider: CloudProvider,
  ) => Promise<CloudSettingsView | null>;
  onToggleChannel: (provider: CloudProvider, enabled: boolean) => void;
  onToggleAuto: (
    patch: Partial<
      Pick<CloudSettingsView, "autoUpload" | "autoDownload" | "uploadDelayHours">
    >,
  ) => void;
  onUpload: () => void;
  onOpenRestore: () => void;
}) {
  const [draft, setDraft] = useState<ChannelDraft>(emptyDraft);
  const [showSecret, setShowSecret] = useState(false);
  const seeded = useRef(false);
  useEffect(() => {
    if (seeded.current || !cloud) return;
    setDraft({
      baseUrl: cloud.baseUrl,
      username: cloud.username,
      remoteDir: cloud.remoteDir,
      password: "",
      passwordDirty: false,
      ossRegion: cloud.ossRegion,
      ossBucket: cloud.ossBucket,
      ossAccessKeyId: cloud.ossAccessKeyId,
      ossPrefix: cloud.ossPrefix,
      ossAccessKeySecret: "",
      ossSecretDirty: false,
    });
    seeded.current = true;
  }, [cloud]);

  /** 另一通道的字段取已保存值：保存本通道时不能把它清掉。 */
  const draftFor = (target: CloudProvider): CloudDraft => ({
    provider: target,
    baseUrl: target === "webdav" ? draft.baseUrl : (cloud?.baseUrl ?? ""),
    username: target === "webdav" ? draft.username : (cloud?.username ?? ""),
    remoteDir: target === "webdav" ? draft.remoteDir : (cloud?.remoteDir ?? ""),
    password:
      target === "webdav" && draft.passwordDirty ? draft.password : null,
    ossRegion: target === "oss" ? draft.ossRegion : (cloud?.ossRegion ?? ""),
    ossBucket: target === "oss" ? draft.ossBucket : (cloud?.ossBucket ?? ""),
    ossAccessKeyId:
      target === "oss" ? draft.ossAccessKeyId : (cloud?.ossAccessKeyId ?? ""),
    ossPrefix: target === "oss" ? draft.ossPrefix : (cloud?.ossPrefix ?? ""),
    ossAccessKeySecret:
      target === "oss" && draft.ossSecretDirty ? draft.ossAccessKeySecret : null,
  });

  const applySaved = (target: CloudProvider, saved: CloudSettingsView) => {
    setShowSecret(false);
    if (target === "webdav") {
      setDraft((current) => ({
        ...current,
        baseUrl: saved.baseUrl,
        username: saved.username,
        remoteDir: saved.remoteDir,
        password: "",
        passwordDirty: false,
      }));
    } else {
      setDraft((current) => ({
        ...current,
        ossRegion: saved.ossRegion,
        ossBucket: saved.ossBucket,
        ossAccessKeyId: saved.ossAccessKeyId,
        ossPrefix: saved.ossPrefix,
        ossAccessKeySecret: "",
        ossSecretDirty: false,
      }));
    }
  };

  const save = async () => {
    const saved = await onSave(draftFor(provider), provider);
    if (saved) applySaved(provider, saved);
  };

  const active = !!cloud?.enabled && cloud.provider === provider;
  // 状态与手动操作跟随「当前配置的通道」，不要求它已启用；自动任务才要求启用。
  const statusVisible = !!cloud && cloud.provider === provider;
  const webdavDirty =
    draft.passwordDirty ||
    draft.baseUrl !== (cloud?.baseUrl ?? "") ||
    draft.username !== (cloud?.username ?? "") ||
    draft.remoteDir !== (cloud?.remoteDir ?? "");
  const ossDirty =
    draft.ossSecretDirty ||
    draft.ossRegion !== (cloud?.ossRegion ?? "") ||
    draft.ossBucket !== (cloud?.ossBucket ?? "") ||
    draft.ossAccessKeyId !== (cloud?.ossAccessKeyId ?? "") ||
    draft.ossPrefix !== (cloud?.ossPrefix ?? "");
  const draftDirty = provider === "webdav" ? webdavDirty : ossDirty;

  // 启用只认已保存的配置：草稿没保存就启用，自动任务会拿着旧连接跑。
  const configured =
    provider === "webdav"
      ? !!cloud?.baseUrl && !!cloud?.username && !!cloud?.hasPassword
      : !!cloud?.ossRegion &&
        !!cloud?.ossBucket &&
        !!cloud?.ossAccessKeyId &&
        !!cloud?.hasOssSecret;
  const canEnable = configured && !draftDirty && !busy;

  const knownMissing =
    statusVisible && cloudStatus?.remote?.exists === false;
  const knownIncompatible =
    statusVisible &&
    (cloudStatus?.manifest?.compatible === false ||
      (cloudStatus?.remote?.exists === true && !cloudStatus?.manifest));
  const restoreBlocked =
    !!busy ||
    draftDirty ||
    !statusVisible ||
    !cloudStatus?.configured ||
    knownMissing ||
    knownIncompatible;
  const delayHours = cloud?.uploadDelayHours ?? 1;
  const autoUploadHint = `改动停下 ${delayHours} 小时后上传；持续编辑最多每 ${Math.min(
    delayHours * 4,
    24,
  )} 小时补传一次，内容没变不重复上传。`;
  const badge = cloudState(cloudStatus, statusVisible);

  const remotePath =
    provider === "webdav"
      ? draft.baseUrl.trim()
        ? `${draft.baseUrl.trim().replace(/\/+$/, "")}/${draft.remoteDir
            .trim()
            .replace(/^\/+/, "")}/ccswitchx-backup.zip`
        : "—"
      : draft.ossRegion.trim() && draft.ossBucket.trim()
        ? `${draft.ossBucket.trim()}.s3.oss-${draft.ossRegion.trim()}.aliyuncs.com/${draft.ossPrefix
            .trim()
            .replace(/^\/+/, "")}/ccswitchx-backup.zip`
        : "—";

  return (
    <section className="rounded-panel border border-border bg-surface">
      <div className="flex items-center gap-2 border-b border-border px-4 py-3">
        <Cloud size={15} className="text-fg-3" aria-hidden />
        <h3 className="text-strong text-fg-1">
          云备份（{PROVIDER_LABEL[provider]}）
        </h3>
        <Badge tone={badge.tone}>{badge.label}</Badge>
        <span className="ml-auto flex items-center gap-2">
          {statusVisible && cloudChecking && (
            <span className="text-caption text-fg-3">检查中…</span>
          )}
          <IconButton
            label="刷新云状态"
            disabled={loading || !statusVisible}
            onClick={onRefreshStatus}
          >
            <RefreshCw size={13} aria-hidden />
          </IconButton>
        </span>
      </div>

      {!cloud ? (
        <p className="px-4 py-3 text-caption text-fg-3">正在读取云备份设置…</p>
      ) : (
        <>
          <div className="space-y-3 border-b border-border px-4 py-3">
            {provider === "webdav" ? (
              <>
                {draft.baseUrl.startsWith("http://") && (
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
                      value={draft.baseUrl}
                      onChange={(event) =>
                        setDraft((current) => ({
                          ...current,
                          baseUrl: event.target.value,
                        }))
                      }
                    />
                  </Field>
                  <Field label="用户名" htmlFor="cloud-username" className="w-40">
                    <Input
                      id="cloud-username"
                      value={draft.username}
                      onChange={(event) =>
                        setDraft((current) => ({
                          ...current,
                          username: event.target.value,
                        }))
                      }
                    />
                  </Field>
                </div>
                <div className="flex flex-wrap items-end gap-2">
                  <Field label="密码" htmlFor="cloud-password" className="w-56">
                    <div className="flex items-center gap-1">
                      <Input
                        id="cloud-password"
                        type={showSecret ? "text" : "password"}
                        placeholder={
                          cloud.hasPassword && !draft.passwordDirty
                            ? "已保存，留空保持不变"
                            : ""
                        }
                        value={draft.password}
                        onChange={(event) =>
                          setDraft((current) => ({
                            ...current,
                            password: event.target.value,
                            passwordDirty: true,
                          }))
                        }
                      />
                      <IconButton
                        label={showSecret ? "隐藏密码" : "显示密码"}
                        onClick={() => setShowSecret((value) => !value)}
                      >
                        {showSecret ? (
                          <EyeOff size={14} aria-hidden />
                        ) : (
                          <Eye size={14} aria-hidden />
                        )}
                      </IconButton>
                    </div>
                  </Field>
                  <Field label="远端目录" htmlFor="cloud-remote-dir" className="w-40">
                    <Input
                      id="cloud-remote-dir"
                      className="font-mono"
                      value={draft.remoteDir}
                      onChange={(event) =>
                        setDraft((current) => ({
                          ...current,
                          remoteDir: event.target.value,
                        }))
                      }
                    />
                  </Field>
                </div>
              </>
            ) : (
              <>
                <div className="flex flex-wrap items-end gap-2">
                  <Field label="地域" htmlFor="cloud-oss-region" className="w-44">
                    <Input
                      id="cloud-oss-region"
                      className="font-mono"
                      placeholder="cn-hangzhou"
                      value={draft.ossRegion}
                      onChange={(event) =>
                        setDraft((current) => ({
                          ...current,
                          ossRegion: event.target.value,
                        }))
                      }
                    />
                  </Field>
                  <Field label="Bucket" htmlFor="cloud-oss-bucket" className="w-52">
                    <Input
                      id="cloud-oss-bucket"
                      className="font-mono"
                      value={draft.ossBucket}
                      onChange={(event) =>
                        setDraft((current) => ({
                          ...current,
                          ossBucket: event.target.value,
                        }))
                      }
                    />
                  </Field>
                  <Field label="对象前缀" htmlFor="cloud-oss-prefix" className="w-40">
                    <Input
                      id="cloud-oss-prefix"
                      className="font-mono"
                      value={draft.ossPrefix}
                      onChange={(event) =>
                        setDraft((current) => ({
                          ...current,
                          ossPrefix: event.target.value,
                        }))
                      }
                    />
                  </Field>
                </div>
                <div className="flex flex-wrap items-end gap-2">
                  <Field
                    label="AccessKey ID"
                    htmlFor="cloud-oss-access-key-id"
                    className="w-64"
                  >
                    <Input
                      id="cloud-oss-access-key-id"
                      className="font-mono"
                      value={draft.ossAccessKeyId}
                      onChange={(event) =>
                        setDraft((current) => ({
                          ...current,
                          ossAccessKeyId: event.target.value,
                        }))
                      }
                    />
                  </Field>
                  <Field
                    label="AccessKey Secret"
                    htmlFor="cloud-oss-secret"
                    className="w-64"
                  >
                    <div className="flex items-center gap-1">
                      <Input
                        id="cloud-oss-secret"
                        type={showSecret ? "text" : "password"}
                        placeholder={
                          cloud.hasOssSecret && !draft.ossSecretDirty
                            ? "已保存，留空保持不变"
                            : ""
                        }
                        value={draft.ossAccessKeySecret}
                        onChange={(event) =>
                          setDraft((current) => ({
                            ...current,
                            ossAccessKeySecret: event.target.value,
                            ossSecretDirty: true,
                          }))
                        }
                      />
                      <IconButton
                        label={showSecret ? "隐藏密钥" : "显示密钥"}
                        onClick={() => setShowSecret((value) => !value)}
                      >
                        {showSecret ? (
                          <EyeOff size={14} aria-hidden />
                        ) : (
                          <Eye size={14} aria-hidden />
                        )}
                      </IconButton>
                    </div>
                  </Field>
                </div>
                <p className="text-caption text-fg-3">
                  走阿里云 OSS 的 S3 兼容接口；AccessKey 建议使用只授予该 Bucket
                  读写权限的 RAM 子账号。
                </p>
              </>
            )}

            <div className="space-y-2">
              <div className="flex flex-wrap items-end gap-x-4 gap-y-2">
                <div className="pb-1.5">
                  <Checkbox
                    id="cloud-auto-upload"
                    label="自动上传"
                    checked={cloud.autoUpload}
                    disabled={!active || !!busy}
                    onChange={(event) =>
                      onToggleAuto({ autoUpload: event.target.checked })
                    }
                  />
                </div>
                <Field label="上传延迟" htmlFor="cloud-upload-delay" className="w-32">
                  <Select
                    id="cloud-upload-delay"
                    value={String(cloud.uploadDelayHours)}
                    disabled={!active || !!busy}
                    onChange={(event) =>
                      onToggleAuto({ uploadDelayHours: Number(event.target.value) })
                    }
                  >
                    {[1, 6, 12, 24].map((hours) => (
                      <option key={hours} value={hours}>
                        {hours} 小时
                      </option>
                    ))}
                  </Select>
                </Field>
              </div>
              <p className="text-caption text-fg-3">{autoUploadHint}</p>
              <div>
                <Checkbox
                  id="cloud-auto-download"
                  label="自动下载"
                  checked={cloud.autoDownload}
                  disabled={!active || !!busy}
                  onChange={(event) =>
                    onToggleAuto({ autoDownload: event.target.checked })
                  }
                />
              </div>
              <p className="text-caption text-fg-3">
                启动时及每 15 分钟检查云端；有更新就覆盖本机（云端为准，覆盖前自动建安全快照）。
              </p>
            </div>
            {draftDirty && (
              <Notice tone="warning">
                配置有未保存的更改：上传与恢复会使用已保存的连接，请先保存。
              </Notice>
            )}
            <div className="flex flex-wrap gap-2">
              <Button
                onClick={() => onTest(draftFor(provider), provider)}
                loading={busy === "cloud-test"}
                disabled={
                  (provider === "webdav"
                    ? !draft.baseUrl.trim()
                    : !draft.ossRegion.trim() || !draft.ossBucket.trim()) ||
                  (!!busy && busy !== "cloud-test")
                }
              >
                测试连接
              </Button>
              <Button
                onClick={() => void save()}
                loading={busy === "cloud-save"}
                disabled={!!busy && busy !== "cloud-save"}
              >
                保存配置
              </Button>
              {active ? (
                <Button
                  variant="danger"
                  loading={busy === "cloud-channel"}
                  disabled={!!busy && busy !== "cloud-channel"}
                  onClick={() => onToggleChannel(provider, false)}
                >
                  停用云端备份
                </Button>
              ) : (
                <Button
                  variant="primary"
                  loading={busy === "cloud-channel"}
                  disabled={!canEnable || (!!busy && busy !== "cloud-channel")}
                  onClick={() => onToggleChannel(provider, true)}
                >
                  启用 {PROVIDER_LABEL[provider]} 备份
                </Button>
              )}
              <Button
                onClick={onUpload}
                loading={busy === "cloud-upload"}
                disabled={
                  !statusVisible ||
                  !cloudStatus?.configured ||
                  draftDirty ||
                  (!!busy && busy !== "cloud-upload")
                }
              >
                立即上传
              </Button>
              <Button
                variant="danger"
                disabled={restoreBlocked}
                title={
                  restoreBlocked
                    ? "云端没有可恢复的备份，或该备份与当前版本不兼容"
                    : undefined
                }
                onClick={onOpenRestore}
              >
                从云端恢复
              </Button>
            </div>
            <p className="text-caption text-fg-3">
              {active
                ? "上传会把当前真源打包并覆盖云端唯一一份备份；恢复前会自动创建本地安全快照。"
                : "先保存完整连接配置，再启用此通道；同一时刻只能启用一种云端备份。"}
            </p>
          </div>

          {statusVisible && (
            <>
              <div className="grid grid-cols-2 gap-3 px-4 py-3 sm:grid-cols-3">
                <div className="min-w-0">
                  <p className="text-caption text-fg-3">远端对象</p>
                  <p
                    className="truncate font-mono text-caption text-fg-1"
                    title={remotePath}
                  >
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
                        : "尚未探测到云端 manifest"
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
                    {cloudStatus?.lastUploadAt
                      ? formatRelativeTime(cloudStatus.lastUploadAt)
                      : "—"}
                  </p>
                </div>
                <div>
                  <p className="text-caption text-fg-3">上次下载</p>
                  <p className="text-body text-fg-1">
                    {cloudStatus?.lastDownloadAt
                      ? formatRelativeTime(cloudStatus.lastDownloadAt)
                      : "—"}
                  </p>
                </div>
                <div>
                  <p className="text-caption text-fg-3">本机本月上传</p>
                  <p className="text-body text-fg-1">
                    {cloudStatus ? formatBytes(cloudStatus.uploadedBytesMonth) : "—"}
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
                    {`云端备份由不兼容的版本上传（manifest schema ${cloudStatus.manifest.schema}），请从源设备重新上传一次。`}
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
              {cloudStatus?.lastError && (
                <div className="px-4 pb-3">
                  <Notice tone="warning">
                    上次上传失败：{cloudStatus.lastError}
                  </Notice>
                </div>
              )}
              {cloudStatus?.lastDownloadError && (
                <div className="px-4 pb-3">
                  <Notice tone="warning">
                    上次自动下载失败：{cloudStatus.lastDownloadError}
                  </Notice>
                </div>
              )}
            </>
          )}
        </>
      )}
    </section>
  );
}
