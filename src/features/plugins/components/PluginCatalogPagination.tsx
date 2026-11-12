import { useEffect, useLayoutEffect, useState } from "react";
import { useTranslation } from "react-i18next";

export function pluginCatalogRange(total: number, columns: number, offset: number) {
  const pageSize = columns * Math.max(1, Math.floor(24 / columns));
  const lastStart = Math.floor(Math.max(0, total - 1) / pageSize) * pageSize;
  const start = Math.min(Math.floor(offset / pageSize) * pageSize, lastStart);
  return { start, end: Math.min(start + pageSize, total), pageSize };
}

export function usePluginCatalogPage(total: number, query: string) {
  const [grid, setGrid] = useState<HTMLDivElement | null>(null);
  const [columns, setColumns] = useState(1);
  const [offset, setOffset] = useState(0);

  useLayoutEffect(() => {
    if (!grid) return;
    const measure = () => {
      if (grid.clientWidth === 0) return;
      const tracks = getComputedStyle(grid).gridTemplateColumns;
      if (tracks !== "none") setColumns(tracks.trim().split(/\s+/).length);
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(grid);
    return () => observer.disconnect();
  }, [grid]);

  useEffect(() => { setOffset(0); }, [query]);

  const { start, end, pageSize } = pluginCatalogRange(total, columns, offset);
  const move = (direction: -1 | 1) => {
    setOffset(start + direction * pageSize);
    grid?.focus({ preventScroll: true });
    grid?.scrollIntoView({ block: "start" });
  };
  return { start, end, setGrid, move, columns };
}

export function PluginCatalogPagination({ start, end, total, loading, move }: {
  start: number;
  end: number;
  total: number;
  loading: boolean;
  move: (direction: -1 | 1) => void;
}) {
  const { t } = useTranslation("plugins");
  if (loading || (start === 0 && end === total)) return null;
  return (
    <div className="mt-3 flex flex-wrap items-center justify-between gap-2">
      <span className="muted text-sm" role="status">{t("board.pageRange", { first: start + 1, last: end, total })}</span>
      <div className="ml-auto flex items-center gap-2">
        <button type="button" className="apple-action-button apple-action-button--small" disabled={start === 0} onClick={() => move(-1)}>
          {t("board.previousPage")}
        </button>
        <button type="button" className="apple-action-button apple-action-button--small" disabled={end === total} onClick={() => move(1)}>
          {t("board.nextPage")}
        </button>
      </div>
    </div>
  );
}
