import { useCallback, useEffect, useState } from "react";
import { api } from "../lib/api";
import type { AssetSummary, AssetType } from "../types";

export function useAssets(assetType: AssetType) {
  const [items, setItems] = useState<AssetSummary[]>([]);
  const [error, setError] = useState<string | null>(null);
  const [loading, setLoading] = useState(true);
  const refresh = useCallback(async () => {
    setLoading(true);
    try {
      setItems(await api.listAssets(assetType));
      setError(null);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [assetType]);
  useEffect(() => {
    void refresh();
  }, [refresh]);
  return { items, error, loading, refresh };
}
