import { describe, expect, it, vi } from "vitest";
import { ProfileOverwriteRequired } from "../../api";
import { applyProfileWithConfirmation } from "./applyProfile";

describe("供应商切换覆盖确认", () => {
  it("正常切换不弹确认", async () => {
    const apply = vi.fn().mockResolvedValue(undefined);
    const confirm = vi.fn();
    expect(await applyProfileWithConfirmation(apply, confirm)).toBe(true);
    expect(apply).toHaveBeenCalledOnce();
    expect(confirm).not.toHaveBeenCalled();
  });

  it.each([false, true])("损坏配置只有用户确认 %s 后才重试覆盖", async (confirmed) => {
    const apply = vi.fn().mockRejectedValueOnce(new ProfileOverwriteRequired("damaged")).mockResolvedValue(undefined);
    const confirm = vi.fn().mockResolvedValue(confirmed);
    expect(await applyProfileWithConfirmation(apply, confirm)).toBe(confirmed);
    expect(confirm).toHaveBeenCalledOnce();
    expect(apply).toHaveBeenCalledTimes(confirmed ? 2 : 1);
    if (confirmed) expect(apply).toHaveBeenLastCalledWith(true);
  });

  it("读取与数据库故障不提供强制覆盖", async () => {
    const confirm = vi.fn();
    await expect(applyProfileWithConfirmation(vi.fn().mockRejectedValue(new Error("permission denied")), confirm)).rejects.toThrow("permission denied");
    expect(confirm).not.toHaveBeenCalled();
  });

  it("覆盖失败正常报错，不循环弹窗或重试", async () => {
    const apply = vi.fn().mockRejectedValueOnce(new ProfileOverwriteRequired("damaged")).mockRejectedValueOnce(new Error("write failed"));
    const confirm = vi.fn().mockResolvedValue(true);
    await expect(applyProfileWithConfirmation(apply, confirm)).rejects.toThrow("write failed");
    expect(apply).toHaveBeenCalledTimes(2);
    expect(confirm).toHaveBeenCalledOnce();
  });
});
