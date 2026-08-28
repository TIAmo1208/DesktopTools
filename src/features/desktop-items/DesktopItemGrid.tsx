import type { DesktopItemSnapshot } from "../../contracts/desktopItem";
import { projectDesktopItems } from "./desktopItemViewModel";
import "./DesktopItemGrid.css";

/** 定义桌面项目网格接收的快照与错误状态。 */
interface DesktopItemGridProps {
    snapshot?: DesktopItemSnapshot;
    failure?: string;
}

/** 呈现 Windows Shell 提供的真实桌面项目集合。 */
export const DesktopItemGrid = ({ snapshot, failure }: DesktopItemGridProps) => {
    // 当前快照对应的页面展示模型。
    const items = snapshot ? projectDesktopItems(snapshot) : [];

    return (
        <section className="desktop-items" aria-labelledby="desktop-items-title">
            <header className="desktop-items__header">
                <div>
                    <p className="desktop-items__eyebrow">Windows Shell</p>
                    <h2 id="desktop-items-title">桌面项目</h2>
                </div>
                <p className="desktop-items__summary">
                    {items.length} 个项目 · 修订 {snapshot?.revision ?? "--"}
                </p>
            </header>

            {failure && (
                <p className="desktop-items__error" role="alert">
                    {failure}
                </p>
            )}

            {snapshot && items.length === 0 && <p className="desktop-items__empty">桌面当前没有可显示项目</p>}

            <ul className="desktop-items__grid">
                {items.map((item) => (
                    <li className="desktop-item" data-availability={item.isAvailable ? "available" : "unavailable"} key={item.key}>
                        <div className="desktop-item__image" aria-hidden="true">
                            {item.imageSource && <img src={item.imageSource} alt="" />}
                        </div>
                        <span className="desktop-item__label" title={item.label}>
                            {item.label}
                        </span>
                        {!item.isAvailable && <span className="desktop-item__status">等待恢复</span>}
                    </li>
                ))}
            </ul>
        </section>
    );
};
