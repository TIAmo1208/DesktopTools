import { Fragment, useEffect, useRef, useState, type RefObject } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import {
    addWeatherCity,
    createWeatherTile,
    getWeatherCities,
    removeWeatherCity,
    searchWeatherCities,
    validateWeatherCity,
    weatherSettings,
} from "../../bridge/weather";
import { WeatherCityDialog } from "../weather/WeatherCityDialog";
import { SettingsCityList } from "./SettingsCityList";
import { SettingsInfo } from "./SettingsInfo";
import backIcon from "../../../resources/weather-back.svg?raw";
import chevronRightIcon from "../../../resources/weather-chevron-right.svg?raw";
import closeIcon from "../../../resources/weather-close.svg?raw";
import eyeIcon from "../../../resources/weather-eye.svg?raw";
import eyeOffIcon from "../../../resources/weather-eye-off.svg?raw";
import {
    SOURCE_CITY_CHECK,
    WEATHER_SOURCES,
    type WeatherAvailability,
    type WeatherCheck,
    type WeatherCity,
    type WeatherCredentials,
    type WeatherField,
    type WeatherSource,
    type WeatherSourceStatus,
} from "../../contracts/weather";
import "./WeatherSettings.css";

/** 各来源字段的标签、填写说明与输入上限；keepBlankOnEdit 表示编辑时留空代表沿用已保存值。 */
const FIELDS: Record<
    WeatherField,
    { label: string; description: string; maxLength: number; secret?: boolean; multiline?: boolean; keepBlankOnEdit?: boolean }
> = {
    host: {
        label: "API Host",
        maxLength: 253,
        description:
            "从和风天气控制台完整复制 API Host，保留所有域名标签，例如 example.other.qweatherapi.com。仅填写主机名，不包含 https://、端口、路径、查询或片段。",
    },
    developerId: {
        label: "开发者 ID",
        maxLength: 64,
        description: "在和风天气控制台的账户设置中查看开发者 ID（Developer ID），以 Q 开头，共 10 位；不要填写邮箱或项目 ID。",
    },
    projectId: {
        label: "项目 ID",
        maxLength: 64,
        description: "在和风天气控制台的项目管理中创建或选择项目，复制项目 ID（Project ID）。该项目需具有城市搜索与天气预报接口权限。",
    },
    credentialId: {
        label: "凭据 ID",
        maxLength: 64,
        description: "在所选项目下创建 JWT 凭据并上传 Ed25519 公钥，复制对应的凭据 ID（Credential ID）。此处不是 API KEY 或临时 JWT 令牌。",
    },
    privateKey: {
        label: "私钥",
        maxLength: 1024,
        secret: true,
        multiline: true,
        keepBlankOnEdit: true,
        description:
            "按和风天气开发文档的 JWT 身份认证说明生成 Ed25519 密钥对：公钥上传到项目凭据，私钥保留在本机。导入对应的 PKCS#8 PEM 私钥文件，或粘贴包含 BEGIN PRIVATE KEY 和 END PRIVATE KEY 的完整内容；不要填写公钥或 JWT 令牌。",
    },
    apiKey: {
        label: "API key",
        maxLength: 1024,
        secret: true,
        keepBlankOnEdit: true,
        description:
            "在 Foreca My API 页面复制当前套餐的 API key，以 Bearer 令牌提交；该密钥是较长的签名令牌，需要完整复制，不要填入账户邮箱、密码或试用链接。",
    },
};

// 弹窗关闭后恢复的空白草稿。
const EMPTY_DRAFT: Record<WeatherField, string> = { host: "", developerId: "", projectId: "", credentialId: "", privateKey: "", apiKey: "" };

/** 图标资源以原始 SVG 保存，尺寸与颜色由所在按键的样式决定。 */
const Glyph = ({ source }: { source: string }) => <span className="weather-settings__glyph" aria-hidden="true" dangerouslySetInnerHTML={{ __html: source }} />;

// 列表可用状态文案；颜色由主题语义色负责。
const AVAILABILITY: Record<WeatherAvailability, string> = { available: "可用", unavailable: "不可用", unconfirmed: "未确认" };

/** 城市弹窗的调用方：各入口都只携带来源，搜索与验证统一使用该来源已保存的凭据。 */
interface CityPicker {
    source: WeatherSource; // 本次弹窗管理的来源。
}

/** 待确认的删除对象：来源凭据与城市项共用同一个确认框，未保存离开复用同一状态位。 */
type WeatherPendingRemoval = { kind: "source"; source: WeatherSource } | { kind: "city"; source: WeatherSource; city: WeatherCity } | { kind: "discard" };

/** 天气来源列表、右键菜单与添加/编辑弹窗；凭据不进入普通偏好保存。 */
export const WeatherSettings = ({ onBack, leaveGuard }: { onBack: () => void; leaveGuard: RefObject<((action: () => void) => void) | undefined> }) => {
    const [sources, setSources] = useState<WeatherSourceStatus[]>([]); // 已配置来源列表。
    const [busy, setBusy] = useState(""); // 当前动作名称。
    const [cityBusy, setCityBusy] = useState(false); // 城市弹窗是否正在执行动作。
    const [status, setStatus] = useState(""); // 脱敏的状态反馈。
    const [failure, setFailure] = useState(false); // 错误色只辅助文本。
    const [menu, setMenu] = useState<{ source: WeatherSource; left: number; top: number }>(); // 跟随鼠标的来源菜单。
    const [form, setForm] = useState<{ source: WeatherSource; adding: boolean }>(); // 添加或编辑弹窗。
    const [removal, setRemoval] = useState<WeatherPendingRemoval>(); // 待确认的删除对象。
    const [draft, setDraft] = useState<Record<WeatherField, string>>(EMPTY_DRAFT); // 只在当前页内存保存。
    const [visible, setVisible] = useState(false); // 仅当前敏感草稿显隐。
    const [expandedLoading, setExpandedLoading] = useState(false); // 当前展开项的城市列表是否正在读取。
    const [city, setCity] = useState<WeatherCity>(); // 编辑弹窗内经完整验证的城市。
    const [cityDraft, setCityDraft] = useState<{ added: WeatherCity[]; removed: string[] }>({ added: [], removed: [] }); // 编辑弹窗内尚未提交的城市改动。
    const [checks, setChecks] = useState<WeatherCheck[]>([]); // 编辑弹窗内本次验证的真实检查结果。
    const [picker, setPicker] = useState<CityPicker>(); // 城市弹窗及其调用方。
    const [expanded, setExpanded] = useState<WeatherSource>(); // 当前展开城市列表的来源。
    const formDialog = useRef<HTMLDialogElement>(null); // 添加与编辑弹窗元素。
    const confirmDialog = useRef<HTMLDialogElement>(null); // 确认框元素。
    const file = useRef<HTMLInputElement>(null); // 本地私钥导入入口。
    const session = useRef<number | null>(null); // 后端会话身份。
    const revision = useRef(0); // 取消和字段改变时拒绝旧结果。
    const opened = useRef(""); // 打开弹窗时的草稿快照，用于判断是否有修改。
    const live = useRef({ dirty: false, busy: "", mounted: true }); // 长生命周期回调读取最新状态。
    const pendingLeave = useRef<() => void>(() => undefined); // 确认后执行的导航。
    const formRequired = form ? WEATHER_SOURCES[form.source].fields.filter((key) => form.adding || !FIELDS[key].keepBlankOnEdit) : []; // 可留空的字段只在编辑态放宽，新增时仍必须填写。
    const formReady = formRequired.every((key) => draft[key].trim().length > 0); // 保存只要求必填字段非空，不以能力验证为门槛。
    /** 该来源是否已有保存的凭据；城市弹窗的搜索、验证与写入都只走已保存凭据。 */
    const sourceSaved = (source: WeatherSource) => sources.some((item) => item.source === source);
    const formCities = form
        ? [...(sources.find((item) => item.source === form.source)?.cities ?? []).filter((item) => !cityDraft.removed.includes(item.id)), ...cityDraft.added]
        : []; // 编辑弹窗内的城市列表＝已保存项减去待删除、加上待添加。

    /** 打开城市弹窗；每次进入都不沿用上一次的选择与验证结果。 */
    const openPicker = (source: WeatherSource) => {
        setCity(undefined);
        setChecks([]);
        setStatus("");
        setFailure(false);
        setPicker({ source });
    };

    /** 丢弃当前验证会话并递增前端身份，旧结果一律作废。 */
    const invalidate = () => {
        revision.current++;
        const previous = session.current; // 不允许取消迟到的新会话。
        session.current = null;
        if (previous !== null) void weatherSettings({ type: "cancel", sessionId: previous }).catch(() => undefined);
    };

    /** 动作名称同步渲染状态与关闭保护读取的最新值。 */
    const applyBusy = (kind: string) => {
        live.current.busy = kind;
        setBusy(kind);
    };

    /** 只提交当前来源声明的字段；所有字段统一去掉首尾空白，不按字段名或来源特判。 */
    const credentials = (): WeatherCredentials => {
        const fields = form ? WEATHER_SOURCES[form.source].fields : WEATHER_SOURCES.qweather.fields; // 没有打开弹窗时按第一个来源装配，调用方本就不会用到。
        const values = Object.fromEntries(fields.map((key) => [key, draft[key].trim()])) as Record<WeatherField, string>;
        return values as unknown as WeatherCredentials; // 后端按字段结构判定来源，这里只负责按来源装配字段。
    };

    /** 字段改变时使会话失效并记录未保存修改。 */
    const handleDraft = (key: WeatherField, value: string) => {
        invalidate();
        setDraft((current) => {
            const next = { ...current, [key]: value }; // 本次编辑后的完整草稿。
            live.current.dirty = JSON.stringify(next) !== opened.current;
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
        const filled: Record<WeatherField, string> = { ...EMPTY_DRAFT, ...saved?.identity }; // 非敏感字段由后端 identity 回填，不再按来源借道地址。
        invalidate();
        opened.current = JSON.stringify(filled);
        live.current.dirty = false;
        setDraft(filled);
        setVisible(false);
        setChecks([]);
        setCity(undefined);
        setCityDraft({ added: [], removed: [] }); // 新开一次编辑，不沿用上一次的城市改动。
        setStatus("");
        setFailure(false);
        setPicker(undefined);
        setForm({ source, adding });
    };

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
        setDraft(EMPTY_DRAFT);
        setChecks([]);
        setCity(undefined);
        setCityDraft({ added: [], removed: [] }); // 丢弃未提交的城市改动，关闭后城市列表回到已保存的状态。
        setStatus(""); // 弹窗内的验证或保存反馈不残留到来源列表。
        setFailure(false);
    };

    /** 串行动作并拒绝取消或卸载后的提交。 */
    const handleAction = async (kind: string, action: (current: number) => Promise<void>) => {
        if (busy) return;
        applyBusy(kind);
        setStatus("");
        setFailure(false);
        const current = ++revision.current; // 当前操作的前端身份。
        try {
            await action(current);
        } catch (error) {
            if (live.current.mounted && current === revision.current) {
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
        const current = ++revision.current; // 当前操作的前端身份。
        try {
            const snapshot = await weatherSettings({ type: "probe", source });
            if (!live.current.mounted || current !== revision.current) return;
            setSources(snapshot.sources);
            const name = WEATHER_SOURCES[source].name; // 本次探测的来源名称。
            const status = snapshot.sources.find((item) => item.source === source); // 探测结果里该来源的权威状态。
            const available = status?.availability === "available"; // 按后端写入的可用状态给结论，不假定成功。
            setFailure(!available);
            const cities = status?.cities ?? []; // 探测结果同时给出该来源的城市列表。
            const note = available ? AVAILABILITY.available : AVAILABILITY.unavailable;
            setStatus(cities.length ? `${name} ${note}` : `${name} ${note}，尚未添加城市或地区，请先添加城市`);
        } catch (error) {
            if (live.current.mounted && current === revision.current) {
                setFailure(true);
                const reason = typeof error === "string" ? error : "探测失败"; // 探测只确认当前天气一次。
                setStatus(`检测失败：${reason}`);
            }
        } finally {
            if (live.current.mounted) applyBusy(""); // 忙碌门槛必须无条件释放：探测期间打开编辑会递增身份，否则按钮永久停在禁用。
        }
    };

    /** 创建绑定当前字段的后端草稿，并清理迟到创建结果。 */
    const ensureSession = async (current: number) => {
        if (session.current !== null) return session.current;
        const result = await weatherSettings({ type: "begin", credentials: credentials() }); // 仅明确搜索或验证时上传。
        if (result.sessionId === null) throw new Error("会话创建失败");
        if (!live.current.mounted || current !== revision.current) {
            void weatherSettings({ type: "cancel", sessionId: result.sessionId }).catch(() => undefined);
            throw new Error("操作已取消");
        }
        session.current = result.sessionId;
        return result.sessionId;
    };

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
            const current = ++revision.current; // 本次搜索的前端身份。
            if (!sourceSaved(picker.source)) {
                setFailure(true);
                setStatus("请先保存该来源的凭据，再添加城市或地区");
                return [];
            }
            const found = await searchWeatherCities(picker.source, search); // 已保存凭据的按来源搜索，不缓存位置检索结果。
            if (current !== revision.current || !live.current.mounted) return [];
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
            const current = ++revision.current; // 本次验证的前端身份。
            setStatus("正在验证天气能力…");
            if (!sourceSaved(picker.source)) {
                setFailure(true);
                setStatus("请先保存该来源的凭据，再验证城市");
                return;
            }
            const result = await validateWeatherCity(picker.source, target); // Rust 按该来源规则校验候选对象，不要求城市已在列表中。
            if (current !== revision.current || !live.current.mounted) return;
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
                setCityDraft((current) => ({ ...current, added: [...current.added.filter((item) => item.id !== target.id), target] }));
                setCity(undefined);
                setChecks([]);
                setStatus("");
                setFailure(false);
                setPicker(undefined);
                return;
            }
            const current = ++revision.current; // 本次写入的前端身份。
            const added = await addWeatherCity(picker.source, target); // 入列只按该来源规则校验候选对象，不要求城市已在列表中。
            if (current !== revision.current || !live.current.mounted) return;
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
            // 每写入一项立即从草稿移除，凭据保存失败后重试不会重复添加或删除同一城市。
            for (const city of [...cityDraft.added]) {
                cities = await addWeatherCity(source, city);
                setCityDraft((current) => ({ ...current, added: current.added.filter((item) => item.id !== city.id) }));
            }
            for (const id of [...cityDraft.removed]) {
                cities = await removeWeatherCity(source, id);
                setCityDraft((current) => ({ ...current, removed: current.removed.filter((item) => item !== id) }));
            }
            if (current !== revision.current || !live.current.mounted) return;
            let next = sources.map((item) => (item.source === source ? { ...item, cities } : item)); // 无凭据改动时只替换城市列表。
            if (live.current.dirty) {
                if (session.current === null) await ensureSession(current);
                if (session.current === null) throw new Error("会话创建失败");
                const snapshot = await weatherSettings({ type: "save", sessionId: session.current }); // 服务端按草稿字段写入凭据。
                if (current !== revision.current || !live.current.mounted) return;
                next = snapshot.sources.map((item) => (item.source === source ? { ...item, cities } : item));
            }
            if (current !== revision.current || !live.current.mounted) return;
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
            if (current !== revision.current || !live.current.mounted) return;
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

    /** 删除城市项：编辑弹窗内只记草稿，其余入口按来源命令立即移除该项。 */
    const handleRemoveCity = (source: WeatherSource, city: WeatherCity) => {
        if (form) {
            // 本次刚加上的城市只需撤回 added，不能同时记入 removed，否则保存时会删除尚不存在的一行。
            setCityDraft((current) => ({
                added: current.added.filter((item) => item.id !== city.id),
                removed:
                    current.added.some((item) => item.id === city.id) || current.removed.includes(city.id) ? current.removed : [...current.removed, city.id],
            }));
            setRemoval(undefined);
            return;
        }
        void handleAction("remove", async (current) => {
            const list = await removeWeatherCity(source, city.id); // 命令返回值即本轮来源，不新增广播事件。
            if (current !== revision.current || !live.current.mounted) return;
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
        if (!menu) return;
        const close = () => setMenu(undefined); // 点击菜单以外区域立即收起。
        document.addEventListener("pointerdown", close);
        return () => document.removeEventListener("pointerdown", close);
    }, [menu]);

    useEffect(() => {
        live.current.mounted = true;
        const guard = (action: () => void) => {
            if (live.current.busy === "save" || live.current.busy === "remove") return;
            if (!live.current.dirty) {
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
            revision.current++;
            const previous = session.current; // 卸载时释放后端草稿。
            session.current = null;
            if (previous !== null) void weatherSettings({ type: "cancel", sessionId: previous }).catch(() => undefined);
            if (leaveGuard.current === guard) leaveGuard.current = undefined;
            void stop.then((unlisten) => unlisten()).catch(() => undefined);
        };
    }, [leaveGuard]);

    useEffect(() => {
        for (const [element, open] of [
            [formDialog.current, !!form],
            [confirmDialog.current, !!removal],
        ] as const) {
            if (!element) continue;
            if (open && !element.open) element.showModal(); // 城市弹窗在编辑弹窗之上独立打开。
            if (!open && element.open) element.close();
        }
    }, [form, removal]);

    return (
        <section className="weather-settings">
            <div className="weather-settings__heading">
                <button
                    type="button"
                    className="weather-settings__icon"
                    title="返回功能格子"
                    aria-label="返回功能格子"
                    onClick={() => leaveGuard.current?.(onBack)}
                >
                    <Glyph source={backIcon} />
                </button>
                <h2>天气</h2>
            </div>
            <h3>已配置的天气来源</h3>
            {sources.length ? (
                <ul className="weather-settings__list">
                    {sources.map((item) => (
                        <li key={item.source} className="weather-settings__item">
                            <div
                                className="weather-settings__item-row"
                                role="button"
                                tabIndex={0}
                                title="显示详情"
                                onClick={() => openForm(item.source, false)}
                                onKeyDown={(event) => {
                                    if (event.key !== "Enter" && event.key !== " ") return;
                                    event.preventDefault();
                                    openForm(item.source, false);
                                }}
                                onContextMenu={(event) => {
                                    event.preventDefault();
                                    setMenu({
                                        source: item.source,
                                        left: Math.min(event.clientX, window.innerWidth - 240),
                                        top: Math.min(event.clientY, window.innerHeight - 132),
                                    });
                                }}
                            >
                                <span className="weather-settings__item-main">
                                    <span>{WEATHER_SOURCES[item.source].name}</span>
                                    <small>{item.address}</small>
                                </span>
                                <span className="weather-settings__mode">{WEATHER_SOURCES[item.source].mode}</span>
                                <span className={`weather-settings__state is-${item.availability}`}>
                                    <i aria-hidden="true" />
                                    {AVAILABILITY[item.availability]}
                                </span>
                                {(() => {
                                    const open = expanded === item.source; // 该项是否已展开城市列表。
                                    return (
                                        <button
                                            type="button"
                                            className={`weather-settings__expand${open ? " is-open" : ""}`}
                                            aria-expanded={open}
                                            aria-label={`${open ? "收起" : "展开"} ${WEATHER_SOURCES[item.source].name} 的城市列表`}
                                            title={`${open ? "收起" : "展开"}城市列表`}
                                            onClick={(event) => {
                                                event.stopPropagation(); // 展开按键只负责展开收起，不触发进入编辑。
                                                void handleExpand(item.source);
                                            }}
                                        >
                                            <Glyph source={chevronRightIcon} />
                                        </button>
                                    );
                                })()}
                                <span className="weather-settings__arrow" aria-hidden="true">
                                    <Glyph source={chevronRightIcon} />
                                </span>
                            </div>
                            {expanded === item.source && (
                                <div className="weather-settings__cities">
                                    {expandedLoading ? (
                                        <p className="weather-settings__status">正在读取城市列表…</p>
                                    ) : (
                                        <SettingsCityList
                                            cities={item.cities}
                                            busy={!!busy}
                                            onDelete={(target) => requestCityRemoval(item.source, target)}
                                            onAdd={() => openPicker(item.source)}
                                        />
                                    )}
                                </div>
                            )}
                        </li>
                    ))}
                </ul>
            ) : (
                <p className="weather-settings__empty">尚未添加任何天气来源</p>
            )}
            <button
                type="button"
                className="weather-settings__add"
                disabled={!!busy}
                onClick={() => openForm((Object.keys(WEATHER_SOURCES) as WeatherSource[])[0], true)}
            >
                + 添加 API
            </button>
            {!form && (
                <p className={failure ? "weather-settings__error" : "weather-settings__status"} role="status">
                    {status}
                </p>
            )}
            {menu && (
                <menu
                    className="desktop-tile-menu weather-settings__menu"
                    role="menu"
                    style={{ left: menu.left, top: menu.top }}
                    onPointerDown={(event) => event.stopPropagation()}
                >
                    <li role="none">
                        <button
                            type="button"
                            role="menuitem"
                            onClick={() => {
                                const source = menu.source; // 先关闭菜单再打开弹窗。
                                setMenu(undefined);
                                openForm(source, false);
                            }}
                        >
                            显示详情
                        </button>
                    </li>
                    <li role="none">
                        <button type="button" role="menuitem" disabled={!!busy} onClick={() => void handleProbe(menu.source)}>
                            检测可用
                        </button>
                    </li>
                    <li className="weather-settings__menu-separator" role="separator" />
                    <li role="none">
                        <button
                            type="button"
                            role="menuitem"
                            disabled={!!busy}
                            onClick={() => {
                                const source = menu.source; // 先关闭菜单再打开弹窗。
                                setMenu(undefined);
                                openPicker(source);
                            }}
                        >
                            添加城市
                        </button>
                    </li>
                    <li role="none" className="weather-settings__submenu-host">
                        <button type="button" role="menuitem" aria-haspopup="menu" onClick={() => undefined}>
                            添加天气格子
                            <span className="weather-settings__submenu-arrow" aria-hidden="true">
                                <Glyph source={chevronRightIcon} />
                            </span>
                        </button>
                        <menu className="weather-settings__submenu desktop-tile-menu" role="menu" aria-label="添加天气格子">
                            {(() => {
                                const cities = sources.find((item) => item.source === menu.source)?.cities ?? []; // 该来源已添加的城市。
                                if (!cities.length)
                                    return (
                                        <li role="none">
                                            <span className="desktop-tile-menu__heading">列表为空</span>
                                        </li>
                                    );
                                return cities.map((item, index) => (
                                    <Fragment key={item.id}>
                                        {index === 0 && <li className="desktop-tile-menu__separator" role="separator" aria-hidden="true" />}
                                        <li role="none">
                                            <button type="button" role="menuitem" onClick={() => handleCreateTile(menu.source, item)}>
                                                {item.name}
                                            </button>
                                        </li>
                                    </Fragment>
                                ));
                            })()}
                        </menu>
                    </li>
                    <li className="weather-settings__menu-separator" role="separator" />
                    <li role="none">
                        <button
                            type="button"
                            role="menuitem"
                            className="weather-settings__menu-danger"
                            onClick={() => {
                                setRemoval({ kind: "source", source: menu.source });
                                setMenu(undefined);
                            }}
                        >
                            删除
                        </button>
                    </li>
                </menu>
            )}
            {form && (
                <dialog
                    ref={formDialog}
                    className="weather-settings__dialog"
                    aria-label={form.adding ? "添加 API" : `编辑 ${WEATHER_SOURCES[form.source].name}`}
                    onCancel={(event) => {
                        event.preventDefault();
                        if (busy !== "save") closeForm();
                    }}
                >
                    <header className="weather-settings__dialog-head">
                        <h2>{form.adding ? "添加 API" : `编辑 ${WEATHER_SOURCES[form.source].name}`}</h2>
                        <button
                            type="button"
                            className="weather-settings__close"
                            aria-label="关闭"
                            title="关闭"
                            disabled={busy === "save"}
                            onClick={() => closeForm()}
                        >
                            <Glyph source={closeIcon} />
                        </button>
                    </header>
                    <div className="weather-settings__dialog-body">
                        {form.adding && (
                            <div className="weather-settings__row">
                                <span>供应商</span>
                                <div className="weather-settings__segmented" role="tablist" aria-label="供应商">
                                    {(Object.keys(WEATHER_SOURCES) as WeatherSource[]).map((item) => (
                                        <button
                                            key={item}
                                            type="button"
                                            role="tab"
                                            aria-selected={item === form.source}
                                            className={`weather-settings__segment${item === form.source ? " is-active" : ""}`}
                                            disabled={!!busy}
                                            onClick={() => {
                                                if (item === form.source) return;
                                                invalidate();
                                                opened.current = JSON.stringify(EMPTY_DRAFT);
                                                live.current.dirty = false;
                                                setDraft(EMPTY_DRAFT);
                                                setForm({ source: item, adding: true });
                                                setChecks([]);
                                                setCity(undefined);
                                                setStatus("");
                                                setFailure(false);
                                            }}
                                        >
                                            {WEATHER_SOURCES[item].name}
                                        </button>
                                    ))}
                                </div>
                            </div>
                        )}
                        {WEATHER_SOURCES[form.source].fields.map((key) =>
                            FIELDS[key].multiline ? (
                                <div className="weather-settings__row weather-settings__row--secret" key={key}>
                                    <span>
                                        <label htmlFor={`weather-${key}`}>{FIELDS[key].label}</label>
                                        <SettingsInfo label={FIELDS[key].label} description={FIELDS[key].description} />
                                    </span>
                                    <div className="weather-settings__secret">
                                        <div className="weather-settings__secret-input">
                                            <textarea
                                                id={`weather-${key}`}
                                                className={visible ? "" : "is-masked"}
                                                autoComplete="off"
                                                spellCheck={false}
                                                maxLength={FIELDS[key].maxLength}
                                                placeholder={form.adding ? "" : "已配置，输入新值代表替换"}
                                                disabled={!!busy}
                                                value={draft[key]}
                                                onPaste={(event) => {
                                                    event.preventDefault();
                                                    handleDraft(key, event.clipboardData.getData("text").slice(0, FIELDS[key].maxLength));
                                                }}
                                                onChange={(event) => handleDraft(key, event.currentTarget.value)}
                                            />
                                            <button
                                                type="button"
                                                className="weather-settings__icon"
                                                aria-label={visible ? "隐藏私钥草稿" : "显示私钥草稿"}
                                                title={visible ? "隐藏私钥草稿" : "显示私钥草稿"}
                                                aria-pressed={visible}
                                                onClick={() => setVisible(!visible)}
                                            >
                                                {visible ? <Glyph source={eyeOffIcon} /> : <Glyph source={eyeIcon} />}
                                            </button>
                                        </div>
                                        <div className="weather-settings__actions">
                                            <button type="button" disabled={!!busy} onClick={() => file.current?.click()}>
                                                导入私钥
                                            </button>
                                            <small>{draft.privateKey ? "已填写草稿" : form.adding ? "" : "已保存的私钥不回读"}</small>
                                        </div>
                                        <input
                                            ref={file}
                                            type="file"
                                            hidden
                                            accept=".pem,.key"
                                            onChange={(event) => {
                                                const selected = event.currentTarget.files?.[0]; // 读取用户明确选择的本地文件。
                                                event.currentTarget.value = "";
                                                if (!selected) return;
                                                void handleAction("import", async (current) => {
                                                    if (selected.size > FIELDS[key].maxLength) throw "私钥文件超过大小限制";
                                                    const text = await selected.text(); // 不保存导入路径。
                                                    if (current !== revision.current || !live.current.mounted) return;
                                                    handleDraft(key, text);
                                                    applyBusy("");
                                                });
                                            }}
                                        />
                                    </div>
                                </div>
                            ) : (
                                <div className="weather-settings__row" key={key}>
                                    <span>
                                        <label htmlFor={`weather-${key}`}>{FIELDS[key].label}</label>
                                        <SettingsInfo label={FIELDS[key].label} description={FIELDS[key].description} />
                                    </span>
                                    {FIELDS[key].secret ? (
                                        <div className="weather-settings__secret-input">
                                            <input
                                                id={`weather-${key}`}
                                                type={visible ? "text" : "password"}
                                                autoComplete="new-password"
                                                spellCheck={false}
                                                maxLength={FIELDS[key].maxLength}
                                                placeholder={form.adding ? "" : "已配置，输入新值代表替换"}
                                                disabled={!!busy}
                                                value={draft[key]}
                                                onChange={(event) => handleDraft(key, event.currentTarget.value)}
                                            />
                                            <button
                                                type="button"
                                                className="weather-settings__icon"
                                                aria-label={visible ? "隐藏草稿" : "显示草稿"}
                                                title={visible ? "隐藏草稿" : "显示草稿"}
                                                aria-pressed={visible}
                                                onClick={() => setVisible(!visible)}
                                            >
                                                {visible ? <Glyph source={eyeOffIcon} /> : <Glyph source={eyeIcon} />}
                                            </button>
                                        </div>
                                    ) : (
                                        <input
                                            id={`weather-${key}`}
                                            autoComplete="off"
                                            spellCheck={false}
                                            maxLength={FIELDS[key].maxLength}
                                            disabled={!!busy}
                                            value={draft[key]}
                                            onChange={(event) => handleDraft(key, event.currentTarget.value)}
                                        />
                                    )}
                                </div>
                            ),
                        )}
                        {!form.adding && (
                            <>
                                <h3>城市列表</h3>
                                <SettingsCityList
                                    cities={formCities}
                                    busy={!!busy}
                                    onDelete={(target) => requestCityRemoval(form.source, target)}
                                    onAdd={() => {
                                        setStatus("");
                                        openPicker(form.source); // 城市列表分组按来源命令添加，与来源展开区同一条后端路径。
                                    }}
                                />
                            </>
                        )}
                    </div>
                    <footer className="weather-settings__actions weather-settings__dialog-actions">
                        <span className={`weather-settings__dialog-status${failure ? " weather-settings__error" : ""}`} role="status">
                            {status}
                        </span>
                        <button type="button" className="weather-settings__primary" disabled={!formReady || !!busy} onClick={() => void handleSave()}>
                            {busy === "save" ? "正在保存" : "保存"}
                        </button>
                        <button type="button" disabled={busy === "save"} onClick={() => closeForm()}>
                            取消
                        </button>
                    </footer>
                </dialog>
            )}
            {picker && (
                <WeatherCityDialog
                    title="添加城市"
                    type="add"
                    modal // 设置窗口的配置弹窗是顶层模态，城市弹窗必须是同一层才叠得上去。
                    state={{ city, checks, status, failure }}
                    busy={cityBusy || busy === "save"}
                    onSearch={handleSearch}
                    onSelect={handleSelect}
                    onValidate={handleValidate}
                    onConfirm={handleConfirm}
                    onClose={() => {
                        setCity(undefined);
                        setChecks([]);
                        setStatus("");
                        setFailure(false);
                        setPicker(undefined);
                    }}
                />
            )}
            {removal && (
                <dialog
                    ref={confirmDialog}
                    className="weather-settings__dialog weather-settings__dialog--confirm"
                    aria-label={removal.kind === "discard" ? "配置尚未保存" : "确认删除"}
                    onCancel={(event) => {
                        event.preventDefault();
                        if (busy === "remove") return;
                        setRemoval(undefined);
                    }}
                >
                    <header className="weather-settings__dialog-head">
                        <h2>{removal.kind === "source" ? "删除天气来源？" : removal.kind === "city" ? "删除城市或地区？" : "配置尚未保存"}</h2>
                    </header>
                    <div className="weather-settings__dialog-body">
                        <p>{confirmText}</p>
                    </div>
                    <footer className="weather-settings__actions weather-settings__dialog-actions">
                        {removal.kind === "discard" ? (
                            <>
                                <button
                                    type="button"
                                    autoFocus
                                    disabled={busy === "save"}
                                    onClick={() => {
                                        pendingLeave.current = () => undefined;
                                        setRemoval(undefined);
                                    }}
                                >
                                    取消关闭
                                </button>
                                <button type="button" className="weather-settings__primary" disabled={!formReady || !!busy} onClick={handleSave}>
                                    保存
                                </button>
                                <button
                                    type="button"
                                    disabled={busy === "save"}
                                    onClick={() => {
                                        const action = pendingLeave.current; // 先保留导航再丢弃草稿。
                                        closeForm(true);
                                        setRemoval(undefined);
                                        pendingLeave.current = () => undefined;
                                        action();
                                    }}
                                >
                                    不保存
                                </button>
                            </>
                        ) : (
                            <>
                                <button type="button" autoFocus disabled={busy === "remove"} onClick={() => setRemoval(undefined)}>
                                    取消
                                </button>
                                <button
                                    type="button"
                                    className="weather-settings__danger"
                                    disabled={busy === "remove"}
                                    onClick={() => {
                                        if (removal.kind === "source") void handleRemove();
                                        else void handleRemoveCity(removal.source, removal.city);
                                    }}
                                >
                                    {busy === "remove" ? "正在删除" : "删除"}
                                </button>
                            </>
                        )}
                    </footer>
                </dialog>
            )}
        </section>
    );
};
