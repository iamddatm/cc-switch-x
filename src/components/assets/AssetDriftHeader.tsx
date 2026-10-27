import type { useDrift } from "../../hooks/useDrift";
import { DriftBanner } from "../DriftBanner";

/** useDrift 返回值里横幅要用的那几项。 */
type DriftState = Pick<ReturnType<typeof useDrift>, "forAsset" | "busy" | "error" | "rescan" | "resolve">;

/**
 * 整文件资产面板右栏顶部的漂移区：当前选中资产的回读横幅 + 钩子级错误。
 * Rules / Subagents 的形状完全一致，收在一处，横幅以后加字段只改这里。
 */
export function AssetDriftHeader({
  assetType,
  selected,
  drift,
}: {
  assetType: string;
  selected: string | null;
  drift: DriftState;
}) {
  return (
    <>
      <DriftBanner
        drifts={selected ? drift.forAsset(assetType, selected) : []}
        busy={drift.busy}
        onResolve={drift.resolve}
        onRescan={() => void drift.rescan()}
      />
      {drift.error && <p className="mb-2 text-caption text-danger-text">{drift.error}</p>}
    </>
  );
}
