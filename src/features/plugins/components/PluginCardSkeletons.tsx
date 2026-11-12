export function PluginCardSkeletons({ count = 3, installed = false }: { count?: number; installed?: boolean }) {
  return Array.from({ length: count }, (_, index) => (
    <div key={index} className="plugin-card pointer-events-none" aria-hidden="true">
      <div className="plugin-card-head">
        <span className="h-[2.125rem] w-[2.125rem] shrink-0 rounded-lg bg-(--tile-bg)" />
        <div className="min-w-0 flex-1 space-y-1.5 py-1">
          <div className="h-3 w-2/3 rounded-full bg-(--tile-bg)" />
          <div className="h-2 w-2/5 rounded-full bg-(--tile-bg)" />
        </div>
      </div>
      {installed ? (
        <div className="plugin-card-foot">
          <span className="h-4 w-1/3 rounded-full bg-(--tile-bg)" />
        </div>
      ) : (
        <div className="plugin-card-desc space-y-2 py-1">
          <div className="h-2.5 w-full rounded-full bg-(--tile-bg)" />
          <div className="h-2.5 w-4/5 rounded-full bg-(--tile-bg)" />
        </div>
      )}
    </div>
  ));
}
