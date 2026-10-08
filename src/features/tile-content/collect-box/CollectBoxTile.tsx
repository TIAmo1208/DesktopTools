import { useCallback, useState } from "react";
import { ComponentTile, type ComponentTileProps } from "../../components/ComponentTile";
import { CollectBox, type CollectBoxToolbarActions } from "./CollectBox";
import { publishMessage } from "../../notifications/messageNotification";
import fenceSortIcon from "../../../../resources/fence-sort.svg?raw";
import collectNewIcon from "../../../../resources/collect-new.svg?raw";

/** 收集箱拥有最新条目与工具状态，公共标题只呈现计算后的文本。 */
export const CollectBoxTile = (props: ComponentTileProps) => {
    const [peek, setPeek] = useState({ title: "收集箱", summary: "" }); // 最新条目由内容计算。
    const [toolbar, setToolbar] = useState<CollectBoxToolbarActions>(); // 内容发布的实例工具。
    const handleToolbarReady = useCallback((actions: CollectBoxToolbarActions) => setToolbar(actions), []); // 保持注册回调稳定。
    return (
        <ComponentTile
            {...props}
            title="收集箱"
            expandOnDrop
            acceptsDrop={toolbar?.ready === true}
            scrollable={false}
            collapsedTitle={peek.title}
            subtitle={peek.summary}
            collapsedSize={{ height: 56 }}
            sortItems={toolbar?.sortItems}
            menuItems={[
                ...(toolbar?.layoutItems ?? []).map((item, index) => ({ ...item, separatorBefore: index === 0 })),
                { label: "恢复最近删除", separatorBefore: true, disabled: !toolbar?.ready, onSelect: () => toolbar?.restore() },
                {
                    label: "新建收集项",
                    separatorBefore: true,
                    disabled: !toolbar?.ready,
                    onSelect: () => toolbar?.create(),
                },
                {
                    label: "移除",
                    description: "仅移除当前格子，保留收集内容，可通过撤销恢复",
                    onSelect: () =>
                        void props.onCommand({ type: "remove-component", instanceId: props.instance.instanceId }).catch((error: unknown) =>
                            publishMessage({
                                type: "error",
                                content: error instanceof Error ? error.message : "移除收集箱格子失败",
                                source: "desktop-tile",
                            }),
                        ),
                },
            ]}
            commands={(sortMenuOpen) =>
                toolbar && (
                    <>
                        <button
                            type="button"
                            title="排序方式"
                            aria-label="排序方式"
                            aria-haspopup="menu"
                            aria-expanded={sortMenuOpen}
                            onClick={(event) => {
                                const rect = event.currentTarget.getBoundingClientRect(); // 共用菜单锚点及焦点恢复。
                                event.currentTarget.closest(".desktop-tile")?.dispatchEvent(
                                    new CustomEvent("desktop-tile-menu", {
                                        detail: { left: rect.left, top: rect.bottom, sorting: true, trigger: event.currentTarget },
                                    }),
                                );
                            }}
                        >
                            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: fenceSortIcon }} />
                        </button>
                        <button type="button" title="新建收集项" aria-label="新建收集项" disabled={!toolbar.ready} onClick={toolbar.create}>
                            <span aria-hidden="true" dangerouslySetInnerHTML={{ __html: collectNewIcon }} />
                        </button>
                    </>
                )
            }
        >
            {(collapsed) =>
                props.registration?.entry === "official:collect-box" ? (
                    <CollectBox
                        collapsed={collapsed}
                        instanceState={props.instance.state}
                        onStateChange={async (state) => {
                            await props.onCommand({
                                type: "update-component-state",
                                instanceId: props.instance.instanceId,
                                stateVersion: props.instance.stateVersion,
                                state,
                            });
                        }}
                        onLatestChange={setPeek}
                        onBeginInputDrag={props.onBeginInputDrag}
                        onEndInputDrag={props.onEndInputDrag}
                        onToolbarReady={handleToolbarReady}
                    />
                ) : (
                    <p className="official-component__error">组件前端入口不存在</p>
                )
            }
        </ComponentTile>
    );
};
