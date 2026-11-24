import { Check } from "lucide-react";
import { Popover } from "./Popover";

export interface FilterMenuGroup {
  key: string;
  title: string;
  value: string;
  options: { value: string; label: string }[];
  onChange: (value: string) => void;
  /** 该组是否偏离默认值；触发器据此显示「筛选 · N」。 */
  active?: boolean;
}

/**
 * 一组单选筛选的轻量下拉：所有组收在一个「筛选」按钮里，
 * 避免状态 / 来源 / Agent 各自占一个下拉框把工具栏撑满。
 */
export function FilterMenu({
  groups,
  triggerLabel,
}: {
  groups: FilterMenuGroup[];
  /** 排序这类自身就是入口的菜单可直接显示当前选项。 */
  triggerLabel?: string;
}) {
  const activeCount = groups.filter((group) => group.active).length;
  const label = triggerLabel ?? (activeCount > 0 ? `筛选 · ${activeCount}` : "筛选");

  return (
    <Popover label={label} variant="quiet" active={activeCount > 0} panelClassName="max-h-96">
      {(close) => (
        <div>
          {groups.map((group, index) => (
            <div key={group.key} className={index > 0 ? "mt-1 border-t border-border pt-1" : undefined}>
              <p className="px-1.5 pb-0.5 pt-1 text-caption text-fg-3">{group.title}</p>
              {group.options.map((option) => {
                const checked = group.value === option.value;
                return (
                  <button
                    key={option.value}
                    type="button"
                    role="menuitemradio"
                    aria-checked={checked}
                    onClick={() => {
                      group.onChange(option.value);
                      close();
                    }}
                    className="flex w-full items-center gap-2 rounded-control px-1.5 py-1 text-left text-sm text-fg-1 hover:bg-subtle focus:bg-subtle focus:outline-none"
                  >
                    <span className="flex w-4 shrink-0 justify-center">
                      {checked && <Check size={13} aria-hidden />}
                    </span>
                    <span className="truncate">{option.label}</span>
                  </button>
                );
              })}
            </div>
          ))}
        </div>
      )}
    </Popover>
  );
}
