import trashIcon from "../../../resources/weather-trash.svg?raw";
import type { WeatherCity } from "../../contracts/weather";
import "./SettingsCityList.css";

/** 图标资源以原始 SVG 保存，尺寸与颜色由所在按键的样式决定。 */
const Glyph = ({ source }: { source: string }) => <span className="settings-city__glyph" aria-hidden="true" dangerouslySetInnerHTML={{ __html: source }} />;

/** 行政区副标题按来源实际返回的字段降级显示。 */
const subtitle = (city: WeatherCity) => [city.adm2, city.adm1, city.country].filter(Boolean).join("，");

/** 城市列表项：来源展开区与编辑弹窗的城市列表共用同一份渲染、右键菜单与删除入口。 */
export const SettingsCityList = ({
    cities,
    onSelect,
    onDelete,
    onMenu,
    onAdd,
    busy = false,
}: {
    cities: WeatherCity[]; // 该来源已添加的城市，顺序即添加顺序。
    onSelect?: (city: WeatherCity) => void; // 点击城市项；未传入时该项只呈现不可操作。
    onDelete?: (city: WeatherCity) => void; // 删除入口；未传入时不呈现删除图标与右键菜单。
    onMenu?: (city: WeatherCity, position: { left: number; top: number }) => void; // 右键弹出城市菜单，命令由调用方提供。
    onAdd?: () => void; // 末行“添加城市或地区”；未传入时不呈现该行。
    busy?: boolean; // 调用方的串行动作期间禁止城市项操作。
}) => (
    <div className="settings-city">
        {cities.map((city) => (
            <div
                key={city.id}
                className="settings-city__item"
                role={onSelect ? "button" : undefined}
                tabIndex={onSelect && !busy ? 0 : undefined}
                onClick={onSelect && !busy ? () => onSelect(city) : undefined}
                onKeyDown={(event) => {
                    if (!onSelect || busy || (event.key !== "Enter" && event.key !== " ")) return;
                    event.preventDefault();
                    onSelect(city);
                }}
                onContextMenu={(event) => {
                    if (!onDelete || !onMenu) return; // 没有可执行命令时不拦截系统菜单。
                    event.preventDefault();
                    if (!busy) onMenu(city, { left: event.clientX, top: event.clientY });
                }}
            >
                <span className="settings-city__main">
                    <span>{city.name}</span>
                    <small>{subtitle(city)}</small>
                </span>
                {onDelete && (
                    <button
                        type="button"
                        className="settings-city__delete"
                        aria-label={`删除 ${city.name}`}
                        title="删除"
                        disabled={busy}
                        onClick={(event) => {
                            event.stopPropagation(); // 删除按键单独接收点击，不触发城市项自身动作。
                            onDelete(city);
                        }}
                    >
                        <Glyph source={trashIcon} />
                    </button>
                )}
            </div>
        ))}
        {cities.length === 0 && <p className="settings-city__empty">尚未添加城市或地区</p>}
        {onAdd && (
            <button type="button" className="settings-city__add" disabled={busy} onClick={onAdd}>
                添加城市或地区
            </button>
        )}
    </div>
);
