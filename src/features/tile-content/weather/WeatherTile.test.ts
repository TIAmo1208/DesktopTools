import assert from "node:assert/strict";
import test from "node:test";
import { readFile } from "node:fs/promises";
import { stripTypeScriptTypes } from "node:module";
import { runInNewContext } from "node:vm";

test("城市弹窗动作串行，卸载后拒绝城市列表迟到结果", async () => {
    const source = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8");
    const gate = source.slice(source.indexOf("    const runCityAction = async"), source.indexOf("\n    /** 按来源搜索候选位置"));
    assert.ok(gate.includes("const runCityAction")); // 城市弹窗的提交门槛由格子自己持有。
    const cityBusy = { current: false };
    const states: any[] = [];
    const action = async () => {
        states.push({ ran: true });
    };
    const scope: any = {
        cityBusy,
        setCityBusyState: () => {},
        setCityDialog: (update: (value: any) => any) => states.push(update({ checks: [], status: "", failure: false })),
    };
    const runCityAction = runInNewContext(stripTypeScriptTypes(gate) + "\nrunCityAction", scope);
    await Promise.all([runCityAction(action), runCityAction(action)]); // 忙碌期间拒绝重复提交。
    assert.equal(states.filter((value) => value.ran).length, 1);
    assert.equal(cityBusy.current, false); // 结束后释放门槛。
    await runCityAction(async () => {
        throw new Error("保存失败"); // 非本 Realm 的错误不能被识别为 Error，回落文案同样进入状态行。
    });
    assert.ok(states.some((value) => value.failure === true && typeof value.status === "string" && value.status.length > 0)); // 失败说明留在弹窗状态行。

    // 城市列表读取按来源与城市标识推导，换源与卸载后都不得回写状态；菜单打开时重读。
    const effect = source.match(/useEffect\(\(\) => \{\s*let active = true; \/\/ 卸载或换源后拒绝迟到读取。[\s\S]*?\}, \[source\]\);/)?.[0];
    assert.ok(effect);
    assert.match(effect, /const current: WeatherSource \| undefined = source; \/\/ 本次读取的来源，换源后不再沿用上一家列表。/); // 只把捕获到的来源传入闭包，菜单监听不读取可变状态。
    const flush = async () => {
        for (let index = 0; index < 4; index++) await Promise.resolve(); // 读取链路含多层封装，逐层放行微任务。
    };
    const requests: Array<{ resolve: (value: unknown) => void; reject: (error: unknown) => void; command: string }> = [];
    const window = new EventTarget(); // 菜单打开事件由窗口广播，测试沿用同一事件通道。
    const cityIdRef = { current: "101010100" }; // 生产实现用 ref 读取最新城市标识，测试保持同一读取路径。
    const notices: any[] = [];
    let lists: Record<string, unknown[]> = {}; // 按来源缓存的已读取列表，未读取的来源缺项。
    let cleanup: (() => void) | undefined;
    const request = (command: string, index: number) => requests.filter((item) => item.command === command)[index]!; // 按发起顺序取同类读取。
    const mount = (next: { source: string; cityId: string }) => {
        cleanup?.();
        runInNewContext(stripTypeScriptTypes(effect), {
            source: next.source,
            cityId: next.cityId,
            cityIdRef,
            window,
            setCity: () => {},
            setCitiesFor: (update: (value: Record<string, unknown[]>) => Record<string, unknown[]>) => {
                lists = update(lists);
            },
            publishMessage: (value: unknown) => notices.push(value),
            getWeatherCities: () => new Promise((resolve, reject) => requests.push({ resolve, reject, command: "cities" })),
            listenWeatherConfiguration: () => Promise.resolve(() => {}), // 配置通知不影响本用例的城市读取通道。
            useEffect: (callback: () => (() => void) | undefined) => {
                cleanup = callback();
            },
        });
    };
    mount({ source: "qweather", cityId: "101010100" });
    assert.equal(requests.map((item) => item.command).join(","), "cities");
    mount({ source: "foreca", cityId: "101010100" }); // 切换来源后上一家的在途读取作废。
    request("cities", 0).resolve([{ id: "old" }]);
    await flush();
    assert.equal(Object.keys(lists).length, 0);
    request("cities", 1).resolve([{ id: "new" }]);
    await flush();
    assert.equal(JSON.stringify(lists.foreca), '[{"id":"new"}]');
    assert.equal("qweather" in lists, false); // 上一家的在途结果不回写。
    window.dispatchEvent(new Event("desktop-tile-menu-opening")); // 菜单打开时重读，设置页新增的城市进入二级菜单。
    assert.equal(requests.length, 3);
    request("cities", 2).resolve([{ id: "added" }]);
    await flush();
    assert.equal(JSON.stringify(lists.foreca), '[{"id":"added"}]');
    window.dispatchEvent(new Event("desktop-tile-menu-opening"));
    request("cities", 3).reject(new Error("天气城市读取失败")); // 读取失败给出可见反馈，且不能把失败当成“列表为空”。
    await flush();
    assert.equal(JSON.stringify(lists.foreca), '[{"id":"added"}]'); // 失败保留上一次成功读取的列表，避免把“读不到”误判成“城市已移除”。
    assert.deepEqual(JSON.parse(JSON.stringify(notices)), [{ type: "error", content: "天气城市读取失败", source: "desktop-tile" }]);
    cleanup?.(); // 卸载使在途读取与菜单监听一起失效。
    request("cities", 1).resolve([{ id: "late" }]);
    request("cities", 3).resolve([{ id: "late" }]);
    await flush();
    assert.equal(JSON.stringify(lists.foreca), '[{"id":"added"}]');
    const settled = requests.length;
    window.dispatchEvent(new Event("desktop-tile-menu-opening")); // 卸载后菜单事件不再发起读取。
    assert.equal(requests.length, settled);
});

test("来源栏按模块有效期一次性到期，隐藏恢复重算且旧回调失效", async () => {
    const source = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8");
    const effect = source.match(/useEffect\(\(\) => \{\s*if \(!visible \|\| paused\) return;[\s\S]*?\}, \[forecast, visible, paused\]\);/)?.[0];
    assert.ok(effect);
    let now = 10_000_000;
    let expired: boolean | undefined;
    let cleanup: (() => void) | undefined;
    let listener: (() => void) | undefined;
    let pending: { callback: () => void; delay: number } | undefined;
    const document = {
        visibilityState: "visible",
        addEventListener: (_: string, callback: () => void) => {
            listener = callback;
        },
        removeEventListener: () => {
            listener = undefined;
        },
    };
    const render = (forecast: unknown, paused = false) => {
        cleanup?.();
        runInNewContext(stripTypeScriptTypes(effect), {
            forecast,
            visible: true,
            paused,
            document,
            Date: { now: () => now },
            setExpired: (value: boolean) => {
                expired = value;
            },
            setTimeout: (callback: () => void, delay: number) => {
                pending = { callback, delay };
                return 1;
            },
            clearTimeout: () => {
                pending = undefined;
            },
            useEffect: (callback: () => (() => void) | undefined) => {
                cleanup = callback();
            },
        });
    };
    const fetchedAt = now;
    render({ daily: {}, moduleFetchedAt: { daily: fetchedAt } });
    assert.equal(pending?.delay, 180 * 60 * 1000);
    assert.equal(expired, false);
    render({ current: {}, hourly: {}, moduleFetchedAt: { current: fetchedAt, hourly: fetchedAt } });
    assert.equal(pending?.delay, 30 * 60 * 1000);
    const obsolete = pending!.callback;
    document.visibilityState = "hidden";
    listener!();
    assert.equal(pending, undefined);
    now += 30 * 60 * 1000;
    document.visibilityState = "visible";
    listener!();
    assert.equal(expired, true);
    assert.equal(pending, undefined);
    render({ hourly: {}, moduleFetchedAt: { hourly: now } });
    obsolete();
    assert.equal(expired, false);
    render({}, true);
    assert.equal(pending, undefined);
    assert.equal(listener, undefined);
    render({});
    assert.equal(expired, false);
    render({ current: {}, moduleFetchedAt: { current: now + 1 } });
    assert.equal(expired, true);
    cleanup?.();
});

test("模块退出禁用交互，反向切换使用中间帧且关闭动画立即收敛", async () => {
    const source = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8");
    const effect = source.match(/useLayoutEffect\(\(\) => \{\s*const container = modulesRef[\s\S]*?\}, \[mode, animateModules, city\?\.id\]\);/)?.[0];
    assert.ok(effect);
    const animations: any[] = [];
    const moduleAnimations = { current: new Map() };
    const document = { activeElement: null as any };
    const container: any = { dataset: {}, getBoundingClientRect: () => ({ left: 0, top: 0 }), children: [] };
    for (let index = 0; index < 3; index++)
        container.children.push({
            hidden: false,
            inert: false,
            dataset: {},
            style: {},
            opacity: "1",
            presentedTop: undefined,
            getBoundingClientRect() {
                return { left: 0, top: this.presentedTop ?? index * 50, width: 100, height: 40 };
            },
            contains(element: unknown) {
                return element === this;
            },
            blur() {
                document.activeElement = null;
            },
            setAttribute() {},
            animate(frames: any[], options: any) {
                const animation = {
                    frames,
                    options,
                    cancelled: false,
                    onfinish: () => {},
                    cancel() {
                        this.cancelled = true;
                    },
                };
                animations.push(animation);
                return animation;
            },
        });
    const render = (mode: string, animateModules = true) =>
        runInNewContext(stripTypeScriptTypes(effect), {
            mode,
            animateModules,
            city: { id: "test" },
            modulesRef: { current: container },
            moduleAnimations,
            document,
            getComputedStyle: (element: any) => ({ opacity: element.opacity }),
            useLayoutEffect: (callback: () => void) => callback(),
        });
    render("summary");
    assert.equal(animations.length, 0);
    assert.equal(container.children[1].hidden, true);
    render("hourly");
    assert.equal(animations.length, 1);
    assert.equal(animations[0].frames[0].opacity, "0");
    assert.equal(animations[0].options.duration, 180);
    animations[0].onfinish();
    assert.equal(moduleAnimations.current.size, 0);
    document.activeElement = container.children[1];
    render("summary");
    const exit = animations.at(-1);
    assert.equal(container.children[1].hidden, false);
    assert.equal(container.children[1].inert, true);
    assert.equal(document.activeElement, null);
    container.children[1].opacity = "0.4";
    container.children[1].presentedTop = 52;
    render("hourly");
    const reverse = animations.at(-1);
    assert.equal(exit.cancelled, true);
    assert.equal(reverse.frames[0].opacity, "0.4");
    assert.equal(reverse.frames[1].opacity, "1");
    assert.equal(container.children[1].inert, false);
    exit.onfinish();
    assert.equal(container.children[1].hidden, false);
    const count = animations.length;
    render("summary", false);
    assert.equal(reverse.cancelled, true);
    assert.equal(animations.length, count);
    assert.equal(container.children[1].hidden, true);
    assert.equal(moduleAnimations.current.size, 0);
    assert.ok(animations.every((animation) => animation.frames.every((frame: any) => !frame.transform.includes("scale"))));
});

test("今天按城市日期识别，不把首条未来日预报视作今天", async () => {
    const source = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8");
    const declaration = source.match(/const isToday = \([\s\S]*?\n\};/)?.[0];
    assert.ok(declaration);
    class FixedDate extends Date {
        constructor(value?: string) {
            super(value ?? "2026-01-01T23:30:00Z");
        }
    }
    const isToday = runInNewContext(`${stripTypeScriptTypes(declaration)}; isToday`, {
        Date: FixedDate,
        dateFormatter: (city: { tz: string }, options: Intl.DateTimeFormatOptions) => new Intl.DateTimeFormat("zh-CN", { ...options, timeZone: city.tz }),
    });
    assert.equal(isToday("2026-01-02T00:00:00+08:00", { tz: "Asia/Shanghai" }), true);
    assert.equal(isToday("2026-01-03T00:00:00+08:00", { tz: "Asia/Shanghai" }), false);
    assert.equal(isToday("2026-01-01T00:00:00-05:00", { tz: "America/New_York" }), true);
    assert.equal(isToday("invalid", { tz: "Asia/Shanghai" }), false);
});

test("小时与日预报到期过滤，暂停停止计时且旧回调不清除新结果", async () => {
    const source = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8");
    const effect = source.match(/useEffect\(\(\) => \{\s*if \(!forecast \|\| !visible \|\| paused\)[\s\S]*?\}, \[forecast, visible, paused\]\);/)?.[0];
    assert.ok(effect);
    let now = Date.parse("2026-03-09T03:59:59.999Z"); // 纽约夏令时开始日的最后一毫秒。
    const document = Object.assign(new EventTarget(), { visibilityState: "visible" });
    const timers = new Map<number, { callback: () => void; delay: number }>();
    let id = 0;
    let cleanup: (() => void) | undefined;
    let forecast: any = {
        current: { temperature: 10 },
        hourly: { hours: [{ forecastTime: "2026-03-09T03:00:00Z" }, { forecastTime: "2026-03-09T04:00:00Z" }] },
        daily: { days: [{ forecastStartTime: "2026-03-08T00:00:00-05:00", forecastEndTime: "2026-03-09T00:00:00-04:00" }] },
    };
    const mount = (visible = true, paused = false) =>
        runInNewContext(stripTypeScriptTypes(effect), {
            forecast,
            visible,
            paused,
            document,
            Date: { now: () => now, parse: Date.parse },
            setTimeout(callback: () => void, delay: number) {
                timers.set(++id, { callback, delay });
                return id;
            },
            clearTimeout(key: number) {
                timers.delete(key);
            },
            setForecast(update: (value: any) => any) {
                forecast = update(forecast);
            },
            useEffect(callback: () => (() => void) | undefined) {
                cleanup = callback();
            },
        });
    mount();
    assert.equal(timers.get(id)?.delay, 1);
    now += 1;
    timers.get(id)!.callback();
    assert.equal(forecast.hourly.hours.length, 1);
    assert.equal(forecast.daily.days.length, 0);
    assert.equal(forecast.current.temperature, 10);
    assert.ok(forecast.moduleErrors.hourly);
    assert.ok(forecast.moduleErrors.daily);
    cleanup?.();
    assert.equal(timers.size, 0);
    mount(false);
    mount(true, true);
    assert.equal(timers.size, 0);
    mount();
    const late = timers.get(id)!.callback;
    document.visibilityState = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));
    assert.equal(timers.size, 0);
    now += 60 * 60 * 1000;
    document.visibilityState = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
    assert.equal(forecast.hourly.hours.length, 0);
    assert.equal(timers.size, 0);
    const fresh = { hourly: { hours: [{ forecastTime: "2026-03-09T05:00:00Z" }] } };
    forecast = fresh;
    late();
    assert.equal(forecast, fresh);
    cleanup?.();
    late();
    assert.equal(timers.size, 0);
});

test("折叠和宿主暂停禁止读取，恢复保留缓存并拒绝旧响应", async () => {
    const source = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8");
    const effect = source.match(
        /useEffect\(\(\) => \{\s*if \(!config \|\| !city\) return;[\s\S]*?\}, \[config\?\.source, config\?\.cityId, city\?\.id, visible, paused, instance\.instanceId, instance\.monitorId, instance\.projectionMonitorId\]\);/,
    )?.[0];
    assert.ok(effect);
    const document = Object.assign(new EventTarget(), { visibilityState: "visible", hasFocus: () => true });
    const window = new EventTarget();
    const requests: Array<{ resolve: (value: unknown) => void; reject: (error: Error) => void }> = [];
    let configurationChanged = () => {};
    let forecast: unknown = { cached: true };
    let failure: unknown;
    let cleanup: (() => void) | undefined;
    const mount = async (visible: boolean, paused: boolean) => {
        cleanup?.();
        runInNewContext(stripTypeScriptTypes(effect), {
            config: { source: "qweather", cityId: "test" }, // 格子自己保存的来源与城市标识。
            city: { id: "test" },
            instance: { instanceId: "weather", monitorId: "DISPLAY1", projectionMonitorId: null },
            visible,
            paused,
            document,
            window,
            getWeatherForecast: () => new Promise((resolve, reject) => requests.push({ resolve, reject })),
            listenWeatherConfiguration: async (handler: () => void) => {
                configurationChanged = handler;
                return () => {};
            },
            listenWeatherForecast: async () => () => {},
            setForecast: (value: unknown) => {
                forecast = value;
            },
            setFailure: (value: unknown) => {
                failure = value;
            },
            setMotionActive: () => {},
            useEffect: (callback: () => (() => void) | undefined) => {
                cleanup = callback();
            },
        });
        await Promise.resolve();
    };
    await mount(false, false);
    await mount(true, true);
    assert.equal(requests.length, 0);
    await mount(true, false);
    assert.equal(requests.length, 1);
    assert.deepEqual(forecast, { cached: true });
    document.visibilityState = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));
    requests[0].resolve({ stale: true });
    await Promise.resolve();
    assert.deepEqual(forecast, { cached: true });
    document.visibilityState = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
    assert.equal(requests.length, 2);
    cleanup?.();
    await mount(true, true);
    requests[1].reject(new Error("late"));
    await Promise.resolve();
    await Promise.resolve();
    assert.equal(failure, undefined);
    await mount(true, false);
    requests[2].resolve({ fresh: true });
    await Promise.resolve();
    assert.deepEqual(forecast, { fresh: true });
    configurationChanged();
    assert.equal(forecast, undefined);
    assert.equal(requests.length, 4);
    const oldConfiguration = requests[3];
    configurationChanged();
    oldConfiguration.resolve({ obsolete: true });
    await Promise.resolve();
    assert.equal(forecast, undefined);
    requests[4].resolve({ newConfiguration: true });
    await Promise.resolve();
    assert.deepEqual(forecast, { newConfiguration: true });
    await mount(true, true);
    configurationChanged();
    assert.equal(forecast, undefined);
    cleanup?.();
    document.dispatchEvent(new Event("visibilitychange"));
    window.dispatchEvent(new Event("focus"));
    assert.equal(requests.length, 5);
});

test("布局观察公共内容盒，拖动和标题显隐立即更新且相同尺寸不重复渲染", async () => {
    const source = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8"); // 执行生产尺寸观察 Effect。
    const effect = source.match(/useEffect\(\(\) => \{\s*const element = summaryRef[\s\S]*?\}, \[instance.instanceId\]\);/)?.[0];
    assert.ok(effect);
    const element = {}; // 公共视口 DOM 身份。
    let size = { width: 0, height: 0 };
    let receive = (_entries: any[]) => {};
    let cleanup = () => {};
    let disconnected = false;
    runInNewContext(stripTypeScriptTypes(effect), {
        summaryRef: {
            current: {
                closest(selector: string) {
                    assert.equal(selector, ".official-component__content");
                    return element;
                },
            },
        },
        instance: { instanceId: "weather" },
        setContentSize(update: (value: typeof size) => typeof size) {
            size = update(size);
        },
        useEffect(callback: () => () => void) {
            cleanup = callback();
        },
        ResizeObserver: class {
            constructor(callback: typeof receive) {
                receive = callback;
            }
            observe(target: unknown) {
                assert.equal(target, element);
            }
            disconnect() {
                disconnected = true;
            }
        },
    });
    receive([{ target: element, contentRect: { width: 456, height: 168 } }]);
    assert.equal(size.width, 456);
    assert.equal(size.height, 168);
    const unchanged = size;
    receive([{ target: element, contentRect: { width: 456, height: 168 } }]);
    assert.equal(size, unchanged);
    receive([{ target: element, contentRect: { width: 456, height: 196 } }]); // 隐藏固定标题释放 28px，无需提交外框尺寸。
    assert.equal(size.height, 196);
    receive([{ target: element, contentRect: { width: 496, height: 408 } }]); // 拖动中的测量值直接进入状态。
    assert.equal(size.width, 496);
    assert.equal(size.height, 408);
    cleanup();
    assert.ok(disconnected);
});

test("天气尺寸按完整、7天、小时、摘要顺序匹配，双维度均须满足", async () => {
    const source = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8");
    const functionSource = source.match(/const selectWeatherMode = \([\s\S]*?\n\};/)?.[0];
    assert.ok(functionSource);
    const sizesSource = source.match(/const weatherSizes = \{[\s\S]*?\n\};/)?.[0];
    assert.ok(sizesSource);
    const selectWeatherMode = runInNewContext(
        `${sizesSource}\n(${stripTypeScriptTypes(functionSource.replace(/^const selectWeatherMode = /, "").replace(/;$/, ""))})`,
        {},
    );
    for (const scale of [12 / 13, 1, 18 / 13]) {
        assert.equal(selectWeatherMode(430 * scale, 320 * scale, scale), "full");
        assert.equal(selectWeatherMode(430 * scale, 320 * scale - 1, scale), "daily");
        assert.equal(selectWeatherMode(430 * scale - 1, 320 * scale, scale), "daily");
        assert.equal(selectWeatherMode(380 * scale, 300 * scale, scale), "daily");
        assert.equal(selectWeatherMode(380 * scale, 300 * scale - 1, scale), "hourly");
        assert.equal(selectWeatherMode(200 * scale, 300 * scale, scale), "daily");
        assert.equal(selectWeatherMode(200 * scale - 1, 320 * scale, scale), "summary");
        assert.equal(selectWeatherMode(380 * scale, 120 * scale, scale), "hourly");
        assert.equal(selectWeatherMode(380 * scale, 120 * scale - 1, scale), "summary");
        assert.equal(selectWeatherMode(380 * scale - 1, 120 * scale, scale), "summary");
    }
});

test("固定模式跳过自动判断，尺寸提交失败不保存模式且连续操作不重入", async () => {
    const source = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8");
    const modeSource = source.match(/const mode = fixedMode[^;]*;/)?.[0];
    assert.ok(modeSource);
    let calls = 0;
    const scope = {
        fixedMode: "daily",
        contentWidth: 900,
        contentHeight: 900,
        fontScale: 1,
        selectWeatherMode: () => {
            calls++;
            return "full";
        },
    };
    assert.equal(runInNewContext(modeSource + "\nmode", scope), "daily");
    assert.equal(calls, 0);
    assert.equal(runInNewContext(modeSource + "\nmode", { ...scope, fixedMode: undefined }), "full");
    assert.equal(calls, 1);

    const handlerSource = source.match(/const handleMode = async[\s\S]*?\n    \};/)?.[0];
    assert.ok(handlerSource);
    const commands: any[] = [];
    const errors: any[] = [];
    const busy = { current: false };
    let fail = false;
    const handleMode = runInNewContext(stripTypeScriptTypes(handlerSource) + "\nhandleMode", {
        busy,
        state: { unrelated: "kept" },
        sizeFor: () => ({ width: 224, height: 352 }),
        publishMessage: (value: unknown) => errors.push(value),
        props: {
            instance: { instanceId: "weather", stateVersion: 1, bounds: { left: 10, top: 20, width: 192, height: 120 } },
            onCommand: async (command: unknown) => {
                commands.push(command);
                if (fail) throw new Error("尺寸失败");
            },
        },
    });
    const pending = handleMode("daily", true, true);
    await handleMode("hourly", true, true);
    await pending;
    assert.equal(commands.length, 2);
    assert.equal(commands[0].bounds.width, 224);
    assert.equal(commands[0].bounds.left, 10);
    assert.equal(commands[1].state.weatherMode, "daily");
    assert.equal(commands[1].state.weatherModeFixed, true);
    assert.equal(commands[1].state.unrelated, "kept");
    commands.length = 0;
    fail = true;
    await handleMode("daily", true, true);
    assert.equal(commands.length, 1);
    assert.equal(errors.length, 1);
    assert.equal(busy.current, false);
    commands.length = 0;
    fail = false;
    await handleMode("daily", false, false);
    assert.equal(commands.length, 1);
    assert.equal(commands[0].type, "update-component-state");
    assert.equal(commands[0].state.weatherModeFixed, false);
});

test("当前天气在到期、隐藏恢复时失效，旧回调不清除新预报且卸载释放计时", async () => {
    const source = await readFile(new URL("./WeatherTile.tsx", import.meta.url), "utf8"); // 沿用仓库测试方式执行生产 Effect。
    const effect = source.match(
        /useEffect\(\(\) => \{\s*const currentFetchedAt[\s\S]*?\}, \[forecast\?\.current, forecast\?\.moduleFetchedAt\?\.current, visible, paused\]\);/,
    )?.[0];
    assert.ok(effect);
    let now = 100_000_000; // 固定时钟，无真实等待。
    const day = 24 * 60 * 60 * 1000;
    const document = Object.assign(new EventTarget(), { visibilityState: "visible" });
    const timers = new Map<number, { callback: () => void; delay: number }>();
    let id = 0;
    let cleanup = () => {};
    let forecast: any = { current: { temperature: 20 }, hourly: { hours: [1] }, moduleFetchedAt: { current: now - day + 1 } };
    const mount = (visible = true, paused = false) =>
        runInNewContext(stripTypeScriptTypes(effect), {
            forecast,
            visible,
            paused,
            document,
            Number,
            Date: { now: () => now },
            setTimeout(callback: () => void, delay: number) {
                timers.set(++id, { callback, delay });
                return id;
            },
            clearTimeout(key: number) {
                timers.delete(key);
            },
            setForecast(update: (value: any) => any) {
                forecast = update(forecast);
            },
            useEffect(callback: () => () => void) {
                cleanup = callback();
            },
        }); // 模拟 React 最新 state，保持事件与定时器真实生产逻辑。
    mount();
    assert.equal(timers.size, 1);
    assert.equal(timers.get(id)?.delay, 1);
    now += 1;
    timers.get(id)!.callback();
    assert.equal(forecast.current, undefined);
    assert.ok(forecast.moduleErrors.current);
    assert.equal(forecast.hourly.hours[0], 1);
    assert.equal(timers.size, 0);
    cleanup();

    forecast = { current: {}, moduleFetchedAt: { current: now }, moduleErrors: { current: "网络失败" } };
    mount();
    document.visibilityState = "hidden";
    document.dispatchEvent(new Event("visibilitychange"));
    assert.equal(timers.size, 0);
    now += day;
    document.visibilityState = "visible";
    document.dispatchEvent(new Event("visibilitychange"));
    assert.equal(forecast.current, undefined);
    assert.equal(forecast.moduleErrors.current, "网络失败");
    cleanup();

    forecast = { current: {}, moduleFetchedAt: { current: now } };
    mount();
    const late = timers.get(id)!.callback; // 保留已经排队的旧回调。
    now += day;
    const fresh = { current: { temperature: 21 }, moduleFetchedAt: { current: now } };
    forecast = fresh;
    late();
    assert.equal(forecast, fresh);
    cleanup();
    assert.equal(timers.size, 0);
    late();
    document.dispatchEvent(new Event("visibilitychange"));
    assert.equal(forecast, fresh);
    assert.equal(timers.size, 0);
    forecast = { current: {}, moduleFetchedAt: { current: now } };
    mount(false, false);
    assert.equal(timers.size, 0);
    mount(true, true);
    assert.equal(timers.size, 0);
    now += day;
    mount();
    assert.equal(forecast.current, undefined);
    cleanup();
});
