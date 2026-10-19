/** 已接入的天气来源；多个来源可以同时配置，来源归属由每个天气格子自己选择。 */
export type WeatherSource = "qweather" | "foreca";

/** 设置页按来源展示的显示名、认证模式、提交字段与底栏署名；来源之间只有这些内容不同。 */
export const WEATHER_SOURCES: Record<WeatherSource, { name: string; mode: string; fields: readonly WeatherField[]; attribution: string }> = {
    qweather: { name: "和风天气", mode: "JWT 凭据", fields: ["host", "developerId", "projectId", "credentialId", "privateKey"], attribution: "和风" },
    foreca: { name: "Foreca", mode: "API key", fields: ["apiKey"], attribution: "Powered by Foreca" },
};

/** 按来源固定署名；接口返回的署名是跳转地址，不适合放进紧凑底栏。 */
export const weatherAttribution = (source: WeatherSource) => WEATHER_SOURCES[source].attribution;

/** 按来源验证后唯一可得的结论项；逐项能力清单目前没有页面呈现，两处调用方共用同一条占位结论。 */
export const SOURCE_CITY_CHECK = { label: "来源与城市", failure: null } as const;

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
    cities: WeatherCity[]; // 该来源已添加的城市列表，展开区与格子菜单共用。
}

/** 天气格子自己保存的取数归属，只有两个标识，不保存城市对象。 */
export interface WeatherTileConfig {
    source: WeatherSource; // 格子选择的来源。
    cityId: string; // 该来源城市列表中的城市标识。
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
    | { type: "save-city"; sessionId: number; cityId: string } // 把验证通过的城市写入该来源城市列表。
    | { type: "probe" | "remove"; source: WeatherSource };

/** 校验一个城市对象的字段类型与长度；供应商身份与坐标由后端负责。 */
const parseWeatherCity = (value: unknown): WeatherCity => {
    if (typeof value !== "object" || value === null) throw new Error("天气城市数据无效");
    const city = value as Record<string, unknown>; // 等待边界校验的原始城市。
    for (const key of ["id", "name", "adm1", "adm2", "country", "lat", "lon", "tz", "fxLink"] as const) {
        const field = city[key]; // 当前城市字段。
        if (typeof field !== "string" || field.length > 1024) throw new Error("天气城市数据无效");
    }
    return {
        id: city.id as string,
        name: city.name as string,
        adm1: city.adm1 as string,
        adm2: city.adm2 as string,
        country: city.country as string,
        lat: city.lat as string,
        lon: city.lon as string,
        tz: city.tz as string,
        fxLink: city.fxLink as string,
    };
};

/** 校验一个已配置来源的列表状态；启用字段已随全局来源偏好删除。 */
export const parseWeatherSourceStatus = (value: unknown): WeatherSourceStatus => {
    if (typeof value !== "object" || value === null) throw new Error("天气来源数据无效");
    const candidate = value as Record<string, unknown>; // 等待边界校验的原始来源状态。
    if (candidate.source !== "qweather" && candidate.source !== "foreca") throw new Error("天气来源无效");
    if (typeof candidate.address !== "string") throw new Error("天气来源地址无效");
    if (candidate.availability !== "unconfirmed" && candidate.availability !== "available" && candidate.availability !== "unavailable")
        throw new Error("天气来源可用状态无效");
    if (!Array.isArray(candidate.cities)) throw new Error("天气来源城市列表无效");
    const identity = candidate.identity === undefined ? {} : candidate.identity; // 编辑回填字段可以缺失。
    if (typeof identity !== "object" || identity === null) throw new Error("天气来源身份字段无效");
    return {
        source: candidate.source,
        address: candidate.address,
        identity: identity as WeatherSourceStatus["identity"],
        availability: candidate.availability,
        cities: candidate.cities.map(parseWeatherCity),
    };
};

/** 校验设置命令返回的完整快照；缺少城市列表的来源状态按无效处理。 */
export const parseWeatherSettingsSnapshot = (value: unknown): WeatherSettingsSnapshot => {
    if (typeof value !== "object" || value === null) throw new Error("天气设置数据无效");
    const candidate = value as Record<string, unknown>; // 等待边界校验的原始快照。
    const sessionId = candidate.sessionId === undefined ? null : candidate.sessionId; // 只有创建的草稿才有会话身份。
    if (sessionId !== null && (!Number.isSafeInteger(sessionId) || (sessionId as number) <= 0)) throw new Error("天气会话身份无效");
    if (!Array.isArray(candidate.cities) || !Array.isArray(candidate.checks) || !Array.isArray(candidate.sources)) throw new Error("天气设置数据无效");
    return {
        sessionId: sessionId as number | null,
        cities: candidate.cities.map(parseWeatherCity),
        checks: candidate.checks as WeatherCheck[],
        sources: candidate.sources.map(parseWeatherSourceStatus),
    };
};

/** 从格子实例状态独立读取两个标识；只选来源、尚未选城市时来源仍然有效。 */
export const parseWeatherTileBinding = (state: unknown): { source?: WeatherSource; cityId?: string } => {
    if (typeof state !== "object" || state === null) return {};
    const candidate = state as Record<string, unknown>; // 格子自己保存的状态载荷。
    const source = candidate.weatherSource; // 格子选择的来源标识。
    const cityId = candidate.weatherCityId; // 该来源城市列表中的城市标识。
    return {
        source: source === "qweather" || source === "foreca" ? source : undefined,
        cityId: typeof cityId === "string" && cityId ? cityId : undefined,
    };
};

/** 从格子实例状态读取取数归属；两个标识缺一即表示尚未选择。 */
export const parseWeatherTileConfig = (state: unknown): WeatherTileConfig | undefined => {
    const { source, cityId } = parseWeatherTileBinding(state);
    return source && cityId ? { source, cityId } : undefined;
};
