import { useEffect, useState } from "react";
import { api } from "../../lib/api";
import type { AssetSummary, AssetType } from "../../types";

/**
 * 整文件资产（Rules / Subagents / Hooks）的编辑状态机：选中 → 读真源 → 新建 / 保存 / 删除。
 * 三者只有表单形状与删除文案不同，其余逐字一样，收在这里才不会同一个 bug 修三遍。
 */
export function useFileAssetEditor<TForm extends { name: string }>({
  assetType,
  items,
  refresh,
  empty,
  parse,
  render,
  removeConfirm,
}: {
  assetType: AssetType;
  items: AssetSummary[];
  refresh: () => Promise<void>;
  empty: TForm;
  parse: (text: string) => TForm;
  render: (form: TForm) => string;
  removeConfirm: (id: string) => string;
}) {
  const [selected, setSelected] = useState<string | null>(null);
  const [form, setForm] = useState<TForm>(empty);
  const [newId, setNewId] = useState("");
  const [formError, setFormError] = useState<string | null>(null);

  // parse / render 必须是模块级函数引用：写成行内闭包会让这个 effect 每轮渲染都重读一次真源
  useEffect(() => {
    if (!selected || !items.some((item) => item.id === selected)) return;
    void api
      .readAsset(assetType, selected)
      .then((text) => {
        setForm(parse(text));
        setFormError(null);
      })
      .catch((e) => setFormError(String(e)));
  }, [assetType, items, parse, selected]);

  const create = () => {
    const id = newId.trim();
    if (!id) return;
    setSelected(id);
    setForm({ ...empty, name: id });
    setNewId("");
  };

  const save = async () => {
    if (!selected) return;
    try {
      await api.writeAsset(assetType, selected, render(form));
      setFormError(null);
      await refresh();
    } catch (e) {
      setFormError(String(e));
    }
  };

  const remove = async () => {
    if (!selected || !window.confirm(removeConfirm(selected))) return;
    await api.deleteAsset(assetType, selected);
    setSelected(null);
    await refresh();
  };

  return {
    selected,
    setSelected,
    form,
    setForm,
    newId,
    setNewId,
    formError,
    create,
    save,
    remove,
  };
}
