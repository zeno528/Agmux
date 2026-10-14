import type { ReactNode } from "react";
import { providerIconUrl } from "../icons";

interface ProfileIconTileProps {
  name: string;
  icon: string | null;
  size?: "xs" | "sm" | "fill" | "lg";
  /** 悬停容器时居中浮现的覆盖动作（如打开官网）；提供时 Logo 悬停退隐让位。 */
  overlay?: ReactNode;
}

const sizes = {
  xs: { tile: "h-6 w-6 rounded-lg", image: "h-3.5 w-3.5", text: "meta-xs" },
  sm: { tile: "h-10 w-10 rounded-[12px]", image: "h-6 w-6", text: "text-sm" },
  fill: { tile: "h-full w-full rounded-[16px]", image: "h-8 w-8", text: "text-2xl" },
  lg: { tile: "h-[76px] w-[76px] rounded-[22px]", image: "h-10 w-10", text: "text-xl" },
} as const;

export function ProfileIconTile({ name, icon, size = "sm", overlay }: ProfileIconTileProps) {
  const current = sizes[size];
  const iconUrl = providerIconUrl(icon);
  return (
    <span className={`group/tile relative grid shrink-0 place-items-center bg-(--provider-icon-bg) ${current.tile}`}>
      {iconUrl ? <img src={iconUrl} alt="" aria-hidden="true" className={`${current.image} ${overlay ? "transition-[opacity,transform] duration-150 group-hover/tile:scale-90 group-hover/tile:opacity-20" : ""}`} /> : <span aria-hidden="true" className={`font-bold text-accent ${current.text}`}>{name.charAt(0)}</span>}
      {overlay}
    </span>
  );
}
