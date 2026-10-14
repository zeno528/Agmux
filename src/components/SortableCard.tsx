import { GripVertical } from "lucide-react";
import { useLayoutEffect, useRef, type ReactNode } from "react";
import { useSortable } from "@dnd-kit/sortable";
import { CSS } from "@dnd-kit/utilities";

interface SortableCardProps {
  id: string;
  active: boolean;
  /** 整卡点击（Codex=进编辑页）。拖拽手柄与卡片内的操作行各自 stopPropagation。 */
  onClick: () => void;
  title: string;
  handleTitle: string;
  children: ReactNode;
}

/** 可拖拽排序的供应商卡片外壳：apple-group 卡片 + is-active 品牌渐变 + 左侧拖拽手柄。
 *  Codex / Claude Code 供应商列表共用；内容行与操作行由 children 提供。
 *  data-profile-id 供 DragOverlay 按 id 复制源卡片。 */
export default function SortableCard({ id, active, onClick, title, handleTitle, children }: SortableCardProps) {
  const sortable = useSortable({ id });
  const style = { transform: CSS.Transform.toString(sortable.transform), transition: sortable.transition };
  return (
    <article
      ref={sortable.setNodeRef}
      data-draggable
      data-profile-id={id}
      style={style}
      className={`apple-group${active ? " is-active brand-gradient-surface" : ""} group flex cursor-pointer select-none flex-col gap-4 px-5 py-[17.33px] sm:h-[84.67px] sm:flex-row sm:items-center sm:justify-between ${sortable.isDragging ? "pointer-events-none opacity-0" : "opacity-100"}`}
      title={title}
      onClick={onClick}
    >
      <span className="drag-handle -ml-5 -mr-4 grid shrink-0 place-items-center self-center rounded-md py-1 pl-3 pr-3 muted transition-colors hover:opacity-70 sm:self-stretch" title={handleTitle} aria-label={handleTitle} {...sortable.attributes} {...sortable.listeners} onClick={(event) => event.stopPropagation()}>
        <GripVertical className="h-4 w-4" strokeWidth={2} aria-hidden="true" />
      </span>
      {children}
    </article>
  );
}

/** 保留列表样式作用域；原生副本无 React 事件，inert 禁止焦点与交互。 */
export function CardDragPreview({ card }: { card: HTMLElement }) {
  const container = useRef<HTMLDivElement>(null);
  useLayoutEffect(() => {
    container.current?.replaceChildren(card);
    return () => { container.current?.replaceChildren(); };
  }, [card]);
  return <div ref={container} className="profile-list" inert />;
}
