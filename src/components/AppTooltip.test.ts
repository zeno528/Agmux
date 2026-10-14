// @ts-expect-error 测试运行于 Node，但应用的浏览器 tsconfig 不加载 Node 类型。
import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { POPOVER_EXIT_MS, tooltipOriginX, tooltipOriginY, tooltipPosition } from "./AppTooltip";

const styleSource = readFileSync(new URL("../style.css", import.meta.url), "utf8");

describe("tooltipPosition", () => {
  it("stays inside the viewport and flips above a bottom-edge trigger", () => {
    expect(tooltipPosition({ left: 760, top: 550, bottom: 574 }, { width: 320, height: 120 }, { width: 800, height: 600 })).toEqual({ left: 472, top: 422 });
    expect(tooltipPosition({ left: 20, top: 20, bottom: 44 }, { width: 320, height: 120 }, { width: 800, height: 600 })).toEqual({ left: 20, top: 52 });
  });
});

describe("浮层缩放原点", () => {
  it("提示卡在按钮下方时贴上缘，上方时贴下缘", () => {
    // 卡片在按钮下方（panelTop >= trigger.bottom）→ 从上边胀开
    expect(tooltipOriginY({ top: 100, bottom: 124 }, 132)).toBe("0%");
    // 空间不足翻到按钮上方 → 从下边胀开
    expect(tooltipOriginY({ top: 500, bottom: 524 }, 368)).toBe("100%");
  });

  it("横向取离按钮中心更近的卡片边缘", () => {
    // 卡片中心在按钮中心右侧 → 贴左边
    expect(tooltipOriginX({ left: 100, right: 124 }, 100, 320)).toBe("0%");
    // 卡片被视口夹到按钮左侧（如贴右边缘的触发器）→ 贴右边
    expect(tooltipOriginX({ left: 700, right: 724 }, 468, 320)).toBe("100%");
  });
  it("按钮在左、卡片也在左时，横向原点贴左边", () => {
    // 回归：曾用 panel.offsetLeft 自身偏移算原点，但卡片此刻尚未写入 left/top，
    // 读到未定位的值导致首次悬停从右侧展开。
    const trigger = { left: 100, right: 124, top: 200, bottom: 224 };
    const pos = tooltipPosition(trigger, { width: 320, height: 120 }, { width: 1200, height: 800 });
    expect(pos.left).toBe(100);
    expect(tooltipOriginX(trigger, pos.left, 320)).toBe("0%");
    expect(tooltipOriginY(trigger, pos.top)).toBe("0%");
  });

  it("位置解算完成前不挂进场动画，避免首帧退回兜底原点", () => {
    const source = readFileSync(new URL("./AppTooltip.tsx", import.meta.url), "utf8");
    expect(source).toContain("const placed = position !== null;");
    expect(source).toContain('data-popover-in={open && placed ? "" : undefined}');
  });

  it("打开时不能清空 position：被动 effect 跑在定位 effect 之后会抹掉解算结果", () => {
    // 回归：曾在退出管理的 useEffect 里 setPosition(null)，它晚于定位 useLayoutEffect 执行，
    // 把刚算好的位置抹掉，而定位 effect 依赖 [open] 不会重跑，
    // position 永远为 null → 卡片停在 visibility: hidden，悬停完全无反应。
    const source = readFileSync(new URL("./AppTooltip.tsx", import.meta.url), "utf8");
    expect(source).not.toContain("setPosition(null)");
  });
});

describe("提示卡出场", () => {
  it("退出时长与 CSS --motion-popover 一致，否则动画被提前截断", () => {
    expect(styleSource).toContain("--motion-popover: 200ms;");
    expect(POPOVER_EXIT_MS).toBe(200);
  });

  it("出场缓出而非 antd 的 motionEaseInQuint", () => {
    // cubic-bezier(.755,.05,.855,.06) 在 200ms 里前 140ms 只走完 14%，
    // 可见动作全挤在最后 3 帧，为悬停菜单的 mouseLeaveDelay 设计；
    // 本项目浮层点击触发、关闭零延迟，必须用缓出曲线才看得见。
    expect(styleSource).toContain("--motion-popover-out: cubic-bezier(0.22, 1, 0.36, 1);");
    expect(styleSource).not.toContain("cubic-bezier(0.755, 0.05, 0.855, 0.06)");
  });

  it("进场出场各走一个关键帧，且共用全局浮层动画类", () => {
    const source = readFileSync(new URL("./AppTooltip.tsx", import.meta.url), "utf8");
    expect(source).toContain('data-popover-out={open ? undefined : (placed ? "" : undefined)}');
    // 出场期间节点仍在场，卸载交给 exiting 状态
    expect(source).toContain("const mounted = open || exiting;");
  });
});
