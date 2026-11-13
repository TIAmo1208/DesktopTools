// 浮岛断言页共用夹具：只替换原生桥与事件推送，几何、切态与消息投影仍走生产代码。
// 证据层为 L1 浏览器断言；命中、穿透、材质与观感仍由 L3 真机取证与用户确认覆盖。

/** 定时等待：断言页不依赖 rAF，隐藏页面也能推进。 */
export const 等待 = (毫秒) => new Promise((resolve) => setTimeout(resolve, 毫秒));

/** 有界轮询：超时抛出带说明的错误，由页面统一写成 FAIL，不静默通过。 */
export const 直到 = async (探针, 说明 = "等待状态收敛", 超时 = 6000) => {
    const 起点 = Date.now();
    while (Date.now() - 起点 < 超时) {
        const 值 = 探针();
        if (值) return 值;
        await 等待(16);
    }
    throw new Error(`${说明}超时（${超时}ms）`);
};

/** 单条浮岛消息投影：字段与 contracts/island 一一对应，页面不另造形态。 */
export const 消息 = (id, 类型, 正文, 来源, 到达时刻 = Date.now(), 加载中 = false) => ({
    id,
    type: 类型,
    content: 正文,
    source: 来源,
    loading: 加载中,
    receivedAtMs: 到达时刻,
});

/**
 * 安装夹具：必须在动态导入产品模块之前调用，保证桥先于模块副作用存在。
 * 消息初读支持 ok / hang / fail，用来覆盖消息通道超时与失败的降级路径。
 */
export const 创建夹具 = ({ 偏好补丁 = {}, 消息初读 = "ok" } = {}) => {
    const 监听表 = new Map(); // 事件名 → 处理函数组，模拟原生事件广播。
    const 推送 = (事件, 载荷) => {
        for (const 处理 of [...(监听表.get(事件) ?? [])]) 处理(载荷);
    };
    const 提交 = []; // set_island_state 的原生几何提交。
    const 关闭记录 = []; // dismiss_island_message 的消息身份。
    const 保存记录 = []; // save_preferences 的完整偏好载荷。
    let 消息集合 = []; // 后端消息队列的页面投影。

    const 偏好 = {
        theme: "dark",
        desktopAppearance: { theme: "dark", accentColor: "#2563eb", useWindowsAccentColor: false },
        doubleClickToOpen: true,
        islandEnabled: true,
        islandForm: "notch",
        islandEdge: "top",
        islandOffset: 0,
        islandRadius: 12,
        interfaceAnimations: true,
        followSystemReducedMotion: false,
        ...偏好补丁,
    };
    // 岛快照只含一个音乐控制实例：消息卡与调试卡不走实例清单。
    const 运行时 = {
        revision: 1,
        manifests: [
            {
                id: "official.music-control",
                name: "音乐控制",
                entry: "official",
                script: "",
                requiredCapabilities: [],
                creatable: true,
                removable: true,
                island: true,
                maxTiles: 1,
                tileTypes: [],
                persistenceVersion: 1,
            },
        ],
        instances: [
            {
                instanceId: "island.official.music-control",
                componentId: "official.music-control",
                entry: null,
                authorizedCapabilities: [],
                status: "ready",
                error: null,
            },
        ],
        failures: [],
    };
    const 岛 = {
        runtime: 运行时,
        instances: [
            {
                instanceId: "island.official.music-control",
                componentId: "official.music-control",
                monitorId: "",
                projectionMonitorId: null,
                projectionBounds: null,
                bounds: { left: 0, top: 0, width: 0, height: 0 },
                stateVersion: 1,
                state: {},
            },
        ],
    };
    const 桌面 = {
        phase: "ready",
        committed: true,
        hosts: [
            {
                monitor: {
                    id: "monitor-1",
                    name: "主屏",
                    primary: true,
                    x: 0,
                    y: 0,
                    width: 1920,
                    height: 1080,
                    workX: 0,
                    workY: 0,
                    workWidth: 1920,
                    workHeight: 1040,
                    scaleFactor: 1,
                    dpi: 96,
                },
                readiness: { guardian: true, page: true, systemImages: true, pixels: true, input: true, hitMap: true },
            },
        ],
    };

    window.__DESKTOP_TOOLS_NATIVE__ = {
        invoke(命令, 载荷) {
            switch (命令) {
                case "get_preferences":
                    return Promise.resolve({ ...偏好 });
                case "save_preferences":
                    保存记录.push(载荷?.preferences);
                    return Promise.resolve(载荷?.preferences);
                case "get_windows_accent_color":
                    return Promise.resolve("#2563eb");
                case "get_foreground_snapshot":
                    return Promise.resolve({ revision: 1, locked: false });
                case "get_island_snapshot":
                    return Promise.resolve(岛);
                case "get_component_runtime_snapshot":
                    return Promise.resolve(运行时);
                case "get_desktop_host_snapshot":
                    return Promise.resolve(桌面);
                case "get_current_music_session":
                    return Promise.resolve(null);
                case "get_music_details":
                    return Promise.resolve({ timeline: null, lyrics: [] });
                case "get_island_messages":
                    if (消息初读 === "hang") return new Promise(() => {}); // 通道超时：初读永不返回。
                    if (消息初读 === "fail") return Promise.reject(new Error("消息通道不可用"));
                    return Promise.resolve(消息集合);
                case "dismiss_island_message": {
                    const 身份 = 载荷?.id;
                    关闭记录.push(身份);
                    消息集合 = 消息集合.filter((项) => 项.id !== 身份);
                    推送("island-messages-changed", 消息集合); // 与原生一致：关闭后广播权威集合，页面不本地臆测。
                    return Promise.resolve();
                }
                case "set_island_state":
                    提交.push(载荷?.request);
                    return Promise.resolve();
                case "start_audio_spectrum":
                    return Promise.resolve({ running: true, deviceId: "", deviceName: "系统默认", fallback: false });
                case "list_audio_output_devices":
                    return Promise.resolve([]);
                default:
                    return Promise.resolve(undefined);
            }
        },
        listen(事件, 处理) {
            const 组 = 监听表.get(事件) ?? [];
            组.push(处理);
            监听表.set(事件, 组);
            return Promise.resolve(() => {
                const 位置 = 组.indexOf(处理);
                if (位置 >= 0) 组.splice(位置, 1);
            });
        },
        isRefreshPaused() {
            return false;
        },
    };

    return {
        偏好,
        提交,
        关闭记录,
        保存记录,
        get 消息集合() {
            return 消息集合;
        },
        pushMusic: (载值) => 推送("music-session-changed", 载值),
        pushDetails: (细节) => 推送("music-timeline-changed", 细节),
        pushMessages: (列表) => {
            消息集合 = 列表;
            推送("island-messages-changed", 列表);
        },
        pushRawMessages: (原始) => 推送("island-messages-changed", 原始),
        pushPreferences: (补丁) => {
            Object.assign(偏好, 补丁);
            推送("preferences-changed", { ...偏好 });
        },
    };
};

/** 媒体快照：播放中且三向可控，覆盖胶囊正常呈现。 */
export const 媒体 = (标题, 艺人, 来源 = "测试播放器") => ({
    media: { sessionId: "fixture-session", title: 标题, artist: 艺人, source: 来源, playback: "playing", canPrevious: true, canPlayPause: true, canNext: true },
});

/** 时间轴：advancing=false 让位置停在给定毫秒；标题与艺人必须与媒体快照一致，否则共享源会丢弃时间轴。 */
export const 时间轴 = (位置毫秒, 标题, 艺人, 时长毫秒 = 180000) => ({
    sessionId: "fixture-session",
    title: 标题,
    artist: 艺人,
    positionTicks: "0",
    endTimeTicks: "0",
    lastUpdatedTimeTicks: "0",
    playbackStatus: 4,
    positionMs: 位置毫秒,
    durationMs: 时长毫秒,
    advancing: false,
});

/** 逐句歌词：起始毫秒严格递增，正文长度足以触发常态横向滚动。 */
export const 逐句歌词 = [
    { startMs: 0, text: "第一句歌词用来占位并验证首句之前的回落与展开三行槽位" },
    { startMs: 10000, text: "第二句歌词足够长以便常态胶囊必须横向滚动而不是整句塞进内容盒" },
    { startMs: 20000, text: "第三句歌词收尾" },
];
