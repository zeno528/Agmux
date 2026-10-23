import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "../../api";
import { deleteCachedMcpProbe, loadMcpServers, setMcpServersCache } from "../../app/managementDataCache";
import type { McpProbeResult, McpServerSpec } from "../../types";
import { McpClientView, useMcpProbes } from "./McpView";

// 与 CliManagement 的交互测试相同：保留 hook 单元，显式推进渲染和 effect。
const hooks = vi.hoisted(() => ({
  cells: [] as unknown[], index: 0, effects: [] as (() => void)[],
  success: vi.fn(), error: vi.fn(), warning: vi.fn(),
}));
vi.mock("react", async (original) => ({
  ...await original<typeof import("react")>(),
  useEffect: (effect: () => void, deps: unknown[]) => {
    const index = hooks.index++;
    const previous = hooks.cells[index] as unknown[] | undefined;
    hooks.cells[index] = deps;
    if (!previous || deps.some((dep, i) => !Object.is(dep, previous[i]))) hooks.effects.push(effect);
  },
  useLayoutEffect: () => undefined,
  useRef: (initial: unknown) => {
    const index = hooks.index++;
    hooks.cells[index] ??= { current: initial };
    return hooks.cells[index];
  },
  useState: (initial: unknown) => {
    const index = hooks.index++;
    if (!(index in hooks.cells)) hooks.cells[index] = typeof initial === "function" ? initial() : initial;
    return [hooks.cells[index], (update: unknown) => {
      hooks.cells[index] = typeof update === "function" ? update(hooks.cells[index]) : update;
    }];
  },
}));
vi.mock("react-i18next", () => ({ useTranslation: () => ({ t: (key: string) => key }) }));
vi.mock("../../app/Feedback", () => ({ useFeedback: () => hooks }));
vi.mock("../../app/managementDataCache", async (original) => ({
  ...await original<typeof import("../../app/managementDataCache")>(),
  loadMcpServers: vi.fn(),
}));

const server: McpServerSpec = {
  name: "probe-fixture", enabled: true, url: "https://example.test/mcp", command: null,
  args: [], env: {}, startup_timeout_sec: null, tool_timeout_sec: null,
  bearer_token_env_var: null, http_headers: {}, env_http_headers: {},
};
const connected: McpProbeResult = {
  ok: true, connection_state: "connected", auth_status: "oAuth", latency_ms: 1,
  status: null, protocol_version: null, server_info: null, tools: [],
  tools_truncated: false, error: null, tools_error: null,
};
function deferred() {
  let resolve!: (result: McpProbeResult) => void;
  let reject!: (error: Error) => void;
  const promise = new Promise<McpProbeResult>((done, fail) => { resolve = done; reject = fail; });
  return { promise, resolve, reject };
}
function renderProbes() {
  hooks.index = 0;
  return useMcpProbes("codex", [server]);
}
beforeEach(() => {
  hooks.cells = []; hooks.index = 0; hooks.effects = [];
  hooks.success.mockClear(); hooks.error.mockClear(); hooks.warning.mockClear();
  deleteCachedMcpProbe(server.name, "codex");
  setMcpServersCache([server]);
  vi.mocked(loadMcpServers).mockResolvedValue([server]);
});
afterEach(() => { vi.restoreAllMocks(); deleteCachedMcpProbe(server.name, "codex"); });

it.each(["response", "rejection"])("较早请求的 %s 晚到也不覆盖新授权、不弹旧错误", async (late) => {
  const old = deferred(), latest = deferred();
  vi.spyOn(api, "probeMcpServer").mockReturnValueOnce(old.promise).mockReturnValueOnce(latest.promise);
  const first = renderProbes().probe(server);
  hooks.cells = [];
  const second = renderProbes().probe(server);
  latest.resolve(connected); await second;
  if (late === "response") old.resolve({ ...connected, ok: false, connection_state: "authentication_required" });
  else old.reject(new Error("old failure"));
  await first;
  expect(renderProbes().probeResults[server.name]).toEqual(connected);
  // 清空渲染单元模拟切页重挂载，检查共享缓存也未被旧结果覆盖。
  hooks.cells = [];
  expect(renderProbes().probeResults[server.name]).toEqual(connected);
  expect(hooks.success).toHaveBeenCalledOnce();
  expect(hooks.error).not.toHaveBeenCalled();
  expect(hooks.warning).not.toHaveBeenCalled();
});

it("旧请求结束不提前清除新请求的加载状态，禁用后不写回过期结果", async () => {
  const old = deferred(), latest = deferred();
  vi.spyOn(api, "probeMcpServer").mockReturnValueOnce(old.promise).mockReturnValueOnce(latest.promise);
  const probes = renderProbes();
  const first = probes.probe(server), second = probes.probe(server);
  old.resolve(connected); await first;
  expect(renderProbes().probingNames[server.name]).toBe(true);
  deleteCachedMcpProbe(server.name, "codex");
  latest.resolve(connected); await second;
  expect(renderProbes().probingNames[server.name]).toBeUndefined();
  expect(renderProbes().probeResults[server.name]).toBeUndefined();
  expect(hooks.success).not.toHaveBeenCalled();
});

it("工具列表与连通检测交叠时各自结束加载，旧工具错误不覆盖新连接", async () => {
  const tools = deferred(), connection = deferred();
  vi.spyOn(api, "probeMcpServer").mockReturnValueOnce(tools.promise).mockReturnValueOnce(connection.promise);
  const probes = renderProbes();
  const first = probes.probe(server, { includeTools: true }), second = probes.probe(server);
  connection.resolve(connected); await second;
  expect(renderProbes().probingNames[server.name]).toBeUndefined();
  expect(renderProbes().toolsLoading[server.name]).toBe(true);
  tools.reject(new Error("old tool failure")); await first;
  expect(renderProbes().toolsLoading[server.name]).toBeUndefined();
  expect(renderProbes().probeResults[server.name]).toEqual(connected);
  expect(hooks.error).not.toHaveBeenCalled();
});

it.each([0, 3])("激活计数为 %s 时首次进页只检测一次，后续激活再检测", async (epoch) => {
  const probe = vi.spyOn(api, "probeMcpServer").mockResolvedValue(connected);
  vi.spyOn(api, "codexMcpSyncPreview").mockResolvedValue({ entries: [], live_count: 1, db_count: 1 });
  const render = (activationEpoch: number, replay = false) => {
    hooks.index = 0;
    McpClientView({ target: "codex", activationEpoch, popEpoch: 0, onHeaderChange: () => undefined });
    const effects = hooks.effects.splice(0);
    effects.forEach((effect) => effect());
    if (replay) effects.forEach((effect) => effect());
  };
  render(epoch, true);
  await vi.waitFor(() => expect(probe).toHaveBeenCalledOnce());
  render(epoch);
  expect(probe).toHaveBeenCalledOnce();
  render(epoch + 1);
  await vi.waitFor(() => expect(probe).toHaveBeenCalledTimes(2));
});
