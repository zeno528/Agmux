import { beforeEach, describe, expect, it, vi } from "vitest";
import { api, ProfileOverwriteRequired } from "./index";
import { webInvoke } from "./web-mock";

vi.mock("./web-mock", () => ({ webInvoke: vi.fn() }));
beforeEach(() => vi.clearAllMocks());

describe("切换覆盖 IPC 契约", () => {
  it.each(["codex", "claude"] as const)("%s 识别覆盖确认错误并传递显式覆盖标志", async (client) => {
    const apply = client === "codex" ? api.codexApplyProfile : api.claudeApplyProfile;
    vi.mocked(webInvoke).mockRejectedValueOnce({ overwrite_required: true, message: "damaged" }).mockResolvedValueOnce(undefined);
    await expect(apply("target")).rejects.toBeInstanceOf(ProfileOverwriteRequired);
    expect(webInvoke).toHaveBeenLastCalledWith(`${client}_apply_profile`, { id: "target", overwrite: false });
    await apply("target", true);
    expect(webInvoke).toHaveBeenLastCalledWith(`${client}_apply_profile`, { id: "target", overwrite: true });
  });

  it("普通错误继续保持原有错误语义", async () => {
    vi.mocked(webInvoke).mockRejectedValueOnce("permission denied");
    await expect(api.claudeApplyProfile("target")).rejects.toBe("permission denied");
  });
});
