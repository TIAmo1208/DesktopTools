/** 仅用于设置页提交的敏感草稿，不保存到浏览器存储。 */
export interface WeatherCredentials {
    host: string; // 账户 API Host。
    developerId: string; // JWT iss。
    projectId: string; // JWT sub。
    credentialId: string; // JWT kid。
    privateKey: string; // 当前编辑的 PEM 草稿。
}

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

/** 五项检查按城市、当前、小时、日预报、来源顺序返回。 */
export interface WeatherSettingsSnapshot {
    configured: boolean; // 仅表示安全存储记录存在。
    sessionId: number | null; // 后端权威草稿身份。
    cities: WeatherCity[]; // 临时城市结果。
    checks: ({ message: string; retryAt: number | null } | null)[]; // null 表示该项通过。
}

/** 保存只提交会话身份，后端自行验证保存条件。 */
export type WeatherSettingsCommand =
    | { type: "status" | "clear" }
    | { type: "begin"; credentials: WeatherCredentials }
    | { type: "search"; sessionId: number; query: string }
    | { type: "validate"; sessionId: number; cityId: string }
    | { type: "save" | "cancel"; sessionId: number };
