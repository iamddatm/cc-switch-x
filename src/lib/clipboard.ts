/**
 * 剪贴板写入的唯一入口：不支持就抛一句能直接给用户看的中文，成功与失败怎么播报由调用方决定
 * （Sessions 页收成行内状态，Agents 页走面板反馈条）。
 */
export async function copyToClipboard(text: string): Promise<void> {
  if (!navigator.clipboard?.writeText) {
    throw new Error("当前环境不支持剪贴板");
  }
  await navigator.clipboard.writeText(text);
}
