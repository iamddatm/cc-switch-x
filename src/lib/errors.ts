/** 统一错误文案：前端抛的 Error 取 message，后端/插件返回的字符串原样保留。 */
export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}