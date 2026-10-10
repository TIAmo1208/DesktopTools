import { useEffect, useRef, useState, type RefObject } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { weatherSettings } from "../../bridge/weather";
import { SettingsInfo } from "./SettingsInfo";
import type { WeatherCity, WeatherCredentials, WeatherSettingsSnapshot } from "../../contracts/weather";
import "./WeatherSettings.css";

/** 天气设置独立草稿、临时城市和验证结果，不使用普通偏好保存。 */
export const WeatherSettings = ({ onBack, leaveGuard }: { onBack: () => void; leaveGuard: RefObject<((action: () => void) => void) | undefined> }) => {
    const [draft, setDraft] = useState<WeatherCredentials>({ host: "", developerId: "", projectId: "", credentialId: "", privateKey: "" }); // 只在当前页内存保存。
    const [configured, setConfigured] = useState(false); // 已保存记录状态，不代表本次验证通过。
    const [editing, setEditing] = useState(true); // 保存后仅显示已配置，不回填敏感字段。
    const [visible, setVisible] = useState(false); // 仅当前私钥草稿显隐。
    const [busy, setBusy] = useState(""); // 当前动作名称。
    const [message, setMessage] = useState(""); // 脱敏的状态反馈。
    const [failure, setFailure] = useState(false); // 错误色只辅助文本。
    const [checks, setChecks] = useState<WeatherSettingsSnapshot["checks"]>([]); // 五项真实检查结果。
    const [city, setCity] = useState<WeatherCity>(); // 当前临时验证城市。
    const [cities, setCities] = useState<WeatherCity[]>([]); // 当前搜索结果。
    const [query, setQuery] = useState(""); // 手动提交搜索。
    const [modal, setModal] = useState<"city" | "leave" | "clear">(); // 复用原生 dialog 的焦点管理。
    const dialog = useRef<HTMLDialogElement>(null); // 当前模态元素。
    const file = useRef<HTMLInputElement>(null); // 本地私钥导入入口。
    const session = useRef<number | null>(null); // 后端会话身份。
    const revision = useRef(0); // 取消和字段改变时拒绝旧结果。
    const live = useRef({ dirty: false, busy: "", mounted: true }); // 长生命周期回调读取最新状态。
    const pendingLeave = useRef<() => void>(() => undefined); // 确认后执行的导航。
    const complete = Object.values(draft).every((value) => value.trim()); // 仅控制本地按钮，后端仍独立校验。
    const verified = checks.length === 5 && checks.every((check) => check === null); // 仅展示和禁用用途。

    /** 字段改变和离开时使会话失效，旧取消只作用于旧身份。 */
    const invalidate = () => {
        revision.current++;
        const previous = session.current; // 不允许取消迟到的新会话。
        session.current = null;
        if (previous !== null) void weatherSettings({ type: "cancel", sessionId: previous }).catch(() => undefined);
        setChecks([]);
    };

    /** 清空敏感草稿，只在成功保存或明确放弃时调用。 */
    const reset = () => {
        invalidate();
        live.current.dirty = false;
        setDraft({ host: "", developerId: "", projectId: "", credentialId: "", privateKey: "" });
        setVisible(false);
        setCity(undefined);
        setCities([]);
        setQuery("");
    };

    useEffect(() => {
        live.current.mounted = true;
        const guard = (action: () => void) => {
            if (live.current.busy === "save" || live.current.busy === "clear") return;
            if (!live.current.dirty) {
                action();
                return;
            }
            pendingLeave.current = action;
            setModal("leave");
        }; // 所有离开入口读取同一份最新状态。
        leaveGuard.current = guard;
        let active = true; // 处理异步监听注册晚于卸载的情况。
        void weatherSettings({ type: "status" })
            .then((snapshot) => {
                if (!active) return;
                setConfigured(snapshot.configured);
                if (!live.current.dirty) setEditing(!snapshot.configured);
            })
            .catch(() => {
                if (active) {
                    setFailure(true);
                    setMessage("无法读取 Windows 凭据状态，请重新打开天气设置");
                }
            });
        const stop = getCurrentWindow().onCloseRequested((event) => {
            event.preventDefault();
            guard(() => {
                void getCurrentWindow()
                    .destroy()
                    .catch(() => {
                        setFailure(true);
                        setMessage("设置窗口关闭失败，请重试");
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
                    setMessage("关闭保护注册失败，请重新打开设置窗口");
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
        const element = dialog.current; // 关闭后浏览器恢复触发入口焦点。
        if (!modal || !element) return;
        element.showModal();
        return () => element.close();
    }, [modal]);

    /** 每次编辑都清除旧验证，不在输入时请求供应商。 */
    const handleDraft = (key: keyof WeatherCredentials, value: string) => {
        invalidate();
        live.current.dirty = true;
        setDraft((previous) => ({ ...previous, [key]: value }));
        setCity(undefined);
        setCities([]);
        setMessage("");
        setFailure(false);
    };

    /** 串行动作并拒绝取消或卸载后的提交。 */
    const handleAction = async (kind: string, action: (current: number) => Promise<void>) => {
        if (live.current.busy) return;
        live.current.busy = kind;
        setBusy(kind);
        setMessage("");
        setFailure(false);
        const current = ++revision.current; // 当前操作的前端身份。
        try {
            await action(current);
        } catch (error) {
            if (live.current.mounted && current === revision.current) {
                setFailure(true);
                setMessage(typeof error === "string" ? error : "天气设置操作失败，请重试");
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
        const result = await weatherSettings({ type: "begin", credentials: draft }); // 仅明确搜索时上传。
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
            const id = await ensureSession(current); // 后端已校验 Host 和密钥格式。
            const result = await weatherSettings({ type: "search", sessionId: id, query }); // 不缓存 GeoAPI 数据。
            if (current !== revision.current || !live.current.mounted) return;
            setCities(result.cities);
            if (!result.cities.length) setMessage("没有找到城市");
        });

    /** 按后端实际能力结果启用保存按钮。 */
    const handleValidate = () =>
        void handleAction("validate", async (current) => {
            if (!city || session.current === null) return;
            setChecks([]);
            const result = await weatherSettings({ type: "validate", sessionId: session.current, cityId: city.id }); // 不传入通过标记。
            if (current !== revision.current || !live.current.mounted) return;
            setChecks(result.checks);
            const passed = result.checks.length === 5 && result.checks.every((check) => check === null); // 全部通过才可保存。
            setFailure(!passed);
            setMessage(passed ? "全部能力验证通过" : "部分能力未通过，原配置未改变");
        });

    /** 取消后立即清除本地门槛；后台有界请求仍由后端互斥保护。 */
    const handleCancelValidation = () => {
        invalidate();
        live.current.busy = "";
        setBusy("");
        setCity(undefined);
        setCities([]);
        setMessage("验证已取消，请重新选择验证城市");
    };

    /** 保存失败保留草稿，成功后清除私钥与临时城市。 */
    const handleSave = () =>
        void handleAction("save", async (current) => {
            if (session.current === null) return;
            const result = await weatherSettings({ type: "save", sessionId: session.current }); // 服务端再次检查验证状态。
            if (current !== revision.current || !live.current.mounted) return;
            reset();
            live.current.busy = "";
            setBusy("");
            setConfigured(result.configured);
            setEditing(false);
            setMessage("凭据已安全保存");
        });

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
                    {"\uE72B"}
                </button>
                <h2>天气</h2>
                <small>{live.current.dirty ? "草稿 · 尚未保存" : configured ? "已配置" : "凭据未配置"}</small>
            </div>
            <p className="weather-settings__source">和风天气 / JWT</p>
            {editing ? (
                <>
                    {(
                        [
                            [
                                "host",
                                "API Host",
                                "从和风天气控制台完整复制 API Host，保留所有域名标签，例如 example.other.qweatherapi.com。仅填写主机名，不包含 https://、端口、路径、查询或片段。",
                            ],
                            [
                                "developerId",
                                "开发者 ID",
                                "在和风天气控制台的账户设置中查看开发者 ID（Developer ID），以 Q 开头，共 10 位；不要填写邮箱或项目 ID。",
                            ],
                            [
                                "projectId",
                                "项目 ID",
                                "在和风天气控制台的项目管理中创建或选择项目，复制项目 ID（Project ID）。该项目需具有城市搜索与天气预报接口权限。",
                            ],
                            [
                                "credentialId",
                                "凭据 ID",
                                "在所选项目下创建 JWT 凭据并上传 Ed25519 公钥，复制对应的凭据 ID（Credential ID）。此处不是 API KEY 或临时 JWT 令牌。",
                            ],
                        ] as const
                    ).map(([key, label, description]) => (
                        <div className="weather-settings__row" key={key}>
                            <span>
                                <label htmlFor={`weather-${key}`}>{label}</label>
                                <SettingsInfo label={label} description={description} />
                            </span>
                            <input
                                id={`weather-${key}`}
                                autoComplete="off"
                                spellCheck={false}
                                maxLength={key === "host" ? 253 : 64}
                                disabled={!!busy}
                                value={draft[key]}
                                onChange={(event) => handleDraft(key, event.currentTarget.value)}
                            />
                        </div>
                    ))}
                    <div className="weather-settings__row weather-settings__row--private">
                        <span>
                            <label htmlFor="weather-private-key">私钥</label>
                            <SettingsInfo
                                label="私钥"
                                description="按和风天气开发文档的 JWT 身份认证说明生成 Ed25519 密钥对：公钥上传到项目凭据，私钥保留在本机。导入对应的 PKCS#8 PEM 私钥文件，或粘贴包含 BEGIN PRIVATE KEY 和 END PRIVATE KEY 的完整内容；不要填写公钥或 JWT 令牌。"
                            />
                        </span>
                        <div className="weather-settings__private">
                            <div className="weather-settings__private-input">
                                <input
                                    id="weather-private-key"
                                    type={visible ? "text" : "password"}
                                    autoComplete="new-password"
                                    spellCheck={false}
                                    maxLength={1024}
                                    disabled={!!busy}
                                    value={draft.privateKey}
                                    onPaste={(event) => {
                                        event.preventDefault();
                                        handleDraft("privateKey", event.clipboardData.getData("text").slice(0, 1024));
                                    }}
                                    onChange={(event) => handleDraft("privateKey", event.currentTarget.value)}
                                />
                                <button
                                    type="button"
                                    className="weather-settings__icon"
                                    aria-label={visible ? "隐藏私钥草稿" : "显示私钥草稿"}
                                    title={visible ? "隐藏私钥草稿" : "显示私钥草稿"}
                                    aria-pressed={visible}
                                    onClick={() => setVisible(!visible)}
                                >
                                    {visible ? "\uED1A" : "\uE890"}
                                </button>
                            </div>
                            <div className="weather-settings__actions">
                                <button type="button" disabled={!!busy} onClick={() => file.current?.click()}>
                                    导入私钥
                                </button>
                                <small>{draft.privateKey ? "已填写草稿" : ""}</small>
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
                                        if (selected.size > 1024) throw "私钥文件超过大小限制";
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
                                setMessage("");
                                setModal("city");
                            }}
                        >
                            选择城市
                        </button>
                    </div>
                    <h3>能力验证</h3>
                    <ul className="weather-settings__checks">
                        {["城市搜索", "当前天气", "24 小时预报", "7 天预报", "来源与详情链接"].map((label, index) => (
                            <li key={label}>
                                <span className={`weather-settings__result-icon${checks[index] === null ? " is-passed" : ""}`} aria-hidden="true">
                                    {checks[index] === null ? "\uE73E" : ""}
                                </span>
                                <span>{label}</span>
                                <span
                                    className={
                                        checks[index]
                                            ? "weather-settings__error"
                                            : checks[index] === null || (busy !== "validate" && index === 0 && city)
                                              ? "weather-settings__passed"
                                              : ""
                                    }
                                >
                                    {checks[index]?.message ??
                                        (checks[index] === null ? "通过" : busy === "validate" ? "验证中" : index === 0 && city ? "通过" : "未验证")}
                                </span>
                            </li>
                        ))}
                    </ul>
                    <div className="weather-settings__actions">
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
                        <button
                            type="button"
                            disabled={busy === "save" || busy === "clear"}
                            onClick={() =>
                                leaveGuard.current?.(() => {
                                    reset();
                                    if (configured) setEditing(false);
                                    else onBack();
                                })
                            }
                        >
                            取消
                        </button>
                    </div>
                </>
            ) : (
                <div className="weather-settings__row">
                    <span>凭据已配置</span>
                    <div className="weather-settings__actions">
                        <button
                            type="button"
                            onClick={() => {
                                setEditing(true);
                                setMessage("");
                            }}
                        >
                            替换凭据
                        </button>
                        <button type="button" disabled={!!busy} onClick={() => setModal("clear")}>
                            清除凭据
                        </button>
                    </div>
                </div>
            )}
            <p className={failure ? "weather-settings__error" : "weather-settings__status"} role="status">
                {message || (busy === "validate" ? "正在验证天气能力…" : "")}
            </p>
            {modal && (
                <dialog
                    ref={dialog}
                    className={`weather-settings__dialog${modal === "city" ? " weather-settings__dialog--city" : ""}`}
                    aria-label={modal === "city" ? "选择验证城市" : modal === "clear" ? "清除天气凭据？" : "放弃未保存的编辑？"}
                    onCancel={(event) => {
                        event.preventDefault();
                        if (busy !== "clear") {
                            if (busy === "search") handleCancelValidation();
                            setModal(undefined);
                        }
                    }}
                >
                    <h2>{modal === "city" ? "选择验证城市" : modal === "clear" ? "清除天气凭据？" : "放弃未保存的编辑？"}</h2>
                    {modal === "city" ? (
                        <>
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
                                <button
                                    type="submit"
                                    className="weather-settings__icon"
                                    title="搜索城市"
                                    aria-label="搜索城市"
                                    disabled={!query.trim() || !!busy}
                                >
                                    {"\uE721"}
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
                                            setMessage("");
                                            setModal(undefined);
                                        }}
                                    >
                                        <span>{result.name}</span>
                                        <small>{[result.adm2, result.adm1, result.country].filter(Boolean).join("，")}</small>
                                    </button>
                                ))}
                            </div>
                            <p role="status">{busy === "search" ? "正在搜索…" : message}</p>
                            <div className="weather-settings__actions">
                                <button
                                    type="button"
                                    onClick={() => {
                                        if (busy === "search") handleCancelValidation();
                                        setModal(undefined);
                                    }}
                                >
                                    取消
                                </button>
                            </div>
                        </>
                    ) : (
                        <>
                            <p>{modal === "clear" ? "清除本机天气凭据并停止联网更新，保留格子布局及城市选择。" : "当前草稿尚未保存，离开后将丢失本次编辑。"}</p>
                            <div className="weather-settings__actions">
                                <button type="button" autoFocus disabled={busy === "clear"} onClick={() => setModal(undefined)}>
                                    {modal === "clear" ? "取消" : "继续编辑"}
                                </button>
                                <button
                                    type="button"
                                    disabled={busy === "save" || busy === "clear"}
                                    onClick={() => {
                                        if (modal === "leave") {
                                            reset();
                                            live.current.busy = "";
                                            setBusy("");
                                            setModal(undefined);
                                            pendingLeave.current();
                                            return;
                                        }
                                        void handleAction("clear", async (current) => {
                                            const result = await weatherSettings({ type: "clear" }); // 失败时保留旧配置和对话框。
                                            if (current !== revision.current || !live.current.mounted) return;
                                            reset();
                                            live.current.busy = "";
                                            setBusy("");
                                            setConfigured(result.configured);
                                            setEditing(true);
                                            setModal(undefined);
                                            setMessage("凭据已清除");
                                        });
                                    }}
                                >
                                    {modal === "clear" ? "清除凭据" : "放弃编辑"}
                                </button>
                            </div>
                            {failure && (
                                <p role="alert" className="weather-settings__error">
                                    {message}
                                </p>
                            )}
                        </>
                    )}
                </dialog>
            )}
        </section>
    );
};
