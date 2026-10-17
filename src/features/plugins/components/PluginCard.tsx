import { useEffect, useState, type ReactNode } from "react";
import { api } from "../../../api";

/// 本地图标 → data URL 的会话级缓存：同路径只请求一次（后端是一次本地文件读，失败回落字母）。
const iconCache = new Map<string, Promise<string | null>>();

function usePluginIcon(iconPath?: string | null): string | null {
  const [src, setSrc] = useState<string | null>(null);
  useEffect(() => {
    if (!iconPath) {
      setSrc(null);
      return;
    }
    let alive = true;
    const pending = iconCache.get(iconPath)
      ?? api.pluginIcon(iconPath).catch(() => null);
    iconCache.set(iconPath, pending);
    void pending.then((url) => {
      if (alive) setSrc(url);
    });
    return () => {
      alive = false;
    };
  }, [iconPath]);
  return src;
}

/** 看板卡片：图标 + 名称/来源 + 右上主操作，描述两行截断，底部元信息与次要操作分区。 */
export function PluginCard({ name, source, description, iconPath, action, status, meta, foot, onOpen }: {
  name: string;
  source?: ReactNode;
  description?: ReactNode;
  /** 插件目录内的本地图标路径；缺省、读取失败或解码失败回落首字母。 */
  iconPath?: string | null;
  action?: ReactNode;
  /** 标题旁的状态灯（如已禁用），不占元信息行的横向空间。 */
  status?: ReactNode;
  meta?: ReactNode;
  foot?: ReactNode;
  onOpen?: () => void;
}) {
  const iconSrc = usePluginIcon(iconPath);
  const [iconFailed, setIconFailed] = useState(false);
  useEffect(() => {
    setIconFailed(false);
  }, [iconPath]);
  const effectiveSrc = iconSrc && !iconFailed ? iconSrc : null;
  return (
    <div className="plugin-card">
      <div className="plugin-card-head">
        <span className="plugin-card-icon" aria-hidden="true">
          {effectiveSrc ? (
            <img className="plugin-card-icon-img" src={effectiveSrc} alt="" loading="lazy"
              onError={() => setIconFailed(true)}
            />
          ) : name.charAt(0).toUpperCase()}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex min-w-0 items-center gap-1.5">
            {onOpen ? (
              <button type="button" className="plugin-card-name plugin-card-name--link truncate" title={name} onClick={onOpen}>{name}</button>
            ) : (
              <span className="plugin-card-name block truncate" title={name}>{name}</span>
            )}
            {status}
          </div>
          {source ? <div className="muted meta-xs truncate">{source}</div> : null}
        </div>
        {action}
      </div>
      {description ? <p className="plugin-card-desc muted">{description}</p> : null}
      {meta || foot ? (
        <div className="plugin-card-foot">
          <div className="flex min-w-0 flex-wrap items-center gap-1.5">{meta}</div>
          <div className="flex shrink-0 items-center gap-1">{foot}</div>
        </div>
      ) : null}
    </div>
  );
}
