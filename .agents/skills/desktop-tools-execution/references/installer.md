# installer：安装包验证

只在安装器、分发资源、版本或卸载行为变化时读本页。构建前提见 `docs/agents/构建运行.md`，验证脚本见 `scripts/验证安装包.ps1`。

## 安全前置（脚本也会拒绝）

- `HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\Desktop Tools` 不存在；存在说明本机已有正式安装，换独立环境验证。
- `build/安装包 验证` 不存在；存在说明上次验证未收尾，先检查再清理。
- `desktop-tools`、`desktop-tools-guardian` 不在运行。
- 不在用户正在使用的安装目录上验证。

## 步骤

```powershell
npm run tauri:installer
.\scripts\验证安装包.ps1 -Installer build\nsis\Desktop-Tools_<版本>_x64-setup.exe
```

- `npm run tauri:installer` 会先跑 `pretauri:installer` 生成许可文件，并已注入 `TMP`。
- 脚本在独立中文空格目录验证首次安装、覆盖安装与卸载，检查两个二进制与安装登记，结束后恢复原桌面菜单与安装器偏好。
- 验证通过只代表安装器行为；不启动宿主，不证明 UI、插件或桌面接管正确。

## 停止条件

- 出现正式安装登记、运行实例或旧测试目录：停止并报告，不自动删除、不强装。
- 脚本退出码非 0 或安装超时：保留现场，读日志与注册表快照，不反复重跑。
- 卸载后注册表或桌面菜单未恢复：按脚本备份文件人工核对，不继续下一轮。
