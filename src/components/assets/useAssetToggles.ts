import { useState } from "react";
import type { AgentStatus } from "../../types";
import { callToggle, outcomeLabel, type MatrixAsset } from "./matrix";

export interface AssetToggles {
  busy: string | null;
  error: string | null;
  outcomes: Record<string, string>;
  toggle: (agent: AgentStatus, enabled: boolean) => Promise<void>;
}

/** 单行「资产 × agent」开关的状态；行内紧凑矩阵和纵向 AgentMatrix 共用同一套调用。 */
export function useAssetToggles(
  assetType: MatrixAsset,
  id: string,
  onChanged: () => void,
  onError?: (error: string) => void,
  onOutcome?: (label: string) => void,
): AssetToggles {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [outcomes, setOutcomes] = useState<Record<string, string>>({});

  const toggle = async (agent: AgentStatus, enabled: boolean) => {
    setBusy(agent.id);
    try {
      const label = outcomeLabel(await callToggle(assetType, id, agent.id, enabled));
      setOutcomes((current) => ({ ...current, [agent.id]: label }));
      setError(null);
      onOutcome?.(label);
      onChanged();
    } catch (e) {
      const message = String(e);
      setError(message);
      onError?.(message);
    } finally {
      setBusy(null);
    }
  };

  return { busy, error, outcomes, toggle };
}