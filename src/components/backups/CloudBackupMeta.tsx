import { Notice } from "../ui/Notice";
import { formatBytes, formatRelativeTime } from "../../lib/format";
import type { CloudStatus } from "../../types";

function Fact({
  label,
  value,
  title,
}: {
  label: string;
  value: string;
  title?: string;
}) {
  return (
    <div className="min-w-0">
      <dt className="text-caption text-fg-3">{label}</dt>
      <dd className="truncate text-body text-fg-1" title={title}>
        {value}
      </dd>
    </div>
  );
}

/**
 * 云端包的身份信息：恢复前先看清包是哪台机器、什么时候传的、有多大，
 * 避免拿旧包覆盖当前真源。旧版裸包没有 manifest，只展示能确认的大小。
 */
export function CloudBackupMeta({ status }: { status: CloudStatus | null }) {
  if (!status) return null;
  const manifest = status.manifest;
  const remote = status.remote;
  const uploadedAt = manifest ? new Date(manifest.createdAt).toLocaleString() : null;
  const missing = remote != null && !remote.exists;

  return (
    <div className="space-y-2">
      <dl className="grid grid-cols-2 gap-x-4 gap-y-2 rounded-panel border border-border bg-subtle px-3 py-2.5">
        <Fact
          label="备份来源"
          value={
            manifest
              ? `${manifest.deviceName} · v${manifest.appVersion}`
              : remote?.exists
                ? "旧版裸包（无 manifest）"
                : "—"
          }
          title={
            manifest
              ? `由 ${manifest.deviceName} 于 ${uploadedAt} 上传`
              : "旧版上传的包没有 manifest，无法确认来源"
          }
        />
        <Fact
          label="上传时间"
          value={manifest ? formatRelativeTime(manifest.createdAt) : "—"}
          title={uploadedAt ?? undefined}
        />
        <Fact
          label="包含文件"
          value={manifest && manifest.files > 0 ? `${manifest.files} 个` : "—"}
        />
        <Fact
          label="包大小"
          value={remote?.sizeBytes != null ? formatBytes(remote.sizeBytes) : "—"}
        />
      </dl>
      {status.remoteError && (
        <Notice tone="danger">云端状态读取失败：{status.remoteError}</Notice>
      )}
      {missing && !status.remoteError && (
        <Notice tone="warning">云端当前没有可恢复的备份，请先在源设备上传。</Notice>
      )}
      {manifest && !manifest.compatible && (
        <Notice tone="danger">
          该备份由不兼容的版本上传（manifest schema {manifest.schema}），恢复会被拒绝，请先升级应用。
        </Notice>
      )}
      {status.manifestError && (
        <Notice tone="warning">云端 manifest 无法解析：{status.manifestError}</Notice>
      )}
    </div>
  );
}
