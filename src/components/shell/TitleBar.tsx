import { useEffect, useState } from "react";
import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Maximize2, Minimize2, Minus, X } from "lucide-react";
import appIcon from "../../../src-tauri/icons/128x128@2x.png";
import { cn } from "../../lib/cn";

const BUTTON =
  "flex h-full w-11 items-center justify-center text-fg-2 transition-colors duration-150 hover:bg-subtle hover:text-fg-1";

/**
 * 自绘标题栏：整条可拖动，双击最大化由 Tauri 的 drag-region 脚本处理；
 * 按钮属于可点击元素，不会触发拖动。
 */
export function TitleBar() {
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!isTauri()) return;
    let active = true;
    let unlisten: (() => void) | undefined;
    const win = getCurrentWindow();
    const sync = async () => {
      try {
        const value = await win.isMaximized();
        if (active) setMaximized(value);
      } catch (error) {
        console.warn("读取窗口最大化状态失败：", error);
      }
    };
    void sync();
    void win
      .onResized(() => void sync())
      .then((dispose) => {
        if (active) unlisten = dispose;
        else dispose();
      })
      .catch((error) => console.warn("监听窗口尺寸失败：", error));
    return () => {
      active = false;
      unlisten?.();
    };
  }, []);

  const run = (action: () => Promise<void>) => {
    if (!isTauri()) return;
    void action().catch((error) => console.warn("窗口控制失败：", error));
  };

  const maximizeLabel = maximized ? "还原窗口" : "最大化窗口";

  return (
    <header
      data-tauri-drag-region="deep"
      className="flex h-[38px] shrink-0 select-none items-center border-b border-border bg-sidebar"
    >
      <div className="flex min-w-0 flex-1 items-center gap-2 px-3">
        <img
          src={appIcon}
          alt=""
          draggable={false}
          className="h-5 w-5 shrink-0 rounded-[5px]"
        />
        <span className="truncate text-[13px] font-medium text-fg-1">CC Switch X</span>
      </div>
      <div className="flex h-full shrink-0 items-center">
        <button
          type="button"
          className={BUTTON}
          aria-label="最小化窗口"
          title="最小化窗口"
          onClick={() => run(() => getCurrentWindow().minimize())}
        >
          <Minus size={16} aria-hidden />
        </button>
        <button
          type="button"
          className={BUTTON}
          aria-label={maximizeLabel}
          title={maximizeLabel}
          onClick={() =>
            run(async () => {
              const win = getCurrentWindow();
              await win.toggleMaximize();
              setMaximized(await win.isMaximized());
            })
          }
        >
          {maximized ? <Minimize2 size={14} aria-hidden /> : <Maximize2 size={14} aria-hidden />}
        </button>
        <button
          type="button"
          className={cn(BUTTON, "hover:bg-danger-soft hover:text-danger-text")}
          aria-label="关闭窗口"
          title="关闭窗口"
          onClick={() => run(() => getCurrentWindow().close())}
        >
          <X size={16} aria-hidden />
        </button>
      </div>
    </header>
  );
}
