import {
    WEATHER_SOURCES,
    type WeatherCity,
    type WeatherCredentials,
    type WeatherField,
    type WeatherSource,
} from "../../../contracts/weather.ts";

/** 天气字段的填写规则；设置界面的标签、说明与表单约束同属一份字段定义。 */
export const WEATHER_FIELDS: Record<
    WeatherField,
    { label: string; description: string; maxLength: number; secret?: boolean; multiline?: boolean; keepBlankOnEdit?: boolean }
> = {
    host: {
        label: "API Host",
        maxLength: 253,
        description:
            "从和风天气控制台完整复制 API Host，保留所有域名标签，例如 example.other.qweatherapi.com。仅填写主机名，不包含 https://、端口、路径、查询或片段。",
    },
    developerId: {
        label: "开发者 ID",
        maxLength: 64,
        description: "在和风天气控制台的账户设置中查看开发者 ID（Developer ID），以 Q 开头，共 10 位；不要填写邮箱或项目 ID。",
    },
    projectId: {
        label: "项目 ID",
        maxLength: 64,
        description: "在和风天气控制台的项目管理中创建或选择项目，复制项目 ID（Project ID）。该项目需具有城市搜索与天气预报接口权限。",
    },
    credentialId: {
        label: "凭据 ID",
        maxLength: 64,
        description: "在所选项目下创建 JWT 凭据并上传 Ed25519 公钥，复制对应的凭据 ID（Credential ID）。此处不是 API KEY 或临时 JWT 令牌。",
    },
    privateKey: {
        label: "私钥",
        maxLength: 1024,
        secret: true,
        multiline: true,
        keepBlankOnEdit: true,
        description:
            "按和风天气开发文档的 JWT 身份认证说明生成 Ed25519 密钥对：公钥上传到项目凭据，私钥保留在本机。导入对应的 PKCS#8 PEM 私钥文件，或粘贴包含 BEGIN PRIVATE KEY 和 END PRIVATE KEY 的完整内容；不要填写公钥或 JWT 令牌。",
    },
    apiKey: {
        label: "API key",
        maxLength: 1024,
        secret: true,
        keepBlankOnEdit: true,
        description:
            "在 Foreca My API 页面复制当前套餐的 API key，以 Bearer 令牌提交；该密钥是较长的签名令牌，需要完整复制，不要填入账户邮箱、密码或试用链接。",
    },
};

/** 弹窗关闭后恢复的空白草稿。 */
export const WEATHER_EMPTY_DRAFT: Record<WeatherField, string> = { host: "", developerId: "", projectId: "", credentialId: "", privateKey: "", apiKey: "" };

/** 编辑弹窗内尚未提交的城市改动。 */
export interface WeatherCityDraft {
    added: WeatherCity[]; // 本次新增、尚未写入来源列表的城市。
    removed: string[]; // 本次移除、尚未从来源列表删除的城市标识。
}

/** 新建一份空白城市草稿，不复用上次编辑的数组。 */
export const emptyWeatherCityDraft = (): WeatherCityDraft => ({ added: [], removed: [] });

/** 弹窗内必须填写的字段：新增时全部必填，编辑时允许敏感材料留空沿用已保存值。 */
export const requiredWeatherFields = (source: WeatherSource, adding: boolean): WeatherField[] =>
    WEATHER_SOURCES[source].fields.filter((key) => adding || !WEATHER_FIELDS[key].keepBlankOnEdit);

/** 按来源装配凭据；所有字段统一去首尾空白，不按字段名或来源特判。 */
export const assembleWeatherCredentials = (source: WeatherSource | undefined, draft: Record<WeatherField, string>): WeatherCredentials => {
    const fields = source ? WEATHER_SOURCES[source].fields : WEATHER_SOURCES.qweather.fields; // 没有打开弹窗时按第一个来源装配，调用方本就不会用到。
    const values = Object.fromEntries(fields.map((key) => [key, draft[key].trim()])) as Record<WeatherField, string>;
    return values as unknown as WeatherCredentials; // 后端按字段结构判定来源，这里只负责按来源装配字段。
};

/** 编辑弹窗内的城市列表＝已保存项减去待删除、加上待添加。 */
export const weatherDraftCities = (saved: WeatherCity[], draft: WeatherCityDraft): WeatherCity[] => [
    ...saved.filter((item) => !draft.removed.includes(item.id)),
    ...draft.added,
];

/** 确认一项候选城市：同标识替换，追加到草稿末尾，不产生重复行。 */
export const confirmWeatherCityDraft = (draft: WeatherCityDraft, city: WeatherCity): WeatherCityDraft => ({
    ...draft,
    added: [...draft.added.filter((item) => item.id !== city.id), city],
});

/** 撤回一项城市：本次刚加上的只从 added 撤回，不能同时记入 removed，否则保存时会删除尚不存在的一行。 */
export const withdrawWeatherCityDraft = (draft: WeatherCityDraft, city: WeatherCity): WeatherCityDraft => {
    const wasAdded = draft.added.some((item) => item.id === city.id); // 本次编辑内新加的城市。
    return {
        added: draft.added.filter((item) => item.id !== city.id),
        removed: wasAdded || draft.removed.includes(city.id) ? draft.removed : [...draft.removed, city.id],
    };
};

/** 保存时的城市提交顺序：先添加后移除，逐项独立提交，失败后重试不会重复处理同一项。 */
export const weatherCityCommitSteps = (draft: WeatherCityDraft): { additions: WeatherCity[]; removals: string[] } => ({
    additions: [...draft.added],
    removals: [...draft.removed],
});

/** 草稿与打开弹窗时的快照是否不同。 */
export const weatherDraftChanged = (draft: Record<WeatherField, string>, opened: string): boolean => JSON.stringify(draft) !== opened;

/** 离开或关闭弹窗的决策：保存与删除进行中直接阻止，有未保存修改先确认，否则放行。 */
export const weatherLeaveDecision = (state: { busy: string; dirty: boolean }): "block" | "proceed" | "confirm" => {
    if (state.busy === "save" || state.busy === "remove") return "block";
    return state.dirty ? "confirm" : "proceed";
};
