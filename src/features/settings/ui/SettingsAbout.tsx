import { useLayoutEffect, useRef, useState } from "react";

import "./SettingsAbout.css";

/** 说明文本：默认收起为一行，超出一行才提供展开；插件与浮岛卡片二级页共用同一呈现。 */
export const SettingsAbout = ({ text }: { text: string }) => {
    const textRef = useRef<HTMLSpanElement>(null); // 用来量是否超出一行。
    const toggleRef = useRef<HTMLButtonElement>(null); // 展开按钮；测量时临时隐藏，避免按钮自占宽度。
    const [open, setOpen] = useState(false); // 展开后显示全文。
    const [overflow, setOverflow] = useState(false); // 收起时是否放不下。
    useLayoutEffect(() => {
        // 展开态保留「收起」；只在收起态按当前容器宽度重新判断是否需要「显示」。
        if (open) return;
        const measure = () => {
            const node = textRef.current;
            if (!node) return;
            const toggle = toggleRef.current;
            const previous = toggle?.style.display;
            if (toggle) toggle.style.display = "none";
            setOverflow(node.scrollWidth > node.clientWidth + 1);
            if (toggle) toggle.style.display = previous ?? "";
        };
        measure();
        const observer = new ResizeObserver(measure); // 窗口或字号变化会改变可用宽度，必须重测。
        if (textRef.current) observer.observe(textRef.current);
        return () => observer.disconnect();
    }, [text, open]);
    if (!text.trim()) return null;
    return (
        <p className="settings-about">
            <span ref={textRef} className={open ? "settings-about__text settings-about__text--open" : "settings-about__text"}>
                {text}
            </span>
            {overflow || open ? (
                <button ref={toggleRef} className="settings-about__toggle" type="button" onClick={() => setOpen((current) => !current)} data-button="ghost">
                    {open ? "收起" : "显示"}
                </button>
            ) : null}
        </p>
    );
};
