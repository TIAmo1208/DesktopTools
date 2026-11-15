import { useEffect, useRef, useState, type RefObject } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
    addWeatherCity,
    createWeatherTile,
    getWeatherCities,
    removeWeatherCity,
    searchWeatherCities,
    validateWeatherCity,
    weatherSettings,
} from "../../../bridge/weather";
import {
    SOURCE_CITY_CHECK,
    WEATHER_SOURCES,
    type WeatherCheck,
    type WeatherCity,
    type WeatherField,
    type WeatherSource,
    type WeatherSourceStatus,
} from "../../../contracts/weather";
import {
    AVAILABILITY,
    assembleWeatherCredentials,
    confirmWeatherCityDraft,
    emptyWeatherCityDraft,
    requiredWeatherFields,
    weatherCityCommitSteps,
    weatherDraftChanged,
    weatherDraftCities,
    weatherLeaveDecision,
    WEATHER_EMPTY_DRAFT,
    WEATHER_FIELDS,
    withdrawWeatherCityDraft,
    type WeatherCityDraft,
} from "./weatherSettingsFlow";
import { createWeatherOperationGate } from "./weatherSettingsSession";

/** 城市弹窗的调用方：各入口都只携带来源，搜索与验证统一使用该来源已保存的凭据。 */
interface CityPicker {
    source: WeatherSource; // 本次弹窗管理的来源。
}

/** 待确认的删除对象：来源凭据与城市项共用同一个确认框，未保存离开复用同一状态位。 */
type WeatherPendingRemoval = { kind: "source"; source: WeatherSource } | { kind: "city"; source: WeatherSource; city: WeatherCity } | { kind: "discard" };

/** 天气设置的领域状态与动作；组件只负责布局与事件绑定。 */
export interface WeatherSettingsOptions {
    leaveGuard: RefObject<((action: () => void) => void) | undefined>; // 天气草稿离开保护。
    onStatus?: (text: string) => void; // 页头展示的来源摘要。
    onBusy?: (busy: boolean) => void; // 报告当前是否忙碌，供页头添加入口决定是否可用。
    openAddKey?: number; // 页头点击添加时自增；每次变化只打开一次新增弹窗。
}

export const useWeatherSettings = ({ leaveGuard, onStatus, onBusy, openAddKey }: WeatherSettingsOptions) => {
    const [sources, setSources] = useState<WeatherSourceStatus[]>([]); // 已配置来源列表。
    const [busy, setBusy] = useState(""); // 当前动作名称。
    const [cityBusy, setCityBusy] = useState(false); // 城市弹窗是否正在执行动作。
    const [status, setStatus] = useState(""); // 脱敏的状态反馈。
    const [failure, setFailure] = useState(false); // 错误色只辅助文本。
    const [menu, setMenu] = useState<{ source: WeatherSource; left: number; top: number }>(); // 跟随鼠标的来源菜单。
    const [cityMenu, setCityMenu] = useState<{ source: WeatherSource; city: WeatherCity; left: number; top: number }>(); // 城市项右键菜单，确认删除前只展示命令。
    const [form, setForm] = useState<{ source: WeatherSource; adding: boolean }>(); // 添加或编辑弹窗。
    const [removal, setRemoval] = useState<WeatherPendingRemoval>(); // 待确认的删除对象。
    const [draft, setDraft] = useState<Record<WeatherField, string>>(WEATHER_EMPTY_DRAFT); // 只在当前页内存保存。
    const [visible, setVisible] = useState(false); // 仅当前敏感草稿显隐。
    const [expandedLoading, setExpandedLoading] = useState(false); // 当前展开项的城市列表是否正在读取。
    const [city, setCity] = useState<WeatherCity>(); // 编辑弹窗内经完整验证的城市。
    const [cityDraft, setCityDraft] = useState<WeatherCityDraft>(emptyWeatherCityDraft); // 编辑弹窗内尚未提交的城市改动。
    const [checks, setChecks] = useState<WeatherCheck[]>([]); // 编辑弹窗内本次验证的真实检查结果。
    const [picker, setPicker] = useState<CityPicker>(); // 城市弹窗及其调用方。
    const [expanded, setExpanded] = useState<WeatherSource>(); // 当前展开城市列表的来源。
    const [gate] = useState(() =>
        createWeatherOperationGate({
            begin: async (credentials) => (await weatherSettings({ type: "begin", credentials })).sessionId,
            cancel: async (sessionId) => {
                await weatherSettings({ type: "cancel", sessionId });
            },
        }),
    ); // 异步操作身份与后端草稿会话统一由闸管理。
    const opened = useRef(""); // 打开弹窗时的草稿快照，用于判断是否有修改。
    const live = useRef({ dirty: false, busy: "", mounted: true }); // 长生命周期回调读取最新状态。
    const pendingLeave = useRef<() => void>(() => undefined); // 确认后执行的导航。
    // 统一页头展示当前已配置的来源数量；来源列表变化时同步。
    useEffect(() => onStatus?.(sources.length ? `${sources.length} 个已配置来源` : "未配置来源"), [sources.length, onStatus]);
    const formRequired = form ? requiredWeatherFields(form.source, form.adding) : []; // 可留空的字段只在编辑态放宽，新增时仍必须填写。
    const formReady = formRequired.every((key) => draft[key].trim().length > 0); // 保存只要求必填字段非空，不以能力验证为门槛。
    /** 该来源是否已有保存的凭据；城市弹窗的搜索、验证与写入都只走已保存凭据。 */
    const sourceSaved = (source: WeatherSource) => sources.some((item) => item.source === source);
    const formCities = form ? weatherDraftCities(sources.find((item) => item.source === form.source)?.cities ?? [], cityDraft) : []; // 编辑弹窗内的城市列表＝已保存项减去待删除、加上待添加。

    /** 打开城市弹窗；每次进入都不沿用上一次的选择与验证结果。 */
    const openPicker = (source: WeatherSource) => {
        setCity(undefined);
        setChecks([]);
        setStatus("");
        setFailure(false);
        setPicker({ source });
    };

    /** 丢弃当前验证会话并递增前端身份，旧结果一律作废。 */
    const invalidate = () => gate.invalidate();

    /** 动作名称同步渲染状态与关闭保护读取的最新值。 */
    const applyBusy = (kind: string) => {
        live.current.busy = kind;
        setBusy(kind);
    };

    /** 只提交当前来源声明的字段；装配与去空白规则由领域模块统一。 */
    const credentials = () => assembleWeatherCredentials(form?.source, draft);

    /** 字段改变时使会话失效并记录未保存修改。 */
    const handleDraft = (key: WeatherField, value: string) => {
        invalidate();
        setDraft((current) => {
            const next = { ...current, [key]: value }; // 本次编辑后的完整草稿。
            live.current.dirty = weatherDraftChanged(next, opened.current);
            return next;
        });
        setChecks([]);
        setCity(undefined);
        setStatus("");
        setFailure(false);
    };

    /** 打开添加或编辑弹窗；编辑模式只回填非敏感字段，敏感材料一律留空。 */
    const openForm = (source: WeatherSource, adding: boolean) => {
        const saved = adding ? undefined : sources.find((item) => item.source === source); // 已保存来源的列表状态。
        const filled: Record<WeatherField, string> = { ...WEATHER_EMPTY_DRAFT, ...saved?.identity }; // 非敏感字段由后端 identity 回填，不再按来源借道地址。
        invalidate();
        opened.current = JSON.stringify(filled);
        live.current.dirty = false;
        setDraft(filled);
        setVisible(false);
        setChecks([]);
        setCity(undefined);
        setCityDraft(emptyWeatherCityDraft()); // 新开一次编辑，不沿用上一次的城市改动。
        setStatus("");
        setFailure(false);
        setPicker(undefined);
        setForm({ source, adding });
    };

    /** 报告忙碌状态；只上报布尔值，不把打开弹窗的函数交给外部执行，避免刷新时误弹。 */
    useEffect(() => onBusy?.(!!busy), [!!busy, onBusy]);

    /** 页头添加：只在 openAddKey 变化时打开一次新增弹窗，挂载和父级刷新都不触发。 */
    const addRequests = useRef(0); // 已经处理过的页头添加请求数。
    useEffect(() => {
        if (!openAddKey || openAddKey === addRequests.current) return;
        addRequests.current = openAddKey;
        openForm((Object.keys(WEATHER_SOURCES) as WeatherSource[])[0], true);
    }, [openAddKey]);

    /** 关闭弹窗并释放后端草稿；有未保存修改时先确认。 */
    const closeForm = (discard = false) => {
        if (!discard && live.current.dirty) {
            setRemoval({ kind: "discard" });
            return;
        }
        invalidate();
        live.current.dirty = false;
        setForm(undefined);
        setPicker(undefined);
        setDraft(WEATHER_EMPTY_DRAFT);
        setChecks([]);
        setCity(undefined);
        setCityDraft(emptyWeatherCityDraft()); // 丢弃未提交的城市改动，关闭后城市列表回到已保存的状态。
        setStatus(""); // 弹窗内的验证或保存反馈不残留到来源列表。
        setFailure(false);
    };

    /** 串行动作并拒绝取消或卸载后的提交。 */
    const handleAction = async (kind: string, action: (current: number) => Promise<void>) => {
        if (busy) return;
        applyBusy(kind);
        setStatus("");
        setFailure(false);
        const current = gate.nextIdentity(); // 当前操作的前端身份。
        try {
            await action(current);
        } catch (error) {
            if (live.current.mounted && gate.isCurrent(current)) {
                setFailure(true);
                setStatus(typeof error === "string" ? error : "天气设置操作失败，请重试");
            }
        } finally {
            if (live.current.mounted) applyBusy(""); // 成功动作会在收尾中递增身份（关闭弹窗释放草稿），忙碌门槛不跟着作废。
        }
    };

    /** 检测可用并刷新列表状态；该来源没有城市时提示先添加城市，探测仍按已保存凭据执行。 */
    const handleProbe = async (source: WeatherSource) => {
        if (busy) return;
        applyBusy("probe");
        setMenu(undefined);
        setStatus("");
        setFailure(false);
        const current = gate.nextIdentity(); // 当前操作的前端身份。
        try {
            const snapshot = await weatherSettings({ type: "probe", source });
            if (!live.current.mounted || !gate.isCurrent(current)) return;
            setSources(snapshot.sources);
            const name = WEATHER_SOURCES[source].name; // 本次探测的来源名称。
            const status = snapshot.sources.find((item) => item.source === source); // 探测结果里该来源的权威状态。
            const available = status?.availability === "available"; // 按后端写入的可用状态给结论，不假定成功。
            setFailure(!available);
            const cities = status?.cities ?? []; // 探测结果同时给出该来源的城市列表。
            const note = available ? AVAILABILITY.available : AVAILABILITY.unavailable;
            setStatus(cities.length ? `${name} ${note}` : `${name} ${note}，尚未添加城市或地区，请先添加城市`);
        } catch (error) {
            if (live.current.mounted && gate.isCurrent(current)) {
                setFailure(true);
                const reason = typeof error === "string" ? error : "探测失败"; // 探测只确认当前天气一次。
                setStatus(`检测失败：${reason}`);
            }
        } finally {
            if (live.current.mounted) applyBusy(""); // 忙碌门槛必须无条件释放：探测期间打开编辑会递增身份，否则按钮永久停在禁用。
        }
    };

    /** 创建绑定当前字段的后端草稿，并清理迟到创建结果。 */
    const ensureSession = (current: number) => gate.ensure(credentials(), () => live.current.mounted, current);

    /** 城市弹窗的串行入口：忙碌期间拒绝重复提交，失败说明留在弹窗内。 */
    const handleCityAction = async (action: () => Promise<void>) => {
        if (cityBusy) return;
        setCityBusy(true);
        setStatus("");
        setFailure(false);
        try {
            await action();
        } catch (error) {
            if (live.current.mounted) {
                setFailure(true);
                setStatus(typeof error === "string" ? error : "天气设置操作失败，请重试");
            }
        } finally {
            if (live.current.mounted) setCityBusy(false);
        }
    };

    /** 搜索只在提交时执行，结果只回给城市弹窗；会话代次继续由调用方管理。 */
    const handleSearch = async (search: string) => {
        if (cityBusy || !picker) return [];
        setCityBusy(true);
        setCity(undefined);
        setChecks([]);
        setStatus("正在搜索…");
        setFailure(false);
        try {
            const current = gate.nextIdentity(); // 本次搜索的前端身份。
            if (!sourceSaved(picker.source)) {
                setFailure(true);
                setStatus("请先保存该来源的凭据，再添加城市或地区");
                return [];
            }
            const found = await searchWeatherCities(picker.source, search); // 已保存凭据的按来源搜索，不缓存位置检索结果。
            if (!gate.isCurrent(current) || !live.current.mounted) return [];
            setStatus(found.length ? "" : "没有找到该位置，可尝试所属城市或英文名称");
            return found;
        } catch (error) {
            if (live.current.mounted) {
                setFailure(true);
                setStatus(typeof error === "string" ? error : "天气设置操作失败，请重试");
            }
            return [];
        } finally {
            if (live.current.mounted) setCityBusy(false);
        }
    };

    /** 新搜索与改选都作废上一次验证结果，避免确认未验证的城市。 */
    const handleSelect = (next?: WeatherCity) => {
        setCity((current) => (current?.id === next?.id ? current : next));
        setChecks([]);
        setStatus("");
        setFailure(false);
    };

    /** 按已保存凭据验证候选城市：验证只给出可用性结论，写入城市列表在确认时发生。 */
    const handleValidate = (target: WeatherCity) =>
        handleCityAction(async () => {
            if (!picker) return;
            setChecks([]);
            const current = gate.nextIdentity(); // 本次验证的前端身份。
            setStatus("正在验证城市…");
            if (!sourceSaved(picker.source)) {
                setFailure(true);
                setStatus("请先保存该来源的凭据，再验证城市");
                return;
            }
            const result = await validateWeatherCity(picker.source, target); // Rust 按该来源规则校验候选对象，不要求城市已在列表中。
            if (!gate.isCurrent(current) || !live.current.mounted) return;
            setCity(result);
            setChecks([SOURCE_CITY_CHECK]); // 按来源命令只返回结论，不拆分为逐项能力。
            setStatus("验证通过");
        });

    /** 确认时才把候选城市写入该来源城市列表；编辑弹窗内的城市改动留到保存时一起提交。 */
    const handleConfirm = async (target: WeatherCity) =>
        handleCityAction(async () => {
            if (!picker) return;
            if (!sourceSaved(picker.source)) throw new Error("请先保存该来源的凭据，再添加城市或地区");
            if (form) {
                // 编辑弹窗：城市改动属于本次编辑，取消时要一起丢弃，因此只记草稿。
                setCityDraft((current) => confirmWeatherCityDraft(current, target));
                setCity(undefined);
                setChecks([]);
                setStatus("");
                setFailure(false);
                setPicker(undefined);
                return;
            }
            const current = gate.nextIdentity(); // 本次写入的前端身份。
            const added = await addWeatherCity(picker.source, target); // 入列只按该来源规则校验候选对象，不要求城市已在列表中。
            if (!gate.isCurrent(current) || !live.current.mounted) return;
            setSources((items) => items.map((item) => (item.source === picker.source ? { ...item, cities: added } : item)));
            setCity(undefined);
            setChecks([]);
            setStatus("");
            setFailure(false);
            setPicker(undefined);
        });

    /**
     * 保存后的可用性验证：不阻塞保存收尾，结论回来后补写来源可用状态与状态行。
     * 后端按该来源的配置代次丢弃旧结果，因此这里只需确认页面仍然挂载。
     */
    const verifyAfterSave = async (source: WeatherSource, name: string) => {
        try {
            const probed = await weatherSettings({ type: "probe", source }); // 用该来源第一座城市验证一次。
            if (!live.current.mounted) return;
            const availability = probed.sources.find((item) => item.source === source)?.availability; // 本次探测后的权威状态。
            setSources((items) => items.map((item) => (item.source === source ? { ...item, availability: availability ?? item.availability } : item)));
            setFailure(availability !== "available");
            setStatus(availability === "available" ? `已保存 ${name}，${AVAILABILITY.available}` : `已保存 ${name}，检测未通过`);
        } catch (error) {
            if (!live.current.mounted) return;
            setFailure(true);
            setStatus(`已保存 ${name}，检测未通过：${typeof error === "string" ? error : "探测失败"}`); // 凭据已写入，失败只作为结论反馈。
        }
    };

    /**
     * 保存：先提交编辑弹窗里积累的城市改动，再写入凭据。
     * 凭据没有改动时只提交城市改动；保存后两个来源都补一次可用性验证（用该来源第一座城市），
     * 验证不阻塞保存也不阻止写入。
     */
    const handleSave = () =>
        handleAction("save", async (current) => {
            if (!form) return;
            const source = form.source; // 本次保存的来源。
            const name = WEATHER_SOURCES[source].name; // 本次保存的来源名称。
            const untouched = !live.current.dirty && !cityDraft.added.length && !cityDraft.removed.length; // 凭据与城市都没有改动。
            if (untouched) {
                closeForm(true);
                setStatus(`已保存 ${name}`);
                return;
            }
            let cities = sources.find((item) => item.source === source)?.cities ?? []; // 本次提交后的城市列表。
            const steps = weatherCityCommitSteps(cityDraft); // 先添加后移除，与保存顺序一致。
            // 每写入一项立即从草稿移除，凭据保存失败后重试不会重复添加或删除同一城市。
            for (const city of steps.additions) {
                cities = await addWeatherCity(source, city);
                setCityDraft((current) => ({ ...current, added: current.added.filter((item) => item.id !== city.id) }));
            }
            for (const id of steps.removals) {
                cities = await removeWeatherCity(source, id);
                setCityDraft((current) => ({ ...current, removed: current.removed.filter((item) => item !== id) }));
            }
            if (!gate.isCurrent(current) || !live.current.mounted) return;
            let next = sources.map((item) => (item.source === source ? { ...item, cities } : item)); // 无凭据改动时只替换城市列表。
            if (live.current.dirty) {
                const sessionId = gate.sessionId ?? (await ensureSession(current)); // 会话只创建一次，迟到结果会被取消。
                const snapshot = await weatherSettings({ type: "save", sessionId }); // 服务端按草稿字段写入凭据。
                if (!gate.isCurrent(current) || !live.current.mounted) return;
                next = snapshot.sources.map((item) => (item.source === source ? { ...item, cities } : item));
            }
            if (!gate.isCurrent(current) || !live.current.mounted) return;
            setSources(next);
            closeForm(true); // 先收起弹窗，保存不等待网络验证。
            setFailure(false);
            setStatus(`已保存 ${name}`);
            const action = pendingLeave.current; // 未保存确认选择保存时继续原导航。
            pendingLeave.current = () => undefined;
            action();
            void verifyAfterSave(source, name); // 保存后补一次可用性验证，不阻塞保存收尾；两个来源走同一条路径。
        });

    /** 删除来源凭据并刷新列表；失败保留旧配置与确认框。 */
    const handleRemove = () =>
        void handleAction("remove", async (current) => {
            if (removal?.kind !== "source") return;
            const source = removal.source; // 本次删除的来源。
            const snapshot = await weatherSettings({ type: "remove", source });
            if (!gate.isCurrent(current) || !live.current.mounted) return;
            setSources(snapshot.sources);
            setRemoval(undefined);
            setStatus(`已删除 ${WEATHER_SOURCES[source].name}`);
        });

    /** 展开或收起某来源的城市列表；城市数据只保留在来源状态里，这里只切换加载状态。 */
    const handleExpand = async (source: WeatherSource) => {
        if (expanded === source) {
            setExpanded(undefined);
            return;
        }
        setExpanded(source);
        setExpandedLoading(true);
        try {
            const list = await getWeatherCities(source);
            if (!live.current.mounted) return;
            setSources((items) => items.map((item) => (item.source === source ? { ...item, cities: list } : item)));
        } catch (error) {
            if (!live.current.mounted) return;
            setFailure(true);
            setStatus(typeof error === "string" ? error : "天气城市读取失败");
        } finally {
            if (live.current.mounted) setExpandedLoading(false);
        }
    };

    /** 城市项的删除入口：只移除该项，正在使用该城市的格子保留配置并自行进入城市已移除状态。 */
    const requestCityRemoval = (source: WeatherSource, city: WeatherCity) => {
        if (busy) return;
        setRemoval({ kind: "city", source, city });
    };

    /** 城市项右键只打开菜单：删除命令由菜单触发，不再直接弹出确认框。 */
    const openCityMenu = (source: WeatherSource, target: WeatherCity, position: { left: number; top: number }) => {
        if (busy) return;
        setMenu(undefined);
        setCityMenu({ source, city: target, ...position });
    };

    /** 城市项右键菜单只有删除命令；编辑弹窗内与来源展开区共用同一份。 */

    /** 删除城市项：编辑弹窗内只记草稿，其余入口按来源命令立即移除该项。 */
    const handleRemoveCity = (source: WeatherSource, city: WeatherCity) => {
        if (form) {
            // 本次刚加上的城市只撤回 added，不写进 removed，避免保存时删除尚不存在的一行。
            setCityDraft((current) => withdrawWeatherCityDraft(current, city));
            setRemoval(undefined);
            return;
        }
        void handleAction("remove", async (current) => {
            const list = await removeWeatherCity(source, city.id); // 命令返回值即本轮来源，不新增广播事件。
            if (!gate.isCurrent(current) || !live.current.mounted) return;
            setSources((items) => items.map((item) => (item.source === source ? { ...item, cities: list } : item)));
            setRemoval(undefined);
        });
    };

    /** 确认框正文按删除对象区分；命令区的未保存确认复用同一确认框。 */
    const confirmText =
        removal?.kind === "source"
            ? `删除 ${WEATHER_SOURCES[removal.source].name} 在本机保存的凭据，并停止该来源的联网更新。`
            : removal?.kind === "city"
              ? `从城市列表移除 ${removal.city.name}，正在使用该城市的天气格子会提示重新选择。`
              : "关闭弹窗将丢失本次修改。可以取消关闭，或直接保存。";

    /** 二级菜单按该来源已添加的城市创建格子；创建链路由 Rust 侧入口完成。 */
    const handleCreateTile = (source: WeatherSource, city: WeatherCity) => {
        setMenu(undefined);
        void createWeatherTile(source, city.id)
            .then(() => setStatus(`已添加 ${city.name} 天气格子`))
            .catch((error: unknown) => {
                if (!live.current.mounted) return;
                setFailure(true);
                setStatus(typeof error === "string" ? error : "天气格子创建失败");
            });
    };

    useEffect(() => {
        live.current.mounted = true;
        const guard = (action: () => void) => {
            const decision = weatherLeaveDecision(live.current); // 忙碌、未保存与放行共用同一条判定。
            if (decision === "block") return;
            if (decision === "proceed") {
                action();
                return;
            }
            pendingLeave.current = action;
            setRemoval({ kind: "discard" });
        }; // 所有离开入口读取同一份最新状态。
        leaveGuard.current = guard;
        let active = true; // 处理异步监听注册晚于卸载的情况。
        void weatherSettings({ type: "status" })
            .then((snapshot) => {
                if (active) setSources(snapshot.sources);
            })
            .catch(() => {
                if (active) {
                    setFailure(true);
                    setStatus("无法读取 Windows 凭据状态，请重新打开天气设置");
                }
            });
        const stop = getCurrentWindow().onCloseRequested((event) => {
            event.preventDefault();
            guard(() => {
                void getCurrentWindow()
                    .destroy()
                    .catch(() => {
                        setFailure(true);
                        setStatus("设置窗口关闭失败，请重试");
                    });
            });
        }); // 原生关闭、托盘关闭与标题栏共用相同门槛。
        void stop
            .then((unlisten) => {
                if (!active) unlisten();
            })
            .catch(() => {
                if (active) {
                    setFailure(true);
                    setStatus("关闭保护注册失败，请重新打开设置窗口");
                }
            });
        return () => {
            active = false;
            live.current.mounted = false;
            live.current.dirty = false;
            live.current.busy = "";
            gate.invalidate(); // 卸载时释放后端草稿。
            if (leaveGuard.current === guard) leaveGuard.current = undefined;
            void stop.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, [leaveGuard]);
    /** 切换供应商：重置草稿与会话，不沿用上一个来源的验证结果。 */
    const switchSource = (source: WeatherSource) => {
        if (form?.source === source) return;
        invalidate();
        opened.current = JSON.stringify(WEATHER_EMPTY_DRAFT);
        live.current.dirty = false;
        setDraft(WEATHER_EMPTY_DRAFT);
        setForm({ source, adding: true });
        setChecks([]);
        setCity(undefined);
        setStatus("");
        setFailure(false);
    };

    /** 导入本地私钥文件：读取文本后按草稿身份丢弃迟到结果。 */
    const importPrivateKeyFile = (selected: File, key: WeatherField) => {
        void handleAction("import", async (current) => {
            if (selected.size > WEATHER_FIELDS[key].maxLength) throw "私钥文件超过大小限制";
            const text = await selected.text(); // 不保存导入路径。
            if (!gate.isCurrent(current) || !live.current.mounted) return;
            handleDraft(key, text);
            applyBusy("");
        });
    };

    /** 取消离开确认，保留当前草稿。 */
    const cancelDiscard = () => {
        pendingLeave.current = () => undefined;
        setRemoval(undefined);
    };

    /** 放弃草稿并继续原导航。 */
    const discardAndLeave = () => {
        const action = pendingLeave.current; // 先保留导航再丢弃草稿。
        closeForm(true);
        setRemoval(undefined);
        pendingLeave.current = () => undefined;
        action();
    };

    return {
        sources,
        busy,
        cityBusy,
        status,
        setStatus,
        failure,
        setFailure,
        menu,
        setMenu,
        cityMenu,
        setCityMenu,
        form,
        removal,
        setRemoval,
        draft,
        setDraft,
        visible,
        setVisible,
        expandedLoading,
        city,
        setCity,
        checks,
        setChecks,
        picker,
        setPicker,
        expanded,
        formReady,
        formCities,
        confirmText,
        openPicker,
        openForm,
        closeForm,
        handleDraft,
        handleProbe,
        handleSearch,
        handleSelect,
        handleValidate,
        handleConfirm,
        handleSave,
        handleRemove,
        handleExpand,
        requestCityRemoval,
        openCityMenu,
        handleRemoveCity,
        handleCreateTile,
        switchSource,
        importPrivateKeyFile,
        cancelDiscard,
        discardAndLeave,
    };
};
