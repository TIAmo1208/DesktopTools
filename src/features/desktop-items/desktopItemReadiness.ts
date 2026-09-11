const images = new Set<HTMLImageElement>(); // 当前窗口由桌面项目组件实际挂载的系统图像，不包含拖动副本。

/** 注册系统图像并在 React 移除节点时释放，宿主无需了解项目 DOM。 */
export const registerDesktopItemImage = (image: HTMLImageElement | null) => {
    if (!image) return;
    images.add(image);
    return () => {
        images.delete(image);
    };
};

/** 汇总实际呈现图像的解码结果，保留既有失败重试与空集合门槛。 */
export const waitForDesktopItemImages = async (): Promise<{ total: number; loaded: number }> => {
    // 每轮重新读取注册集合，避免把已卸载图像计入就绪结果。
    for (let attempt = 0; attempt < 20; attempt += 1) {
        const current = [...images]; // 本轮需要完成解码的图像。
        await Promise.all(current.map((image) => image.decode().catch(() => undefined)));
        const loaded = [...images].filter((image) => image.naturalWidth > 0).length; // 仍然挂载且已有像素的图像数。
        if (loaded === images.size) return { total: images.size, loaded };
        await new Promise((resolve) => window.setTimeout(resolve, 100));
    }
    return { total: images.size, loaded: [...images].filter((image) => image.naturalWidth > 0).length };
};
