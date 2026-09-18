import { useRef } from "react";
import type { FolderPortalInstance, WorkspaceCommand, WorkspaceSnapshot } from "../../contracts/workspace";
import type { DesktopHostMonitor } from "../../contracts/desktopHost";
import type { TileAppearanceChange } from "../../contracts/tile";
import { DesktopTileBase } from "../tiles/DesktopTileBase";
import { DesktopTileHeader } from "../tiles/DesktopTileHeader";
import { DesktopTileViewport } from "../tiles/DesktopTileViewport";
import { useDesktopTileGeometry } from "../tiles/useDesktopTileGeometry";
import { useDesktopTileExpansion } from "../tiles/useDesktopTileExpansion";
import { FolderPortal, type FolderPortalActions } from "./FolderPortal";
import { usePreferences } from "../settings/PreferencesProvider";
import "./FolderPortalSurface.css";

/** 文件夹门户复用公共格子几何交互；目录内容仍由门户自身负责。 */
export const FolderPortalSurface = ({
    portal,
    onCommand,
    currentMonitor,
    monitors,
    onBeginInputDrag,
    onEndInputDrag,
    onFailure,
}: {
    portal: FolderPortalInstance;
    onCommand: (command: WorkspaceCommand, monitorId?: string) => Promise<WorkspaceSnapshot>;
    currentMonitor?: DesktopHostMonitor;
    monitors: DesktopHostMonitor[];
    onBeginInputDrag: () => Promise<void>;
    onEndInputDrag: () => Promise<void>;
    onFailure: (error: unknown) => void;
}) => {
    const surfaceRef = useRef<HTMLElement>(null); // 公共格子表面。
    const contentRef = useRef<FolderPortalActions>(null); // 当前实例独有的内容操作。
    const { bounds, rootPath } = portal; // 权威工作区几何和绑定。
    const { preferences } = usePreferences(); // 复用功能格子的吸附偏好。
    const expansion = useDesktopTileExpansion(surfaceRef, portal.appearance.collapsed, portal.appearance.autoExpand, currentMonitor?.id); // 折叠保留内容会话。
    const handleAppearance = (change: TileAppearanceChange) => {
        void onCommand({ type: "set-folder-portal-appearance", portalId: portal.id, change }).catch(onFailure);
    }; // 外观独立提交。
    const geometry = useDesktopTileGeometry({
        surfaceRef,
        bounds,
        collapsed: expansion.collapsed,
        title: rootPath,
        minimum: { width: 164, height: 154 },
        snap: preferences?.componentSnap !== false,
        currentMonitor,
        monitors,
        onFocus: () => surfaceRef.current?.focus({ preventScroll: true }),
        onCommit: (bounds, monitorId) => onCommand({ type: "update-folder-portal-bounds", portalId: portal.id, bounds }, monitorId),
        onBeginInputDrag,
        onEndInputDrag,
        onFailure,
    });
    return (
        <DesktopTileBase
            ref={surfaceRef}
            className="folder-portal-host"
            appearance={portal.appearance}
            collapsed={expansion.collapsed}
            onAppearanceChange={handleAppearance}
            onPointerEnter={expansion.onPointerEnter}
            onPointerLeave={expansion.onPointerLeave}
            menuItems={[
                { label: "刷新目录", separatorBefore: true, onSelect: () => void contentRef.current?.refresh() },
                { label: "返回根目录", onSelect: () => contentRef.current?.reset() },
                { label: "切换列表/平铺", separatorBefore: true, onSelect: () => contentRef.current?.toggleLayout() },
                { label: "复制选择", separatorBefore: true, onSelect: () => void contentRef.current?.clipboard(false) },
                { label: "剪切选择", onSelect: () => void contentRef.current?.clipboard(true) },
                { label: "粘贴到当前目录", onSelect: () => void contentRef.current?.paste() },
                {
                    label: "移除门户",
                    separatorBefore: true,
                    onSelect: () => void onCommand({ type: "remove-folder-portal", portalId: portal.id }).catch(onFailure),
                },
            ]}
            onResize={(event, direction) => void geometry.handlePointerDown(event, direction)}
            style={{
                left: bounds.left,
                top: bounds.top,
                right: "auto",
                width: bounds.width,
                height: expansion.collapsed ? 28 : bounds.height,
                zIndex: 1000 + portal.zIndex,
            }}
            data-hit-id={portal.id}
            data-portal-id={portal.id}
            data-folder-portal-path={portal.rootPath}
            data-hit-kind="component"
            data-hit-priority="60"
            tabIndex={-1}
            onPointerDown={(event) => event.stopPropagation()}
            onKeyDown={(event) => {
                event.stopPropagation();
                geometry.handleKeyDown(event);
            }}
        >
            <DesktopTileHeader
                title={rootPath.split(/[\\/]/).filter(Boolean).pop() ?? rootPath}
                editing={false}
                onRenameEnd={() => undefined}
                onRename={() => Promise.resolve()}
                onPointerDown={(event) => void geometry.handlePointerDown(event)}
            />
            <DesktopTileViewport collapsed={expansion.collapsed} scrollable={false} className="folder-portal-host__content">
                <FolderPortal
                    key={rootPath}
                    ref={contentRef}
                    rootPath={rootPath}
                    monitorId={currentMonitor?.id ?? portal.monitorId}
                    onBeginInputDrag={onBeginInputDrag}
                    onEndInputDrag={onEndInputDrag}
                    onCurrentPathChange={(path) => {
                        if (surfaceRef.current) surfaceRef.current.dataset.folderPortalPath = path;
                    }}
                />
            </DesktopTileViewport>
        </DesktopTileBase>
    );
};
