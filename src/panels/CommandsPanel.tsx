import { AgentMatrix } from "../components/AgentMatrix";
import { AssetDriftHeader } from "../components/assets/AssetDriftHeader";
import {
  AssetEditorLayout,
  AssetEditorPane,
  AssetSourceList,
} from "../components/assets/AssetEditor";
import { useFileAssetEditor } from "../components/assets/useFileAssetEditor";
import { useSourceScan } from "../components/assets/useSourceScan";
import { Textarea } from "../components/ui/Input";
import { useAgents } from "../hooks/useAgents";
import { useAssets } from "../hooks/useAssets";
import { useDrift } from "../hooks/useDrift";
import { api } from "../lib/api";

/** 命令的真源就是整份 markdown：frontmatter 与正文一起原样编辑。 */
interface CommandForm {
  name: string;
  body: string;
}

const EMPTY: CommandForm = { name: "", body: "" };

const parseCommand = (text: string): CommandForm => ({ name: "", body: text });
const renderCommand = (form: CommandForm) => form.body;

export function CommandsPanel() {
  const { items, error, refresh } = useAssets("command");
  const { agents, refresh: refreshAgents } = useAgents();
  const drift = useDrift();
  const scan = useSourceScan("command", agents, (agentId) => api.scanCommands(agentId), refresh);
  const { selected, setSelected, form, setForm, newId, setNewId, formError, create, save, remove } =
    useFileAssetEditor<CommandForm>({
      assetType: "command",
      items,
      refresh,
      empty: EMPTY,
      parse: parseCommand,
      render: renderCommand,
      removeConfirm: (id) =>
        `删除真源里的命令 ${id}？已落盘的文件要先在下面取消勾选。`,
    });

  return (
    <AssetEditorLayout>
      <AssetSourceList
        title="命令"
        items={items}
        selected={selected}
        onSelect={setSelected}
        listError={error}
        emptyHint="真源里还没有命令。可以在下面新建，或从 Claude Code 的 commands 目录扫描进来。"
        newIdLabel="新命令 ID"
        newIdValue={newId}
        onNewIdChange={setNewId}
        onCreate={create}
        scan={scan}
      />
      <AssetEditorPane
        selected={selected}
        formError={formError}
        onSave={() => void save()}
        onDelete={() => void remove()}
        notSelectedHint="选择左侧命令进行编辑。"
        header={<AssetDriftHeader assetType="command" selected={selected} drift={drift} />}
      >
        <label className="block text-body">
          文件内容（markdown，可带 frontmatter）
          <Textarea
            aria-label="命令内容"
            rows={16}
            className="mt-1 font-mono text-caption"
            value={form.body}
            onChange={(e) => setForm({ ...form, body: e.target.value })}
          />
        </label>
        <p className="mt-2 text-caption text-fg-3">
          文件名就是命令名（例如 review.md → /review）。内容按原字节写入目标工具，
          不重排 frontmatter；工具侧的任何手改都算漂移，先裁决再同步。
        </p>
        {selected && (
          <AgentMatrix
            assetType="command"
            id={selected}
            agents={agents}
            onChanged={() => {
              void refresh();
              void refreshAgents();
              void drift.refresh();
            }}
          />
        )}
      </AssetEditorPane>
    </AssetEditorLayout>
  );
}
