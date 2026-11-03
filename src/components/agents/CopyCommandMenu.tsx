import { Copy } from "lucide-react";
import { Menu } from "../ui/Menu";
import { installCommandsFor } from "./installCommands";

/**
 * 复制某个 agent 的安装 / 升级命令。
 *
 * 刻意只做「复制」：本应用不代跑安装脚本，也不 `npm i -g`——那类命令会改用户机器上
 * 我们管不着的全局环境，装什么、什么时候装由用户在终端里自己决定。
 * 结果交给面板的反馈条播报，行内不再叠一层状态。
 */
export function CopyCommandMenu({
  agentId,
  onResult,
}: {
  agentId: string;
  /** 成功与失败都走这一条，由面板决定是绿色提示还是红色报错。 */
  onResult: (message: string, failed: boolean) => void;
}) {
  const commands = installCommandsFor(agentId);
  if (commands.length === 0) return null;

  const copy = async (label: string, command: string) => {
    try {
      if (!navigator.clipboard?.writeText) {
        throw new Error("当前环境不支持剪贴板");
      }
      await navigator.clipboard.writeText(command);
      onResult(`已复制${label}：${command}`, false);
    } catch (cause) {
      onResult(String(cause), true);
    }
  };

  return (
    <Menu
      label="复制安装 / 升级命令"
      trigger={<Copy size={14} aria-hidden />}
      items={commands.map((item) => ({
        label: item.label,
        onSelect: () => void copy(item.label, item.command),
      }))}
    />
  );
}
