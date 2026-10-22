import { useEffect, useState } from "react";
import { JsonEditor } from "../components/JsonEditor";
import { useAssets } from "../hooks/useAssets";
import { api } from "../lib/api";

export function McpPanel() {
  const { items, error, refresh } = useAssets("mcp");
  const [selected, setSelected] = useState<string | null>(null);
  const [content, setContent] = useState("");
  const [newId, setNewId] = useState("");
  const [saveError, setSaveError] = useState<string | null>(null);

  useEffect(() => {
    if (!selected) {
      setContent("");
      return;
    }
    // 新建但尚未落盘的 ID 不在列表里，此时保留编辑器里的模板内容
    if (!items.some((item) => item.id === selected)) return;
    void api
      .readAsset("mcp", selected)
      .then(setContent)
      .catch((e) => setSaveError(String(e)));
  }, [selected, items]);

  const create = () => {
    const id = newId.trim();
    if (!id) return;
    setSelected(id);
    setContent(JSON.stringify({ id, name: id, transport: "stdio", command: "", args: [] }, null, 2));
    setNewId("");
  };

  const save = async () => {
    if (!selected) return;
    try {
      JSON.parse(content);
    } catch {
      setSaveError("JSON 无效");
      return;
    }
    try {
      await api.writeAsset("mcp", selected, content);
      setSaveError(null);
      await refresh();
    } catch (e) {
      setSaveError(String(e));
    }
  };

  const remove = async () => {
    if (!selected || !window.confirm("确定删除该 MCP 服务器？")) return;
    await api.deleteAsset("mcp", selected);
    setSelected(null);
    await refresh();
  };

  return (
    <div className="grid grid-cols-[240px_1fr] gap-6">
      <section>
        <h2 className="mb-3 text-xl font-semibold">MCP</h2>
        {error && <p className="text-sm text-red-600">{error}</p>}
        <ul className="space-y-1">
          {items.map((item) => (
            <li key={item.id}>
              <button
                type="button"
                className="w-full rounded px-2 py-1 text-left hover:bg-slate-100"
                onClick={() => setSelected(item.id)}
              >
                {item.name}
              </button>
            </li>
          ))}
        </ul>
        <div className="mt-4 flex gap-2">
          <input
            aria-label="新 MCP ID"
            className="min-w-0 flex-1 rounded border border-slate-300 px-2 py-1 text-sm"
            placeholder="新 MCP ID"
            value={newId}
            onChange={(e) => setNewId(e.target.value)}
          />
          <button
            type="button"
            className="rounded border border-slate-300 px-2 py-1 text-sm"
            onClick={create}
          >
            新建
          </button>
        </div>
      </section>
      <section>
        {selected ? (
          <>
            <div className="mb-3 flex gap-2">
              <button
                type="button"
                className="rounded bg-slate-800 px-3 py-1.5 text-sm text-white"
                onClick={save}
              >
                保存
              </button>
              <button
                type="button"
                className="rounded border border-red-300 px-3 py-1.5 text-sm text-red-700"
                onClick={remove}
              >
                删除
              </button>
            </div>
            {saveError && <p className="mb-2 text-sm text-red-600">{saveError}</p>}
            <JsonEditor value={content} onChange={setContent} />
          </>
        ) : (
          <p className="text-sm text-slate-500">选择左侧 MCP 服务器进行编辑。</p>
        )}
      </section>
    </div>
  );
}
