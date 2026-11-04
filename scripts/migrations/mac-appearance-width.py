#!/usr/bin/env python3
"""显式将 Mac 输入区绝对最大宽度转换为可用宽度比例。需先退出 Velune。

以 1280 pt 可用宽度为固定参考，旧值 / 1280，再钳制到 50%—100%；
旧默认 960 pt 对应新默认 75%。预览为默认，--apply 原子写入。
"""
import argparse
import json
import math
import os
from pathlib import Path
import shutil
import tempfile


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--home", required=True, type=Path)
    parser.add_argument("--apply", action="store_true")
    args = parser.parse_args()
    home = args.home.expanduser().resolve(strict=True)
    target = home / "mac-appearance.json"
    with (home / "runtime.lock").open("a+b") as lock:
        if os.name == "nt":
            import msvcrt
            lock.seek(0)
            msvcrt.locking(lock.fileno(), msvcrt.LK_NBLCK, 1)
        else:
            import fcntl
            fcntl.flock(lock.fileno(), fcntl.LOCK_EX | fcntl.LOCK_NB)
        if not target.exists():
            print("没有 Mac 外观配置，无需迁移")
            return
        value = json.loads(target.read_text())
        if "composerWidthRatio" in value:
            ratio = value["composerWidthRatio"]
            if isinstance(ratio, bool) or not isinstance(ratio, (int, float)) or not math.isfinite(ratio) or not 0.5 <= ratio <= 1:
                raise ValueError("输入区宽度比例无效；原配置保持不变，请先核对")
            if "composerWidth" in value:
                raise ValueError("外观配置同时含旧宽度与新比例；原配置保持不变，请先核对")
            print("Mac 输入区宽度已为比例，无需迁移")
            return
        width = value.get("composerWidth")
        if isinstance(width, bool) or not isinstance(width, (int, float)) or not math.isfinite(width) or not 640 <= width <= 1440:
            raise ValueError("旧输入区宽度无效；原配置保持不变，请先核对")
        ratio = min(max(width / 1280, 0.5), 1)
        value.pop("composerWidth")
        value["composerWidthRatio"] = ratio
        print(f"Mac 输入区宽度：{width:g} pt → {ratio:.1%}（以 1280 pt 可用宽度为参考，钳制到 50%—100%）")
        if not args.apply:
            print("预览完成；传 --apply 写入")
            return
        backup = home / "mac-appearance.before-width-ratio.json"
        if backup.exists():
            if backup.read_bytes() != target.read_bytes():
                raise ValueError("已有不同的迁移前外观备份；原配置保持不变，请先核对")
        else:
            shutil.copy2(target, backup)
        temporary = None
        try:
            with tempfile.NamedTemporaryFile(mode="w", encoding="utf-8", dir=home, prefix=".mac-appearance-", delete=False) as output:
                temporary = Path(output.name)
                json.dump(value, output, ensure_ascii=False, indent=2)
                output.write("\n")
                output.flush()
                os.fsync(output.fileno())
            os.chmod(temporary, target.stat().st_mode & 0o777)
            os.replace(temporary, target)
        finally:
            if temporary is not None:
                temporary.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
