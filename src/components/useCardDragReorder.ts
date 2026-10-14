import { useEffect, useState } from "react";
import { KeyboardSensor, PointerSensor, useSensor, useSensors, type DragEndEvent, type DragStartEvent } from "@dnd-kit/core";
import { arrayMove } from "@dnd-kit/sortable";

interface CardDragReorder {
  sensors: ReturnType<typeof useSensors>;
  dragPreview: HTMLElement | null;
  onDragStart: (event: DragStartEvent) => void;
  onDragEnd: (event: DragEndEvent) => void;
  onDragCancel: () => void;
}

/** 供应商卡片拖拽重排的公共编排：传感器、拖拽态、arrayMove + 持久化。
 *  Codex / Claude Code 供应商列表共用；持久化由调用方注入（失败回滚 items 由本钩子负责，
 *  刷新等善后由调用方的 persist 闭包自理）。 */
export function useCardDragReorder<T extends { id: string }>(
  items: T[],
  setItems: (items: T[]) => void,
  persist: (previous: T[], next: T[]) => Promise<void>,
): CardDragReorder {
  const [dragPreview, setDragPreview] = useState<HTMLElement | null>(null);
  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 5 } }), useSensor(KeyboardSensor));

  useEffect(() => () => {
    document.body.classList.remove("drag-active");
  }, []);

  const blurDragHandle = () => {
    const activeElement = document.activeElement;
    if (activeElement instanceof HTMLElement && activeElement.classList.contains("drag-handle")) activeElement.blur();
  };

  const onDragEnd = (event: DragEndEvent) => {
    document.body.classList.remove("drag-active");
    blurDragHandle();
    setDragPreview(null);
    const { active, over } = event;
    if (!over || active.id === over.id) return;
    const oldIndex = items.findIndex((item) => item.id === String(active.id));
    const newIndex = items.findIndex((item) => item.id === String(over.id));
    if (oldIndex < 0 || newIndex < 0) return;
    const previous = items;
    const next = arrayMove(items, oldIndex, newIndex);
    setItems(next);
    void persist(previous, next);
  };

  const onDragStart = ({ active }: DragStartEvent) => {
    document.body.classList.add("drag-active");
    const source = [...document.querySelectorAll<HTMLElement>("[data-profile-id]")]
      .find((node) => node.dataset.profileId === String(active.id));
    // 原生复制当前卡片，按钮、布局和显示状态随源卡片一起更新，无需另一套预览 JSX。
    const preview = source?.cloneNode(true) as HTMLElement | undefined;
    if (source && preview) {
      const rect = active.rect.current.initial ?? source.getBoundingClientRect();
      preview.removeAttribute("data-profile-id");
      preview.removeAttribute("data-draggable");
      preview.classList.remove("opacity-0", "opacity-100");
      preview.classList.add("drag-dragging", "profile-drag-preview");
      Object.assign(preview.style, { transform: "", transition: "", width: `${rect.width}px`, height: `${rect.height}px` });
    }
    setDragPreview(preview ?? null);
  };

  const onDragCancel = () => {
    document.body.classList.remove("drag-active");
    blurDragHandle();
    setDragPreview(null);
  };

  return { sensors, dragPreview, onDragStart, onDragEnd, onDragCancel };
}
