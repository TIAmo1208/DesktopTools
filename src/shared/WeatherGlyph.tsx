/** 桌面天气格与浮岛天气卡共用的天气图形；尺寸与颜色由所在容器决定。 */
export const WeatherGlyph = ({ code, className }: { code?: string; className?: string }) => (
    <svg className={className} viewBox="0 0 24 24" aria-hidden="true">
        <circle cx="12" cy="12" r="4" />
        <path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M4.93 19.07l1.41-1.41M17.66 6.34l1.41-1.41" />
        {code && Number(code) >= 300 && <path d="M7 17h9a3 3 0 0 0 0-6 4.5 4.5 0 0 0-8.5-1A3 3 0 0 0 7 17Z" />}
    </svg>
);
