import test from "node:test"; // Node 内置测试入口。
import assert from "node:assert/strict"; // 严格断言。
import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises"; // 构造临时夹具。
import { tmpdir } from "node:os"; // 系统临时目录。
import { join } from "node:path"; // 路径拼接。
import { 解析清单, 目标目录名, 生成同步计划 } from "./同步插件.mjs"; // 被测函数。

// 清单解析要跳过 BOM、注释与其他段，并保留值里的等号。
test("解析 manifest.ini 的 plugin 段", () => {
    const 清单 = 解析清单(
        "\uFEFF# 注释\r\n[plugin]\r\nid = plugin.theme.test\r\nname = 主题-测试\r\nauthor = 测试作者\r\ndescription = a = b\r\n[other]\r\nname = 忽略\r\n",
    );
    assert.equal(清单.id, "plugin.theme.test");
    assert.equal(清单.name, "主题-测试");
    assert.equal(清单.author, "测试作者");
    assert.equal(清单.description, "a = b");
});

// 安装写入的目录名是 作者@名称；缺作者时退回名称，缺名称直接报错。
test("目录名按作者与名称生成", () => {
    assert.equal(目标目录名({ name: "主题-测试", author: "测试作者" }), "测试作者@主题-测试");
    assert.equal(目标目录名({ name: "主题-测试" }), "主题-测试");
    assert.throws(() => 目标目录名({ author: "测试作者" }), /缺少 name/);
});

// 同 id 的插件沿用目标已有目录，避免开发目录出现两份注册；示例默认跳过。
test("同步计划复用同 id 目录并默认跳过示例", async () => {
    const 根 = await mkdtemp(join(tmpdir(), "desktop-tools-sync-"));
    try {
        const 源根 = join(根, "plugin");
        const 目标根 = join(根, "target");
        await mkdir(join(源根, "主题-测试"), { recursive: true });
        await writeFile(join(源根, "主题-测试", "manifest.ini"), "[plugin]\nid = plugin.theme.test\nname = 主题-测试\nauthor = 测试作者\n");
        await mkdir(join(源根, "示例", "示例-最小格子"), { recursive: true });
        await writeFile(
            join(源根, "示例", "示例-最小格子", "manifest.ini"),
            "[plugin]\nid = plugin.example.minimal\nname = 示例-最小格子\nauthor = 测试作者\n",
        );
        await mkdir(join(目标根, "旧目录"), { recursive: true });
        await writeFile(join(目标根, "旧目录", "manifest.ini"), "[plugin]\nid = plugin.theme.test\nname = 旧名\n");

        const 默认计划 = await 生成同步计划({ 源根, 目标根 });
        assert.equal(默认计划.length, 1);
        assert.equal(默认计划[0].目标目录, join(目标根, "旧目录"));
        assert.equal(默认计划[0].是否新增, false);

        const 含示例计划 = await 生成同步计划({ 源根, 目标根, 包含示例: true });
        assert.equal(含示例计划.length, 2);
        assert.ok(含示例计划.some((项) => 项.名称 === "示例-最小格子"));
    } finally {
        await rm(根, { recursive: true, force: true });
    }
});
