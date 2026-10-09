import { useEffect, useRef, useState } from "react";
import type { ApplicationPreferences } from "../../contracts/settings";
import type { FolderPortalInstance, WorkspaceCommand, WorkspaceSnapshot } from "../../contracts/workspace";
import { SettingsSwitch } from "./SettingsSwitch";
import "../tiles/DesktopTileMenu.css";
import "./FeatureSettings.css";

/** 功能格子开关与二级配置，隐藏不销毁工作区实例。 */
export const FeatureSettings = ({
    preferences,
    disabled,
    debugBuild,
    onPreferences,
    workspace,
    onWorkspaceCommand,
    onChooseFolder,
    onCreateCollectBox,
}: {
    preferences?: ApplicationPreferences; // 已保存的格子偏好。
    disabled: boolean; // 提交中的互斥状态。
    debugBuild: boolean; // Rust 构建提供的调试边界。
    onPreferences: (patch: Partial<ApplicationPreferences>) => void; // 提交用户意图。
    workspace?: WorkspaceSnapshot; // 当前工作区映射快照。
    onWorkspaceCommand: (command: WorkspaceCommand) => Promise<void>; // 提交映射事务。
    onChooseFolder: () => Promise<string | null>; // 打开系统目录选择器。
    onCreateCollectBox: () => Promise<void>; // 调用统一收集箱创建入口。
}) => {
    const [detail, setDetail] = useState<"clock" | "music" | "collect" | "folder" | "debug">(); // 当前二级菜单。
    const [selectedPortalId, setSelectedPortalId] = useState<string>(); // 当前单击选中的映射。
    const [portalMenu, setPortalMenu] = useState<{ portalId: string; left: number; top: number }>(); // 当前跟随鼠标的映射菜单。
    const portalMenuRef = useRef<HTMLMenuElement>(null); // 右键菜单外部点击边界。
    const unavailable = disabled || !preferences; // 加载前不允许修改。
    const [imageWidth, setImageWidth] = useState(String(preferences?.collectImagePreviewWidth ?? 78)); // 输入草稿提交前允许暂时为空。
    useEffect(() => setImageWidth(String(preferences?.collectImagePreviewWidth ?? 78)), [preferences?.collectImagePreviewWidth, disabled]);
    const portals = workspace?.folderPortals ?? []; // 当前已创建的文件夹映射。
    const portalTitle = (portal: FolderPortalInstance) => portal.rootPath.split(/[\\/]/).filter(Boolean).pop() ?? portal.rootPath; // 路径末级作为用户可见标题。
    const remap = async (portalId: string) => {
        const rootPath = await onChooseFolder();
        if (rootPath) await onWorkspaceCommand({ type: "remap-folder-portal", portalId, rootPath });
        setPortalMenu(undefined);
    };
    useEffect(() => {
        if (!portalMenu) return;
        const closeOutside = (event: PointerEvent) => {
            if (event.target instanceof Node && portalMenuRef.current?.contains(event.target)) return;
            setPortalMenu(undefined);
        };
        const closeEscape = (event: KeyboardEvent) => {
            if (event.key === "Escape") setPortalMenu(undefined);
        };
        const closeOnBlur = () => setPortalMenu(undefined);
        document.addEventListener("pointerdown", closeOutside, true);
        document.addEventListener("keydown", closeEscape);
        window.addEventListener("blur", closeOnBlur);
        return () => {
            document.removeEventListener("pointerdown", closeOutside, true);
            document.removeEventListener("keydown", closeEscape);
            window.removeEventListener("blur", closeOnBlur);
        };
    }, [portalMenu]);
    return (
        <section className="feature-settings">
            {detail && (
                <button className="feature-settings__back" type="button" title="返回功能格子" aria-label="返回功能格子" onClick={() => setDetail(undefined)}>
                    {"\uE72B"}
                </button>
            )}
            <h2>
                {detail === "clock"
                    ? "本地时间"
                    : detail === "music"
                      ? "音乐控制"
                      : detail === "collect"
                        ? "收集箱"
                        : detail === "folder"
                          ? "文件夹映射"
                          : detail === "debug"
                            ? "Debug 测试"
                            : "功能格子"}
            </h2>
            {!detail ? (
                <>
                    {(
                        [
                            { id: "clock", key: "clockEnabled", label: "本地时间" },
                            { id: "music", key: "musicEnabled", label: "音乐控制" },
                            { id: "collect", key: "collectEnabled", label: "收集箱" },
                            { id: "folder", key: "folderPortalEnabled", label: "文件夹映射" },
                            ...(debugBuild ? [{ id: "debug", key: "debugEnabled", label: "Debug 测试" }] : []),
                        ] as const
                    ).map((feature) => (
                        <div className="feature-settings__row feature-settings__row--link" key={feature.id}>
                            <button
                                className="feature-settings__open"
                                type="button"
                                title={`${feature.label}设置`}
                                aria-label={`${feature.label}设置`}
                                onClick={() => setDetail(feature.id as "clock" | "music" | "collect" | "folder" | "debug")}
                            >
                                {feature.label}
                            </button>
                            <SettingsSwitch
                                label={feature.label}
                                disabled={unavailable}
                                checked={Boolean(
                                    preferences?.[feature.key as "clockEnabled" | "musicEnabled" | "collectEnabled" | "folderPortalEnabled" | "debugEnabled"],
                                )}
                                onChange={(checked) => onPreferences({ [feature.key]: checked } as Partial<ApplicationPreferences>)}
                            />
                            <span className="feature-settings__chevron" aria-hidden="true">
                                {"\uE76C"}
                            </span>
                        </div>
                    ))}
                </>
            ) : detail === "clock" ? (
                <>
                    <label className="feature-settings__row">
                        时间格式
                        <select
                            disabled={unavailable}
                            value={preferences?.clockHour12 ? "12" : "24"}
                            onChange={(event) => onPreferences({ clockHour12: event.currentTarget.value === "12" })}
                        >
                            <option value="24">24 小时制</option>
                            <option value="12">12 小时制</option>
                        </select>
                    </label>
                    <div className="feature-settings__row">
                        <span>显示秒数</span>
                        <SettingsSwitch
                            label="显示秒数"
                            disabled={unavailable}
                            checked={preferences?.clockShowSeconds ?? true}
                            onChange={(checked) => onPreferences({ clockShowSeconds: checked })}
                        />
                    </div>
                </>
            ) : detail === "music" ? (
                <>
                    <div className="feature-settings__row">
                        <span>显示媒体来源</span>
                        <SettingsSwitch
                            label="显示媒体来源"
                            disabled={unavailable}
                            checked={preferences?.musicShowSource ?? true}
                            onChange={(checked) => onPreferences({ musicShowSource: checked })}
                        />
                    </div>
                </>
            ) : detail === "collect" ? (
                <>
                    <p className="feature-settings__hint">收集项保存在配置目录的 collect 文件夹中，多个收集箱格子共享同一批条目。</p>
                    <label className="feature-settings__row">
                        <span>图片文件收入方式</span>
                        <select
                            disabled={unavailable}
                            value={preferences?.collectImageImportMode ?? "copy"}
                            onChange={(event) => onPreferences({ collectImageImportMode: event.currentTarget.value as "copy" | "move" | "link" })}
                        >
                            <option value="copy">复制到资源目录</option>
                            <option value="move">移动到资源目录</option>
                            <option value="link">只保存原路径链接</option>
                        </select>
                    </label>
                    <div className="feature-settings__row">
                        <span>编辑窗退出时自动保存</span>
                        <SettingsSwitch
                            label="编辑窗退出时自动保存"
                            disabled={unavailable}
                            checked={preferences?.collectAutoSaveOnExit ?? false}
                            onChange={(checked) => onPreferences({ collectAutoSaveOnExit: checked })}
                        />
                    </div>
                    <label className="feature-settings__row">
                        <span>图片预览宽度（DIP）</span>
                        <input
                            type="number"
                            min={48}
                            max={156}
                            step={1}
                            disabled={unavailable}
                            value={imageWidth}
                            onChange={(event) => setImageWidth(event.currentTarget.value)}
                            onKeyDown={(event) => {
                                if (event.key === "Enter") event.currentTarget.blur();
                            }}
                            onBlur={() => {
                                const value = Number(imageWidth); // 失焦或 Enter 提交整数，非法输入恢复已保存值。
                                if (Number.isInteger(value) && value >= 48 && value <= 156) {
                                    if (value !== preferences?.collectImagePreviewWidth) onPreferences({ collectImagePreviewWidth: value });
                                } else setImageWidth(String(preferences?.collectImagePreviewWidth ?? 78));
                            }}
                        />
                    </label>
                    <div className="feature-settings__row feature-settings__folder-action">
                        <span className="feature-settings__row-title">
                            添加收集箱
                            <small>在桌面创建新的收集箱格子。</small>
                        </span>
                        <button className="feature-settings__action" type="button" disabled={unavailable} onClick={() => void onCreateCollectBox()}>
                            添加
                        </button>
                    </div>
                </>
            ) : detail === "folder" ? (
                <>
                    <div className="feature-settings__row feature-settings__folder-action">
                        <span className="feature-settings__row-title">
                            新建文件夹映射
                            <small>选择目录并在桌面创建新的独立映射格子。</small>
                        </span>
                        <button
                            className="feature-settings__action"
                            type="button"
                            disabled={unavailable}
                            onClick={() =>
                                void onChooseFolder().then(async (rootPath) => {
                                    if (rootPath)
                                        await onWorkspaceCommand({
                                            type: "create-folder-portal",
                                            rootPath,
                                            bounds: { left: 24, top: 24, width: 360, height: 280 },
                                        });
                                })
                            }
                        >
                            新建
                        </button>
                    </div>
                    {portals.length > 0 && <div className="feature-settings__portal-separator" aria-hidden="true" />}
                    {portals.length > 0 && (
                        <div className="feature-settings__portal-table">
                            <div className="feature-settings__portal-header">
                                <span>文件夹映射</span>
                                <span>绑定路径</span>
                            </div>
                            {portals.map((portal) => (
                                <div
                                    className={`feature-settings__portal-row${selectedPortalId === portal.id ? " is-selected" : ""}`}
                                    key={portal.id}
                                    onClick={() => {
                                        setSelectedPortalId(portal.id);
                                        setPortalMenu(undefined);
                                    }}
                                    onContextMenu={(event) => {
                                        event.preventDefault();
                                        setSelectedPortalId(portal.id);
                                        setPortalMenu({
                                            portalId: portal.id,
                                            left: Math.min(event.clientX, window.innerWidth - 210),
                                            top: Math.min(event.clientY, window.innerHeight - 100),
                                        });
                                    }}
                                >
                                    <span>{portalTitle(portal)}</span>
                                    <span title={portal.rootPath}>{portal.rootPath}</span>
                                </div>
                            ))}
                        </div>
                    )}
                    {portalMenu && (
                        <menu
                            ref={portalMenuRef}
                            className="desktop-tile-menu feature-settings__portal-menu"
                            role="menu"
                            style={{ left: portalMenu.left, top: portalMenu.top }}
                            onPointerDown={(event) => event.stopPropagation()}
                        >
                            <li role="none">
                                <button type="button" role="menuitem" onClick={() => void remap(portalMenu.portalId)}>
                                    重新映射路径
                                </button>
                            </li>
                            <li role="none">
                                <button
                                    type="button"
                                    role="menuitem"
                                    onClick={() => {
                                        const portalId = portalMenu.portalId;
                                        setPortalMenu(undefined);
                                        void onWorkspaceCommand({ type: "remove-folder-portal", portalId });
                                    }}
                                >
                                    移除映射
                                </button>
                            </li>
                        </menu>
                    )}
                </>
            ) : (
                debugBuild && (
                    <div className="feature-settings__row">
                        <span>显示详细宿主状态</span>
                        <SettingsSwitch
                            label="显示详细宿主状态"
                            disabled={unavailable}
                            checked={preferences?.debugShowDetails ?? true}
                            onChange={(checked) => onPreferences({ debugShowDetails: checked })}
                        />
                    </div>
                )
            )}
        </section>
    );
};
