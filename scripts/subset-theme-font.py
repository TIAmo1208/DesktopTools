"""按 GB2312 一级字 + ASCII + 常用标点裁剪思源宋体，生成单文件 woff2。

只在打包字体时运行一次，产物随插件进仓库；仓库不保留全量字体。
用法：python scripts/subset-theme-font.py <源字体> <输出.woff2>
"""

import io
import subprocess
import sys

# ASCII 可打印字符。
ASCII = [chr(code) for code in range(0x20, 0x7F)]
# GB2312 一级汉字（按拼音序的 3755 个常用字）。
LEVEL_ONE = []
for high in range(0xB0, 0xD8):
    for low in range(0xA1, 0xFF):
        try:
            LEVEL_ONE.append(bytes([high, low]).decode("gb2312"))
        except UnicodeDecodeError:
            continue
# 界面上常用、但不在一级字里的符号与全角标点。
EXTRA = list("　、。，．·；：！？…—～《》〈〉「」『』【】〔〕（）％＃＠＆＊＋－／＝＜＞？［］｛｝｜ＡＢＣ〇")
CHARS = sorted(set("".join(ASCII) + "".join(LEVEL_ONE) + "".join(EXTRA)))
UNICODES = ",".join(f"U+{ord(character):04X}" for character in CHARS)

def main() -> None:
    source, target = sys.argv[1], sys.argv[2]
    subprocess.run(
        [
            sys.executable, "-m", "fontTools.subset", source,
            f"--unicodes={UNICODES}",
            "--layout-features=*",
            "--flavor=woff2",
            f"--output-file={target}",
        ],
        check=True,
    )
    print(f"characters={len(CHARS)}")

if __name__ == "__main__":
    main()
