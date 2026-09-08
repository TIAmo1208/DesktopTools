import { exitDesktopTools } from "../../bridge/desktopHost";
import type { DesktopHostMonitor, DesktopHostSnapshot } from "../../contracts/desktopHost";
import { usePreferences } from "../settings/PreferencesProvider";
import "./DebugTest.css";

/** 只由已注册的 Debug 格子呈现宿主状态及测试命令。 */
export const DebugTest = ({
    monitor,
    host,
    itemCount,
    fenceCount,
    revision,
}: {
    monitor?: DesktopHostMonitor; // 当前可见显示器。
    host?: DesktopHostSnapshot; // 原生宿主事实。
    itemCount: number; // 真实桌面项目数。
    fenceCount: number; // 当前围栏数。
    revision?: number; // 当前布局修订。
}) => {
    const { preferences } = usePreferences(); // Debug 二级设置。
    const [ready, total] = [
        host?.hosts.filter(({ readiness }) => readiness.guardian && readiness.page && readiness.systemImages && readiness.hitMap).length ?? 0,
        host?.hosts.length ?? 0,
    ]; // 实际就绪门槛统计。
    return (
        <div className="debug-test">
            <dl>
                <div>
                    <dt>阶段</dt>
                    <dd>{host?.phase ?? "starting"}</dd>
                </div>
                {preferences?.debugShowDetails !== false && (
                    <>
                        <div>
                            <dt>显示器</dt>
                            <dd>
                                {monitor?.name ?? "正在发现显示器"} · DPI {monitor?.dpi ?? "--"}
                            </dd>
                        </div>
                        <div>
                            <dt>宿主</dt>
                            <dd>Windowed WebView2 · {total} 屏</dd>
                        </div>
                        <div>
                            <dt>就绪</dt>
                            <dd>
                                {ready} / {total}
                            </dd>
                        </div>
                        <div>
                            <dt>项目</dt>
                            <dd>{itemCount} 个</dd>
                        </div>
                        <div>
                            <dt>围栏</dt>
                            <dd>
                                {fenceCount} 个 · 布局修订 {revision ?? "--"}
                            </dd>
                        </div>
                    </>
                )}
            </dl>
            <div className="debug-test__actions">
                <button type="button" title="重载渲染器" aria-label="重载渲染器" onClick={() => window.location.reload()}>
                    {"\uE72C"}
                </button>
                <button type="button" onClick={() => void exitDesktopTools()}>
                    退出应用
                </button>
            </div>
        </div>
    );
};
