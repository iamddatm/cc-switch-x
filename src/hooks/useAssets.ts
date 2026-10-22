import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type { AssetSummary, AssetType } from "../types";

export function useAssets(assetType: AssetType) {
  const [items, setItems] = useState<AssetSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const refresh = useCallback(async () => {
    try {
      setItems(await api.listAssets(assetType));
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, [assetType]);
  useEffect(() => {
    void refresh();
  }, [refresh]);
  return { items, error, refresh };
}
