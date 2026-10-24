import { useEffect, useState } from "react";
import { Eye, EyeOff, Globe, Search, TestTube2, X } from "lucide-react";
import { Button } from "../components/ui/Button";
import { IconButton } from "../components/ui/IconButton";
import { Input } from "../components/ui/Input";
import { Notice } from "../components/ui/Notice";
import { PageHeader } from "../components/ui/PageHeader";
import { api } from "../lib/api";
import type { DetectedProxy } from "../types";

export function NetworkPanel() {
  const [url, setUrl] = useState("");
  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [passwordDirty, setPasswordDirty] = useState(false);
  const [hasPassword, setHasPassword] = useState(false);
  const [showPassword, setShowPassword] = useState(false);
  const [dirty, setDirty] = useState(false);
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [testing, setTesting] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [detected, setDetected] = useState<DetectedProxy[]>([]);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    void api
      .getProxySettings()
      .then((value) => {
        if (cancelled) return;
        setUrl(value.url ?? "");
        setUsername(value.username ?? "");
        setHasPassword(value.hasPassword);
        setPassword("");
        setPasswordDirty(false);
        setDirty(false);
      })
      .catch((e) => {
        if (!cancelled) setError(String(e));
      })
      .finally(() => {
        if (!cancelled) setLoading(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const markDirty = () => {
    setDirty(true);
    setMessage(null);
    setError(null);
  };

  const save = async () => {
    setSaving(true);
    setError(null);
    setMessage(null);
    try {
      const view = await api.setProxySettings(
        url.trim(),
        username.trim(),
        passwordDirty ? password : null,
      );
      setHasPassword(view.hasPassword);
      setPassword("");
      setPasswordDirty(false);
      setDirty(false);
      setMessage(url.trim() ? "代理设置已保存。" : "已切换为直连。");
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  };

  const test = async () => {
    if (!url.trim()) {
      setError("请先填写代理地址");
      return;
    }
    setTesting(true);
    setError(null);
    setMessage(null);
    try {
      const result = await api.testProxySettings(
        url.trim(),
        username.trim(),
        passwordDirty ? password : null,
      );
      if (result.success) {
        setMessage(`代理可用，延迟 ${result.latencyMs} ms。`);
      } else {
        setError(`代理连接失败：${result.error ?? "未知原因"}`);
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setTesting(false);
    }
  };

  const scan = async () => {
    setScanning(true);
    setError(null);
    setMessage(null);
    try {
      const found = await api.scanLocalProxies();
      setDetected(found);
      if (found.length === 0) {
        setMessage("没有在常见端口发现本地代理。");
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setScanning(false);
    }
  };

  const clear = () => {
    setUrl("");
    setUsername("");
    setPassword("");
    setPasswordDirty(true);
    setHasPassword(false);
    setDetected([]);
    setDirty(true);
    setError(null);
    setMessage(null);
  };

  const clearPassword = () => {
    setPassword("");
    setPasswordDirty(true);
    setHasPassword(false);
    setDirty(true);
    setMessage(null);
  };

  const onEnter = (event: React.KeyboardEvent<HTMLInputElement>) => {
    if (event.key === "Enter" && dirty && !saving) void save();
  };

  return (
    <div className="flex h-full min-h-0 flex-col">
      <PageHeader icon={Globe} title="网络" />
      <div className="min-h-0 flex-1 overflow-auto px-6 py-5">
        <div className="max-w-3xl">
          <section className="rounded-panel border border-border bg-surface p-4">
            <div className="flex items-center gap-2">
              <h3 className="text-strong text-fg-1">全局出站代理</h3>
              <span
                className="text-fg-3"
                title="代理地址支持 http / https / socks5 / socks5h；留空表示直连。"
              >
                <Globe size={14} aria-hidden />
              </span>
            </div>
            <p className="mt-1 text-caption text-fg-2">
              所有出站请求（Skills 下载、skills.sh 搜索、应用更新）都会经过此代理；留空表示直连。
            </p>

            {loading ? (
              <p className="mt-4 text-caption text-fg-3">正在读取代理设置…</p>
            ) : (
              <div className="mt-4 space-y-3">
                <div className="flex gap-2">
                  <Input
                    aria-label="代理地址"
                    className="flex-1 font-mono"
                    placeholder="http://127.0.0.1:7890 / socks5://127.0.0.1:1080"
                    value={url}
                    onChange={(event) => {
                      setUrl(event.target.value);
                      markDirty();
                    }}
                    onKeyDown={onEnter}
                  />
                  <IconButton
                    label="扫描本地代理"
                    size="md"
                    disabled={scanning}
                    onClick={() => void scan()}
                  >
                    <Search size={15} aria-hidden />
                  </IconButton>
                  <IconButton
                    label="测试代理"
                    size="md"
                    disabled={!url.trim() || testing}
                    onClick={() => void test()}
                  >
                    <TestTube2 size={15} aria-hidden />
                  </IconButton>
                  <IconButton
                    label="清空代理输入"
                    size="md"
                    disabled={!url && !username && !hasPassword && !password}
                    onClick={clear}
                  >
                    <X size={15} aria-hidden />
                  </IconButton>
                  <Button
                    variant="primary"
                    disabled={!dirty || saving}
                    loading={saving}
                    onClick={() => void save()}
                  >
                    保存
                  </Button>
                </div>

                <div className="grid grid-cols-2 gap-2">
                  <Input
                    aria-label="用户名（可选）"
                    placeholder="用户名（可选）"
                    value={username}
                    onChange={(event) => {
                      setUsername(event.target.value);
                      markDirty();
                    }}
                    onKeyDown={onEnter}
                  />
                  <div className="relative">
                    <Input
                      aria-label="密码（可选）"
                      className="pe-8 font-mono"
                      type={showPassword ? "text" : "password"}
                      placeholder={
                        hasPassword ? "已保存密码，留空保持不变" : "密码（可选）"
                      }
                      value={password}
                      onChange={(event) => {
                        setPassword(event.target.value);
                        setPasswordDirty(true);
                        markDirty();
                      }}
                      onKeyDown={onEnter}
                    />
                    <IconButton
                      label={showPassword ? "隐藏密码" : "显示密码"}
                      className="absolute end-0.5 top-0.5"
                      onClick={() => setShowPassword((value) => !value)}
                    >
                      {showPassword ? (
                        <EyeOff size={14} aria-hidden />
                      ) : (
                        <Eye size={14} aria-hidden />
                      )}
                    </IconButton>
                  </div>
                </div>

                {hasPassword && !passwordDirty && (
                  <p className="text-caption text-fg-3">
                    密码已保存在本机密钥文件。
                    <button
                      type="button"
                      className="ms-1 text-action-text hover:underline"
                      onClick={clearPassword}
                    >
                      清除密码
                    </button>
                  </p>
                )}

                {detected.length > 0 && (
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="text-caption text-fg-3">检测到：</span>
                    {detected.map((item) => (
                      <Button
                        key={item.url}
                        size="sm"
                        className="font-mono"
                        onClick={() => {
                          setUrl(item.url);
                          markDirty();
                        }}
                      >
                        {item.url}
                      </Button>
                    ))}
                  </div>
                )}

                {message && <Notice tone="success">{message}</Notice>}
                {error && <Notice tone="danger">{error}</Notice>}
              </div>
            )}
          </section>
        </div>
      </div>
    </div>
  );
}
