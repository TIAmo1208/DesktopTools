import arrowLeft from "../../../resources/weather-back.svg?raw";
import check from "../../../resources/weather-check.svg?raw";
import chevronRight from "../../../resources/weather-chevron-right.svg?raw";
import close from "../../../resources/weather-close.svg?raw";
import eye from "../../../resources/weather-eye.svg?raw";
import eyeOff from "../../../resources/weather-eye-off.svg?raw";
import search from "../../../resources/weather-search.svg?raw";

/** 天气设置使用的 Lucide 图标；尺寸与颜色由所在按键的样式决定。 */
const Icon = ({ source }: { source: string }) => <span className="weather-settings__glyph" aria-hidden="true" dangerouslySetInnerHTML={{ __html: source }} />;

/** 返回功能格子列表。 */
export const BackIcon = () => <Icon source={arrowLeft} />;

/** 关闭当前弹窗。 */
export const CloseIcon = () => <Icon source={close} />;

/** 指示整项可以进入详情。 */
export const ChevronRightIcon = () => <Icon source={chevronRight} />;

/** 显示当前敏感草稿。 */
export const RevealIcon = () => <Icon source={eye} />;

/** 遮蔽当前敏感草稿。 */
export const ConcealIcon = () => <Icon source={eyeOff} />;

/** 提交城市搜索。 */
export const SearchIcon = () => <Icon source={search} />;

/** 已通过的能力验证项。 */
export const PassedIcon = () => <Icon source={check} />;
