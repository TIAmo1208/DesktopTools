/** 已接入的天气来源；一次只启用一家，不自动失败切源。 */
export type WeatherSource = "qweather" | "foreca";

/** 设置页按来源展示的显示名、认证模式与提交字段。 */
export const WEATHER_SOURCES: Record<WeatherSource, { name: string; mode: string; fields: readonly WeatherField[] }> = {
    qweather: { name: "和风天气", mode: "JWT 凭据", fields: ["host", "developerId", "projectId", "credentialId", "privateKey"] },
    foreca: { name: "Foreca", mode: "API key", fields: ["apiKey"] },
};

/** 设置页按来源收集的敏感字段；提交时只发送当前来源声明的字段。 */
export type WeatherField = "host" | "developerId" | "projectId" | "credentialId" | "privateKey" | "apiKey";

/** 和风账户凭据；字段集合与 Foreca 不相交，后端按结构判定来源。 */
export interface QweatherCredentials {
    host: string; // 账户 API Host。
    developerId: string; // JWT iss。
    projectId: string; // JWT sub。
    credentialId: string; // JWT kid。
    privateKey: string; // 当前编辑的 PEM 草稿。
}

/** Foreca 账户凭据。 */
export interface ForecaCredentials {
    apiKey: string; // 控制台签发的 Bearer 密钥。
}

/** 只提交当前来源的字段，不保存到浏览器存储。 */
export type WeatherCredentials = QweatherCredentials | ForecaCredentials;

/** 会话内搜索结果，不加入工作区或偏好快照。 */
export interface WeatherCity {
    id: string; // 供应商位置身份。
    name: string; // 城市名称。
    adm1: string; // 一级行政区。
    adm2: string; // 二级行政区。
    country: string; // 国家。
    lat: string; // 纬度。
    lon: string; // 经度。
    tz: string; // IANA 时区。
    fxLink: string; // 对应城市详情。
}

/** 来源可用状态；未确认表示已配置但本次启动尚未请求过。 */
export type WeatherAvailability = "unconfirmed" | "available" | "unavailable";

/** 一个已配置来源的列表状态。 */
export interface WeatherSourceStatus {
    source: WeatherSource; // 来源标识。
    address: string; // 列表副标题显示的服务地址。
    identity: Partial<Record<WeatherField, string>>; // 编辑模式可回填的非敏感字段，敏感材料不回读。
    availability: WeatherAvailability; // 最近一次请求结果。
    enabled: boolean; // 是否当前启用的来源。
}

/** 一项能力验证结果；名称与项数由后端按当前草稿来源给出。 */
export interface WeatherCheck {
    label: string; // 验证项名称。
    failure: { message: string; retryAt: number | null } | null; // null 表示该项通过。
}

/** 按当前来源逐项返回的验证结果与列表状态。 */
export interface WeatherSettingsSnapshot {
    sessionId: number | null; // 后端权威草稿身份。
    cities: WeatherCity[]; // 临时城市结果。
    checks: WeatherCheck[]; // 空数组表示尚未验证。
    sources: WeatherSourceStatus[]; // 未配置来源不出现在列表中。
}

/** 保存只提交会话身份，后端自行验证保存条件；探测与删除只提交来源。 */
export type WeatherSettingsCommand =
    | { type: "status" }
    | { type: "begin"; credentials: WeatherCredentials }
    | { type: "search"; sessionId: number; query: string }
    | { type: "validate"; sessionId: number; cityId: string }
    | { type: "save" | "cancel"; sessionId: number }
    | { type: "probe" | "remove"; source: WeatherSource };
