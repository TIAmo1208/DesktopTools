import { useEffect, useState } from "react";
import { listenComponentHostEvents } from "../../../bridge/components";
import type { ComponentInstanceSnapshot, ComponentRegistrationSnapshot } from "../../../contracts/component";
import { usePreferences } from "../../settings/PreferencesProvider";
import { ComponentTile, type ComponentTileProps } from "../../components/ComponentTile";
import "./LocalClock.css";

/** 呈现官方本地时间组件。 */
const LocalClock = ({
    instance,
    registration,
    visible,
    monitorId,
}: {
    instance: ComponentInstanceSnapshot;
    registration: ComponentRegistrationSnapshot;
    visible: boolean; // 实际展开状态包含临时悬停展开。
    monitorId: string; // 当前呈现宿主，不等同于离线显示器归属。
}) => {
    const [unixMillis, setUnixMillis] = useState<number>(); // 宿主授权能力或事件提供的当前时间。
    const [failure, setFailure] = useState<string>(); // 当前实例能力调用失败说明。
    const canReadTime = registration.authorizedCapabilities.includes("read-host-time"); // 当前清单实际获得的宿主时间能力。
    const { preferences } = usePreferences(); // 跨窗口同步的时间格式。
    const hour12 = preferences?.clockHour12 ?? false; // 用户设置的时间制式。
    const [paused, setPaused] = useState(() => window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.() ?? false); // 当前页面实际暂停状态。

    useEffect(() => {
        const handleRefreshState = () => setPaused(window.__DESKTOP_TOOLS_NATIVE__?.isRefreshPaused?.() ?? false); // 恢复后重新建立活动订阅。
        window.addEventListener("desktop-refresh-changed", handleRefreshState);
        return () => window.removeEventListener("desktop-refresh-changed", handleRefreshState);
    }, []);
    useEffect(() => {
        if (!visible || paused || !canReadTime) return;
        let active = true; // 折叠、暂停及重新挂载后拒绝旧结果。
        let stop: (() => void) | undefined; // 当前时间订阅的释放入口。
        const handleFailure = (error: unknown) => {
            if (active) setFailure(error instanceof Error ? error.message : "组件时间读取失败");
        }; // 错误只影响当前时钟。
        void listenComponentHostEvents(
            instance.instanceId,
            monitorId,
            (event) => {
                if (active && event.kind === "time-tick") {
                    setUnixMillis(event.unixMillis);
                    setFailure(undefined);
                }
            },
            handleFailure,
        )
            .then((unlisten) => {
                if (!active) {
                    unlisten();
                    return;
                }
                stop = unlisten;
            })
            .catch(handleFailure);
        return () => {
            active = false;
            stop?.();
        };
    }, [instance.instanceId, monitorId, visible, paused, canReadTime]);

    const text = unixMillis
        ? new Intl.DateTimeFormat("zh-CN", {
              hour: "2-digit",
              minute: "2-digit",
              second: preferences?.clockShowSeconds === false ? undefined : "2-digit",
              hour12,
          }).format(new Date(unixMillis))
        : "--:--:--"; // 当前组件显示的宿主时间文本。

    return (
        <>
            <time className="local-clock" dateTime={unixMillis ? new Date(unixMillis).toISOString() : undefined}>
                {text}
            </time>
            {failure && <p className="local-clock__error">{failure}</p>}
        </>
    );
};

/** 时间内容拥有订阅，通用宿主只传入实例和能力。 */
export const ClockTile = (props: ComponentTileProps) => (
    <ComponentTile {...props} title="本地时间" minimum={{ width: 164, height: 90 }}>
        {(collapsed) =>
            props.registration?.entry === "official:local-clock" ? (
                <LocalClock
                    instance={props.instance}
                    registration={props.registration}
                    visible={!collapsed}
                    monitorId={props.currentMonitor?.id ?? props.instance.monitorId}
                />
            ) : (
                <p className="local-clock__error">组件前端入口不存在</p>
            )
        }
    </ComponentTile>
);
