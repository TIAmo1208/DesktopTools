/** 原时钟的 zh-CN 呈现契约；生产使用系统时区，固定时区仅用于跨端对照。 */
export const clockText = (unixMillis: number | null | undefined, hour12: boolean, seconds: boolean, timeZone?: string) =>
    unixMillis
        ? new Intl.DateTimeFormat("zh-CN", { hour: "2-digit", minute: "2-digit", second: seconds ? "2-digit" : undefined, hour12, timeZone }).format(new Date(unixMillis))
        : "--:--:--";
