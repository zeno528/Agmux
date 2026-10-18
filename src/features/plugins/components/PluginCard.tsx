import { useEffect, useState, type ReactNode } from "react";
import { getCachedPluginIcon, loadPluginIcon } from "../../../app/managementDataCache";

function usePluginIcon(iconPath?: string | null): string | null {
  const [loaded, setLoaded] = useState(() => ({ path: iconPath, src: getCachedPluginIcon(iconPath) }));
  useEffect(() => {
    if (!iconPath || getCachedPluginIcon(iconPath)) return;
    let alive = true;
    void loadPluginIcon(iconPath).catch(() => null).then((src) => {
      if (alive) setLoaded({ path: iconPath, src });
    });
    return () => {
      alive = false;
    };
  }, [iconPath]);
  return getCachedPluginIcon(iconPath) ?? (loaded.path === iconPath ? loaded.src : null);
}

/** 看板卡片：图标 + 名称/来源 + 右上主操作，描述两行截断，底部元信息与次要操作分区。 */
export function PluginCard({ name, source, description, iconPath, action, meta, foot, onOpen }: {
  name: string;
  source?: ReactNode;
  description?: ReactNode;
  /** 插件目录内的本地图标路径；缺省、读取失败或解码失败回落首字母。 */
  iconPath?: string | null;
  action?: ReactNode;
  meta?: ReactNode;
  foot?: ReactNode;
  onOpen?: () => void;
}) {
  const iconSrc = usePluginIcon(iconPath);
  const [failedPath, setFailedPath] = useState<string | null>(null);
  const effectiveSrc = iconSrc && failedPath !== iconPath ? iconSrc : null;
  return (
    <div className="plugin-card">
      <div className="plugin-card-head">
        <span className="plugin-card-icon" aria-hidden="true">
          {effectiveSrc ? (
            <img className="plugin-card-icon-img" src={effectiveSrc} alt=""
              onError={() => setFailedPath(iconPath ?? null)}
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
