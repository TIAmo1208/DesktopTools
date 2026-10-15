import { useEffect, useRef, useState, type RefObject } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { weatherSettings } from "../../bridge/weather";
import { SettingsInfo } from "./SettingsInfo";
import { SettingsSwitch } from "./SettingsSwitch";
import { BackIcon, ChevronRightIcon, CloseIcon, ConcealIcon, PassedIcon, RevealIcon, SearchIcon } from "./WeatherIcons";
import type { ApplicationPreferences } from "../../contracts/settings";
import {
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

/** 各来源字段的标签、填写说明与输入上限，随供应商切换展示。 */
const FIELDS: Record<WeatherField, { label: string; description: string; maxLength: number; secret?: boolean }> = {
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
        description:
            "按和风天气开发文档的 JWT 身份认证说明生成 Ed25519 密钥对：公钥上传到项目凭据，私钥保留在本机。导入对应的 PKCS#8 PEM 私钥文件，或粘贴包含 BEGIN PRIVATE KEY 和 END PRIVATE KEY 的完整内容；不要填写公钥或 JWT 令牌。",
    },
    apiKey: {
        label: "API key",
        maxLength: 1024,
        secret: true,
        description:
            "在 Foreca My API 页面复制当前套餐的 API key，以 Bearer 令牌提交；该密钥是较长的签名令牌，需要完整复制，不要填入账户邮箱、密码或试用链接。",
    },
};

// 弹窗关闭后恢复的空白草稿。
const EMPTY_DRAFT: Record<WeatherField, string> = { host: "", developerId: "", projectId: "", credentialId: "", privateKey: "", apiKey: "" };

// 列表可用状态文案；颜色由主题语义色负责。
const AVAILABILITY: Record<WeatherAvailability, string> = { available: "可用", unavailable: "不可用", unconfirmed: "未确认" };

/** 天气来源列表、右键菜单与添加/编辑弹窗；凭据不进入普通偏好保存。 */
export const WeatherSettings = ({
    onBack,
    leaveGuard,
    preferences,
    onPreferences,
}: {
    onBack: () => void; // 返回功能格子列表。
    leaveGuard: RefObject<((action: () => void) => void) | undefined>; // 未保存草稿离开保护。
    preferences?: ApplicationPreferences; // 已保存偏好，提供当前启用来源。
    onPreferences: (patch: Partial<ApplicationPreferences>) => void; // 保存启用来源。
}) => {
    const [sources, setSources] = useState<WeatherSourceStatus[]>([]); // 已配置来源列表。
    const [busy, setBusy] = useState(""); // 当前动作名称。
    const [status, setStatus] = useState(""); // 脱敏的状态反馈。
    const [failure, setFailure] = useState(false); // 错误色只辅助文本。
    const [menu, setMenu] = useState<{ source: WeatherSource; left: number; top: number }>(); // 跟随鼠标的来源菜单。
    const [form, setForm] = useState<{ source: WeatherSource; adding: boolean }>(); // 添加或编辑弹窗。
    const [confirm, setConfirm] = useState<"remove" | "discard">(); // 删除与未保存确认。
    const [pending, setPending] = useState<WeatherSource>(); // 待删除来源。
    const [draft, setDraft] = useState<Record<WeatherField, string>>(EMPTY_DRAFT); // 只在当前页内存保存。
    const [visible, setVisible] = useState(false); // 仅当前敏感草稿显隐。
    const [city, setCity] = useState<WeatherCity>(); // 当前临时验证城市。
    const [cities, setCities] = useState<WeatherCity[]>([]); // 当前搜索结果。
    const [query, setQuery] = useState(""); // 手动提交搜索。
    const [checks, setChecks] = useState<WeatherCheck[]>([]); // 当前来源的真实检查结果。
    const [picker, setPicker] = useState(false); // 城市选择弹窗。
    const formDialog = useRef<HTMLDialogElement>(null); // 添加与编辑弹窗元素。
    const pickerDialog = useRef<HTMLDialogElement>(null); // 城市选择弹窗元素。
    const confirmDialog = useRef<HTMLDialogElement>(null); // 确认框元素。
    const file = useRef<HTMLInputElement>(null); // 本地私钥导入入口。
    const session = useRef<number | null>(null); // 后端会话身份。
    const revision = useRef(0); // 取消和字段改变时拒绝旧结果。
    const opened = useRef(""); // 打开弹窗时的草稿快照，用于判断是否有修改。
    const live = useRef({ dirty: false, busy: "", mounted: true }); // 长生命周期回调读取最新状态。
    const pendingLeave = useRef<() => void>(() => undefined); // 确认后执行的导航。
    const complete = form ? WEATHER_SOURCES[form.source].fields.every((key) => (draft[key] ?? "").trim()) : false; // 仅控制本地按钮，后端仍独立校验。
    const verified = checks.length > 0 && checks.every((check) => check.failure === null); // 仅展示和禁用用途。
    const activeSource = preferences?.weatherSource ?? sources.find((item) => item.enabled)?.source; // 已保存偏好优先，加载前回退后端状态。
    const enabled = sources.find((item) => item.source === activeSource); // 当前启用来源的列表状态。

    /** 丢弃当前验证会话并递增前端身份，旧结果一律作废。 */
    const invalidate = () => {
        revision.current++;
        const previous = session.current; // 不允许取消迟到的新会话。
        session.current = null;
        if (previous !== null) void weatherSettings({ type: "cancel", sessionId: previous }).catch(() => undefined);
    };

    /** 只提交当前来源声明的字段，多余字段不发送。 */
    const credentials = (): WeatherCredentials =>
        form?.source === "foreca"
            ? { apiKey: draft.apiKey.trim() }
            : {
                  host: draft.host.trim(),
                  developerId: draft.developerId.trim(),
                  projectId: draft.projectId.trim(),
                  credentialId: draft.credentialId.trim(),
                  privateKey: draft.privateKey,
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
        setCities([]);
        setStatus("");
        setFailure(false);
    };

    /** 打开添加或编辑弹窗；编辑模式只回填非敏感字段，敏感材料一律留空。 */
    const openForm = (source: WeatherSource, adding: boolean) => {
        const saved = adding ? undefined : sources.find((item) => item.source === source); // 已保存来源的列表状态。
        const filled: Record<WeatherField, string> = { ...EMPTY_DRAFT, ...saved?.identity };
        if (source === "qweather") filled.host = saved?.address ?? ""; // 和风的 Host 由列表状态回填。
        invalidate();
        opened.current = JSON.stringify(filled);
        live.current.dirty = false;
        setDraft(filled);
        setVisible(false);
        setChecks([]);
        setCity(undefined);
        setCities([]);
        setQuery("");
        setStatus("");
        setFailure(false);
        setPicker(false);
        setForm({ source, adding });
    };

    /** 关闭弹窗并释放后端草稿；有未保存修改时先确认。 */
    const closeForm = (discard = false) => {
        if (!discard && live.current.dirty) {
            setConfirm("discard");
            return;
        }
        invalidate();
        live.current.dirty = false;
        setForm(undefined);
        setConfirm(undefined);
        setPicker(false);
        setDraft(EMPTY_DRAFT);
        setChecks([]);
        setCity(undefined);
        setCities([]);
        setQuery("");
        setStatus(""); // 弹窗内的验证或保存反馈不残留到来源列表。
        setFailure(false);
    };

    /** 串行动作并拒绝取消或卸载后的提交。 */
    const handleAction = async (kind: string, action: (current: number) => Promise<void>) => {
        if (live.current.busy) return;
        live.current.busy = kind;
        setBusy(kind);
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
            if (live.current.mounted && current === revision.current) {
                live.current.busy = "";
                setBusy("");
            }
        }
    };

    /** 检测可用或切换启用来源；打开另一来源的开关先做一次存活探测。 */
    const handleProbe = async (source: WeatherSource, activate: boolean) => {
        if (live.current.busy) return;
        live.current.busy = activate ? "switch" : "probe";
        setBusy(live.current.busy);
        setMenu(undefined);
        setStatus("");
        setFailure(false);
        const current = ++revision.current; // 当前操作的前端身份。
        try {
            const snapshot = await weatherSettings({ type: "probe", source });
            if (!live.current.mounted || current !== revision.current) return;
            setSources(snapshot.sources);
            if (activate) onPreferences({ weatherSource: source }); // 探测通过才提交新的启用来源。
            setStatus(activate ? `已切换到 ${WEATHER_SOURCES[source].name}，正在获取天气数据` : `${WEATHER_SOURCES[source].name} ${AVAILABILITY.available}`);
        } catch (error) {
            if (live.current.mounted && current === revision.current) {
                setFailure(true);
                const reason = typeof error === "string" ? error : "探测失败"; // 探测只确认当前天气一次。
                setStatus(activate ? `切换失败：${reason}${enabled ? `，仍使用 ${WEATHER_SOURCES[enabled.source].name}` : ""}` : `检测失败：${reason}`);
            }
        } finally {
            if (live.current.mounted && current === revision.current) {
                live.current.busy = "";
                setBusy("");
            }
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

    /** 搜索只在提交时执行，结果仅用于当前验证。 */
    const handleSearch = () =>
        void handleAction("search", async (current) => {
            setCities([]);
            setCity(undefined);
            setChecks([]);
            const id = await ensureSession(current); // 后端已校验来源字段格式。
            const result = await weatherSettings({ type: "search", sessionId: id, query }); // 不缓存位置检索结果。
            if (current !== revision.current || !live.current.mounted) return;
            setCities(result.cities);
            if (!result.cities.length) setStatus("没有找到城市");
        });

    /** 按后端实际能力结果启用保存按钮。 */
    const handleValidate = () =>
        void handleAction("validate", async (current) => {
            if (!city || session.current === null) return;
            setChecks([]);
            const result = await weatherSettings({ type: "validate", sessionId: session.current, cityId: city.id }); // 不传入通过标记。
            if (current !== revision.current || !live.current.mounted) return;
            setChecks(result.checks);
            const passed = result.checks.length > 0 && result.checks.every((check) => check.failure === null); // 全部通过才可保存。
            setFailure(!passed);
            setStatus(passed ? "全部能力验证通过" : "部分能力未通过，原配置未改变");
        });

    /** 取消后立即清除本地门槛；后台有界请求仍由后端互斥保护。 */
    const handleCancelValidation = () => {
        invalidate();
        live.current.busy = "";
        setBusy("");
        setCity(undefined);
        setCities([]);
        setChecks([]);
        setStatus("验证已取消，请重新选择验证城市");
    };

    /** 保存成功后该来源成为当前使用；失败保留旧配置与本次草稿。 */
    const handleSave = () =>
        void handleAction("save", async (current) => {
            if (session.current === null || !form) return;
            const source = form.source; // 本次保存的来源。
            const snapshot = await weatherSettings({ type: "save", sessionId: session.current }); // 服务端再次检查验证状态。
            if (current !== revision.current || !live.current.mounted) return;
            setSources(snapshot.sources);
            closeForm(true);
            live.current.busy = "";
            setBusy("");
            onPreferences({ weatherSource: source }); // 保存并启用当前来源。
            setStatus(`已保存并启用 ${WEATHER_SOURCES[source].name}，正在获取天气数据`);
            const action = pendingLeave.current; // 未保存确认选择保存时继续原导航。
            pendingLeave.current = () => undefined;
            action();
        });

    /** 删除来源凭据并刷新列表；失败保留旧配置与确认框。 */
    const handleRemove = () =>
        void handleAction("remove", async (current) => {
            if (!pending) return;
            const source = pending; // 本次删除的来源。
            const snapshot = await weatherSettings({ type: "remove", source });
            if (current !== revision.current || !live.current.mounted) return;
            setSources(snapshot.sources);
            setConfirm(undefined);
            setPending(undefined);
            setStatus(`已删除 ${WEATHER_SOURCES[source].name}`);
        });

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
            setConfirm("discard");
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
            [pickerDialog.current, picker],
            [confirmDialog.current, !!confirm],
        ] as const) {
            if (!element) continue;
            if (open && !element.open) element.showModal(); // 城市弹窗在编辑弹窗之上独立打开。
            if (!open && element.open) element.close();
        }
    }, [form, picker, confirm]);

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
                    <BackIcon />
                </button>
                <h2>天气</h2>
                <small>{enabled ? `当前使用 ${WEATHER_SOURCES[enabled.source].name}` : "未启用来源"}</small>
            </div>
            <h3>已配置的天气来源</h3>
            {sources.length ? (
                <ul className="weather-settings__list">
                    {sources.map((item) => {
                        const current = item.source === activeSource; // 当前启用来源由已保存偏好决定。
                        return (
                            <li
                                key={item.source}
                                className="weather-settings__item"
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
                                <span onClick={(event) => event.stopPropagation()}>
                                    <SettingsSwitch
                                        label={`启用 ${WEATHER_SOURCES[item.source].name}`}
                                        disabled={!!busy || current}
                                        checked={current}
                                        onChange={(checked) => {
                                            if (checked) void handleProbe(item.source, true);
                                        }}
                                    />
                                </span>
                                <span className="weather-settings__arrow" aria-hidden="true">
                                    <ChevronRightIcon />
                                </span>
                            </li>
                        );
                    })}
                </ul>
            ) : (
                <p className="weather-settings__empty">尚未添加任何天气来源</p>
            )}
            <button type="button" className="weather-settings__add" disabled={!!busy} onClick={() => openForm("qweather", true)}>
                + 添加 API
            </button>
            {!form && (
                <p className={failure ? "weather-settings__error" : "weather-settings__status"} role="status">
                    {busy === "switch" ? "正在确认来源是否可用…" : status}
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
                        <button type="button" role="menuitem" disabled={!!busy} onClick={() => void handleProbe(menu.source, false)}>
                            检测可用
                        </button>
                    </li>
                    <li className="weather-settings__menu-separator" role="separator" />
                    <li role="none">
                        <button
                            type="button"
                            role="menuitem"
                            className="weather-settings__menu-danger"
                            onClick={() => {
                                setPending(menu.source);
                                setMenu(undefined);
                                setConfirm("remove");
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
                            <CloseIcon />
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
                                                setCities([]);
                                                setQuery("");
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
                            key === "privateKey" ? (
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
                                                {visible ? <ConcealIcon /> : <RevealIcon />}
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
                                                    if (selected.size > FIELDS.privateKey.maxLength) throw "私钥文件超过大小限制";
                                                    const text = await selected.text(); // 不保存导入路径。
                                                    if (current !== revision.current || !live.current.mounted) return;
                                                    handleDraft("privateKey", text);
                                                    live.current.busy = "";
                                                    setBusy("");
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
                                                {visible ? <ConcealIcon /> : <RevealIcon />}
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
                        <div className="weather-settings__row">
                            <span>验证城市</span>
                            <div className="weather-settings__city">
                                <span>{city?.name ?? "尚未选择"}</span>
                                {city && <small>{[city.adm2, city.adm1, city.country].filter(Boolean).join("，")}</small>}
                            </div>
                            <button
                                type="button"
                                disabled={!complete || !!busy}
                                onClick={() => {
                                    setStatus("");
                                    setPicker(true);
                                }}
                            >
                                选择城市
                            </button>
                        </div>
                        <h3>验证结果</h3>
                        <ul className="weather-settings__checks">
                            {checks.map((check) => (
                                <li key={check.label}>
                                    <span className={`weather-settings__result-icon${check.failure === null ? " is-passed" : ""}`} aria-hidden="true">
                                        {check.failure === null ? <PassedIcon /> : null}
                                    </span>
                                    <span>{check.label}</span>
                                    <span className={check.failure ? "weather-settings__error" : check.failure === null ? "weather-settings__passed" : ""}>
                                        {check.failure?.message ?? "通过"}
                                    </span>
                                </li>
                            ))}
                        </ul>
                    </div>
                    <footer className="weather-settings__actions weather-settings__dialog-actions">
                        <span className={`weather-settings__dialog-status${failure ? " weather-settings__error" : ""}`} role="status">
                            {busy === "validate" ? "正在验证天气能力…" : status}
                        </span>
                        {busy === "validate" ? (
                            <button type="button" onClick={handleCancelValidation}>
                                取消验证
                            </button>
                        ) : (
                            <button type="button" disabled={!city || !!busy} onClick={handleValidate}>
                                {checks.length ? "重新验证" : "验证配置"}
                            </button>
                        )}
                        <button type="button" className="weather-settings__primary" disabled={!verified || !!busy} onClick={handleSave}>
                            {busy === "save" ? "正在保存" : "保存并启用"}
                        </button>
                        <button type="button" disabled={busy === "save"} onClick={() => closeForm()}>
                            取消
                        </button>
                    </footer>
                </dialog>
            )}
            {picker && (
                <dialog
                    ref={pickerDialog}
                    className="weather-settings__dialog weather-settings__dialog--city"
                    aria-label="选择验证城市"
                    onCancel={(event) => {
                        event.preventDefault();
                        if (busy === "search") handleCancelValidation();
                        setPicker(false);
                    }}
                >
                    <header className="weather-settings__dialog-head">
                        <h2>选择验证城市</h2>
                        <button
                            type="button"
                            className="weather-settings__close"
                            aria-label="关闭"
                            title="关闭"
                            onClick={() => {
                                if (busy === "search") handleCancelValidation();
                                setPicker(false);
                            }}
                        >
                            <CloseIcon />
                        </button>
                    </header>
                    <div className="weather-settings__dialog-body">
                        <form
                            className="weather-settings__search"
                            onSubmit={(event) => {
                                event.preventDefault();
                                handleSearch();
                            }}
                        >
                            <input
                                autoFocus
                                aria-label="搜索城市"
                                value={query}
                                maxLength={100}
                                disabled={busy === "search"}
                                onChange={(event) => {
                                    setQuery(event.currentTarget.value);
                                    setCities([]);
                                }}
                            />
                            <button type="submit" className="weather-settings__icon" title="搜索城市" aria-label="搜索城市" disabled={!query.trim() || !!busy}>
                                <SearchIcon />
                            </button>
                        </form>
                        <div className="weather-settings__cities">
                            {cities.map((result) => (
                                <button
                                    type="button"
                                    key={result.id}
                                    disabled={!!busy}
                                    onClick={() => {
                                        setCity(result);
                                        setChecks([]);
                                        setStatus("");
                                        setPicker(false);
                                    }}
                                >
                                    <span>{result.name}</span>
                                    <small>{[result.adm2, result.adm1, result.country].filter(Boolean).join("，")}</small>
                                </button>
                            ))}
                        </div>
                        <p role="status">{busy === "search" ? "正在搜索…" : status}</p>
                    </div>
                </dialog>
            )}
            {confirm && (
                <dialog
                    ref={confirmDialog}
                    className="weather-settings__dialog weather-settings__dialog--confirm"
                    aria-label={confirm === "remove" ? "删除天气来源？" : "配置尚未保存"}
                    onCancel={(event) => {
                        event.preventDefault();
                        if (busy === "remove") return;
                        setConfirm(undefined);
                        setPending(undefined);
                    }}
                >
                    <header className="weather-settings__dialog-head">
                        <h2>{confirm === "remove" ? "删除天气来源？" : "配置尚未保存"}</h2>
                    </header>
                    <div className="weather-settings__dialog-body">
                        <p>
                            {confirm === "remove"
                                ? `删除 ${pending ? WEATHER_SOURCES[pending].name : ""} 在本机保存的凭据，并停止该来源的联网更新。`
                                : "关闭弹窗将丢失本次修改。可以取消关闭，或直接保存。"}
                        </p>
                    </div>
                    <footer className="weather-settings__actions weather-settings__dialog-actions">
                        {confirm === "remove" ? (
                            <>
                                <button
                                    type="button"
                                    autoFocus
                                    disabled={busy === "remove"}
                                    onClick={() => {
                                        setConfirm(undefined);
                                        setPending(undefined);
                                    }}
                                >
                                    取消
                                </button>
                                <button type="button" className="weather-settings__danger" disabled={busy === "remove"} onClick={() => void handleRemove()}>
                                    {busy === "remove" ? "正在删除" : "删除"}
                                </button>
                            </>
                        ) : (
                            <>
                                <button
                                    type="button"
                                    autoFocus
                                    disabled={busy === "save"}
                                    onClick={() => {
                                        pendingLeave.current = () => undefined;
                                        setConfirm(undefined);
                                    }}
                                >
                                    取消关闭
                                </button>
                                <button type="button" className="weather-settings__primary" disabled={!verified || !!busy} onClick={handleSave}>
                                    保存
                                </button>
                                <button
                                    type="button"
                                    disabled={busy === "save"}
                                    onClick={() => {
                                        const action = pendingLeave.current; // 先保留导航再丢弃草稿。
                                        closeForm(true);
                                        setConfirm(undefined);
                                        pendingLeave.current = () => undefined;
                                        action();
                                    }}
                                >
                                    不保存
                                </button>
                            </>
                        )}
                    </footer>
                </dialog>
            )}
        </section>
    );
};
