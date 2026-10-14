import { RotateCcw } from "lucide-react";
import { useId, useRef, type CSSProperties, type PointerEvent as ReactPointerEvent, type ReactNode } from "react";
import { useTranslation } from "react-i18next";

export const claudeEffortLevels = ["low", "medium", "high", "xhigh", "max"] as const;
export const codexEffortLevels = [...claudeEffortLevels, "ultra"] as const;

/** 拨动手感：指针拉过档距的 0.45 倍才改目标；圆头按 40ms 时间常数自己滑到目标档位。
 * 拉伸未过释放半径时圆头以 0.15 的弹性系数向指针轻微倾斜，松手弹回原档。 */
const DETENT_RELEASE = 0.45;
const CHASE_TAU = 0.04;
const RUBBER_GIVE = 0.15;

export function isOverEffortThumb(rect: Pick<DOMRect, "left" | "top" | "width" | "height">, x: number, y: number) {
  return Math.hypot(x - rect.left - rect.width / 2, y - rect.top - rect.height / 2) <= rect.width / 2;
}

export function ReasoningEffortSlider({ value, levels, onChange, onCommit, onReset, children, leading, fast = false, disabled = false }: {
  value: string;
  levels: readonly string[];
  onChange: (value: string) => void;
  onCommit?: (value: string) => void;
  onReset?: () => void;
  children?: ReactNode;
  leading?: ReactNode;
  fast?: boolean;
  disabled?: boolean;
}) {
  const { t } = useTranslation("profiles");
  const labelId = useId();
  const index = levels.indexOf(value);
  const labelLevels = [...new Set(["", ...levels, value])];
  const showParticles = fast || value === "ultra";
  // 拖动是「拨动」模型：圆头永远只落在档位上，不贴指针落点。锚定在当前档位时指针拉不动它，
  // 只以弹性系数微微倾斜（皮筋拉伸）；拉过释放半径就把目标设为指针最近的档位，圆头按时间
  // 常数自己滑过去，途中指针继续拨动会平滑改目标。位置由 rAF 循环直写内联变量，不经 React
  // 状态（值仍由原生 range 按档位离散提交；释放半径小于 0.5 档，圆头先动、值后跨档；
  // 拉伸中松手时值未跨档，移除内联变量即由 CSS 过渡弹回原档）。
  const dragGeometry = useRef<{ track: HTMLElement; rect: DOMRect; thumbWidth: number; target: number; lean: number; visual: number; frame: number } | null>(null);
  const dragToPointer = (event: ReactPointerEvent<HTMLDivElement>) => {
    const geometry = dragGeometry.current;
    if (!geometry) return;
    const span = geometry.rect.width - geometry.thumbWidth;
    const stops = Math.max(1, levels.length - 1);
    const pointer = span > 0 ? Math.min(1, Math.max(0, (event.clientX - geometry.rect.left - geometry.thumbWidth / 2) / span)) * stops : 0;
    const stretch = pointer - geometry.target;
    if (Math.abs(stretch) > DETENT_RELEASE) {
      geometry.target = Math.min(stops, Math.max(0, Math.round(pointer)));
      geometry.lean = 0;
    } else {
      geometry.lean = stretch * RUBBER_GIVE;
    }
  };
  const startChase = () => {
    const geometry = dragGeometry.current!;
    let last = performance.now();
    const step = (now: number) => {
      const g = dragGeometry.current;
      if (g !== geometry) return;
      const goal = g.target + g.lean;
      if (g.visual !== goal) {
        g.visual += (goal - g.visual) * (1 - Math.exp(-Math.min(0.05, (now - last) / 1000) / CHASE_TAU));
        if (Math.abs(g.visual - goal) < 0.002) g.visual = goal;
        g.track.style.setProperty("--effort-progress", (g.visual / Math.max(1, levels.length - 1)).toFixed(4));
      }
      last = now;
      g.frame = requestAnimationFrame(step);
    };
    geometry.frame = requestAnimationFrame(step);
  };
  const endDrag = () => {
    const geometry = dragGeometry.current;
    dragGeometry.current = null;
    if (!geometry) return;
    cancelAnimationFrame(geometry.frame);
    // :active 在松手帧结束后失效，过渡恢复；此刻移除内联变量，圆头从当前位置平滑滑到档位中心。
    requestAnimationFrame(() => geometry.track.style.removeProperty("--effort-progress"));
  };
  return (
    <div className="reasoning-effort-slider" data-selected={index >= 0} style={{
      "--effort-color": value === "ultra" ? "var(--reasoning-ultra)" : value ? "var(--accent)" : "var(--text-secondary)",
      "--effort-progress": index < 0 ? 0 : index / Math.max(1, levels.length - 1),
      "--effort-energy": index < 0 ? 0 : 0.3 + index / Math.max(1, levels.length - 1) * 0.7,
      "--effort-particle-duration": value === "ultra" ? "0.9s" : "1s",
    } as CSSProperties}>
      <div className="reasoning-effort-slider__header">
        <div className="reasoning-effort-slider__leading">{leading}</div>
        <div className="reasoning-effort-slider__heading">
          <strong id={labelId} className="field-label">
            <span className="sr-only">{t("modelSelection.effort")}: {value ? value[0].toUpperCase() + value.slice(1) : t("modelSelection.default")}</span>
            <span className="reasoning-effort-slider__label" aria-hidden="true">
              {labelLevels.map((level) => (
                <span key={level} className="reasoning-effort-slider__label-step" data-current={level === value}
                  style={{ transform: `translateY(${(levels.indexOf(level) - index) * 50}%)` }}>
                  {level ? level[0].toUpperCase() + level.slice(1) : t("modelSelection.default")}
                </span>
              ))}
            </span>
          </strong>
          {children}
          {fast && value === "ultra" && <span className="reasoning-effort-slider__quota-hint" aria-hidden="true">{t("modelSelection.ultraQuotaHint")}</span>}
        </div>
        <button type="button" className="apple-icon-button" disabled={disabled || (!value && !onReset)}
          title={t("modelSelection.resetEffort")} aria-label={t("modelSelection.resetEffort")}
          onClick={() => { if (onReset) onReset(); else { onChange(""); onCommit?.(""); } }}><RotateCcw size={16} aria-hidden="true" /></button>
      </div>
      <div className="reasoning-effort-slider__track"
        onPointerDown={(event) => {
          const track = event.currentTarget;
          track.style.removeProperty("--effort-progress");
          // 只有按在隐形原生 range（圆头拖动）才连续跟踪；点圆点/轨道跳档不写内联变量，
          // 始终走 CSS 过渡，避免闪跳。
          if (disabled || !levels.length || !(event.target instanceof HTMLInputElement)) return;
          const thumb = track.querySelector<HTMLElement>(".reasoning-effort-slider__thumb");
          const rect = track.getBoundingClientRect();
          const thumbWidth = thumb?.getBoundingClientRect().width ?? 0;
          const stops = Math.max(1, levels.length - 1);
          const span = rect.width - thumbWidth;
          const pointer = span > 0 ? Math.min(1, Math.max(0, (event.clientX - rect.left - thumbWidth / 2) / span)) * stops : 0;
          const anchor = Math.max(0, levels.indexOf(value));
          const near = Math.abs(pointer - anchor) <= DETENT_RELEASE;
          const target = near ? anchor : Math.min(stops, Math.max(0, Math.round(pointer)));
          // 立即以内联变量接管渲染：压住原生按下的跳档，远按由追逐循环弹过去，近按锚住并随拉伸微倾。
          dragGeometry.current = { track, rect, thumbWidth, target, lean: near ? (pointer - anchor) * RUBBER_GIVE : 0, visual: anchor, frame: 0 };
          track.style.setProperty("--effort-progress", (anchor / stops).toFixed(4));
          startChase();
        }}
        onPointerMove={(event) => {
          if (dragGeometry.current) { dragToPointer(event); return; }
          // 非拖动的悬停探测：只读圆头几何，拖动中完全跳过（放大态已由 input:active 提供）。
          const track = event.currentTarget;
          const thumb = track.querySelector<HTMLElement>(".reasoning-effort-slider__thumb")!;
          track.toggleAttribute("data-thumb-hover", event.pointerType === "mouse"
            && isOverEffortThumb(thumb.getBoundingClientRect(), event.clientX, event.clientY));
        }}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
        onPointerLeave={(event) => {
          event.currentTarget.removeAttribute("data-thumb-hover");
        }}>
        <span className="reasoning-effort-slider__fill" aria-hidden="true">
          {fast && value === "ultra" && <span className="reasoning-effort-slider__gradient" />}
          {showParticles && <span className="reasoning-effort-slider__particles" data-floating={!fast} />}
        </span>
        <div className="reasoning-effort-slider__stops">
          {levels.map((level, stop) => (!showParticles || stop > index) && (
            <button key={level} type="button" className="reasoning-effort-slider__stop" tabIndex={-1} disabled={disabled}
              aria-label={`${t("modelSelection.effort")}: ${level}`} title={level} aria-pressed={value === level}
              data-filled={stop <= index}
              style={{ "--effort-progress": stop / Math.max(1, levels.length - 1) } as CSSProperties}
              onClick={() => { onChange(level); onCommit?.(level); }}><span aria-hidden="true" /></button>
          ))}
        </div>
        <span className="reasoning-effort-slider__thumb-rail" aria-hidden="true"><span className="reasoning-effort-slider__thumb" /></span>
        <input type="range" min={0} max={Math.max(0, levels.length - 1)} step={1}
        value={Math.max(0, index)} disabled={levels.length === 0} aria-disabled={disabled || levels.length === 0}
        aria-labelledby={labelId} aria-valuetext={value || t("modelSelection.default")}
        // 保存时保留焦点，拦住输入，避免 disabled 触发失焦后再重复聚焦。
        onPointerDown={(event) => { if (disabled) event.preventDefault(); }}
        onKeyDown={(event) => { if (disabled && event.key !== "Tab") event.preventDefault(); }}
        onChange={(event) => { if (!disabled && levels.length) onChange(levels[Number(event.target.value)]); }}
        onPointerUp={(event) => {
          if (!disabled && levels.length) {
            const next = levels[Number(event.currentTarget.value)];
            onChange(next);
            onCommit?.(next);
          }
        }}
        onKeyUp={(event) => {
          if (!disabled && levels.length && ["ArrowLeft", "ArrowRight", "ArrowUp", "ArrowDown", "Home", "End"].includes(event.key)) {
            const next = levels[Number(event.currentTarget.value)];
            onChange(next);
            onCommit?.(next);
          }
        }} onBlur={() => { if (!disabled && index >= 0) onCommit?.(value); }} />
      </div>
    </div>
  );
}
