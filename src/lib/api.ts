import { invoke } from "@tauri-apps/api/core";
import type { AppSettings, AssetSummary, AssetType, Theme } from "../types";

export const api = {
  listAssets: (assetType?: AssetType) => invoke<AssetSummary[]>("list_assets", { assetType }),
  readAsset: (assetType: AssetType, id: string) => invoke<string>("read_asset", { assetType, id }),
  writeAsset: (assetType: AssetType, id: string, content: string) =>
    invoke<AssetSummary>("write_asset", { assetType, id, content }),
  deleteAsset: (assetType: AssetType, id: string) =>
    invoke<boolean>("delete_asset", { assetType, id }),
  getSettings: () => invoke<AppSettings>("get_settings"),
  setTheme: (theme: Theme) => invoke<AppSettings>("set_theme", { theme }),
  setStoreRoot: (storeRoot: string) => invoke<boolean>("set_store_root", { storeRoot }),
};
