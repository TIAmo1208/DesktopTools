; 覆盖模板的强杀回退；钩子结束后新启动的实例也只能阻止安装。
!macroundef CheckIfAppIsRunning
!macro CheckIfAppIsRunning executableName productName
  nsis_tauri_utils::FindProcessCurrentUser "${executableName}"
  Pop $R0
  ${If} $R0 = 0
    SetErrorLevel 1
    Abort "Desktop Tools 仍在运行，请正常退出后重试。"
  ${EndIf}
!macroend

; 覆盖与卸载仅请求正常退出，为桌面恢复与 guardian 清理保留时间。
!macro DESKTOP_TOOLS_STOP
  nsis_tauri_utils::FindProcessCurrentUser "desktop-tools.exe"
  Pop $R0
  ${If} $R0 = 0
    ${IfNot} ${FileExists} "$INSTDIR\desktop-tools.exe"
      SetErrorLevel 1
      Abort "请先退出正在运行的 Desktop Tools，再重新执行安装或卸载。"
    ${EndIf}
    nsExec::ExecToStack /TIMEOUT=15000 '"$INSTDIR\desktop-tools.exe" --desktop-menu exit'
    Pop $R0
    Pop $R1
    ${If} $R0 != 0
      SetErrorLevel 1
      Abort "无法请求 Desktop Tools 正常退出，请退出程序后重试。"
    ${EndIf}
  ${EndIf}

  ; 无可靠的跨进程退出事件接口，仅在此阶段最多检查 15 秒。
  StrCpy $R2 0
  ${Do}
    nsis_tauri_utils::FindProcessCurrentUser "desktop-tools.exe"
    Pop $R0
    nsis_tauri_utils::FindProcessCurrentUser "desktop-tools-guardian.exe"
    Pop $R1
    ${If} $R0 != 0
    ${AndIf} $R1 != 0
      ${ExitDo}
    ${EndIf}
    ${If} $R2 >= 60
      SetErrorLevel 1
      Abort "Desktop Tools 或 guardian 尚未退出，已停止操作。请确认桌面恢复后重试。"
    ${EndIf}
    Sleep 250
    IntOp $R2 $R2 + 1
  ${Loop}
!macroend

; 安装前阻止默认安装模板强制结束正在恢复桌面的程序。
!macro NSIS_HOOK_PREINSTALL
  !insertmacro DESKTOP_TOOLS_STOP
!macroend

; 文件就位后先写入预设插件，再沿用应用的菜单注册入口。
!macro NSIS_HOOK_POSTINSTALL
  nsExec::ExecToStack /TIMEOUT=60000 '"$INSTDIR\desktop-tools.exe" --install-preset-plugins "$INSTDIR\plugin" "$APPDATA\${BUNDLEID}"'
  Pop $R0
  Pop $R1
  ${If} $R0 != 0
    SetErrorLevel 1
    Abort "程序已安装，但预设插件安装失败。请重新运行安装程序。"
  ${EndIf}
  nsExec::ExecToStack /TIMEOUT=15000 '"$INSTDIR\desktop-tools.exe" --register-desktop-menu'
  Pop $R0
  Pop $R1
  ${If} $R0 != 0
    SetErrorLevel 1
    Abort "程序文件已安装，但桌面菜单注册失败。请重新运行安装程序。"
  ${EndIf}
!macroend

; 先等待正常退出，再清理菜单；用户勾选时一并移除随程序安装的预设插件。
!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro DESKTOP_TOOLS_STOP
  StrCpy $DeleteAppDataCheckboxState 0
  nsExec::ExecToStack /TIMEOUT=15000 '"$INSTDIR\desktop-tools.exe" --unregister-desktop-menu'
  Pop $R0
  Pop $R1
  ${If} $R0 != 0
    SetErrorLevel 1
    Abort "桌面菜单移除失败，已保留程序文件。请重试卸载。"
  ${EndIf}
  ${If} $DeletePresetPluginsCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    nsExec::ExecToStack /TIMEOUT=60000 '"$INSTDIR\desktop-tools.exe" --uninstall-preset-plugins "$INSTDIR\plugin" "$APPDATA\${BUNDLEID}"'
    Pop $R0
    Pop $R1
    ${If} $R0 != 0
      ; 预设插件清理失败不阻塞卸载，用户可在插件目录手动删除。
      MessageBox MB_ICONEXCLAMATION "随程序安装的预设插件未能全部移除，可稍后在插件目录手动删除。程序将继续卸载。"
    ${EndIf}
  ${EndIf}
!macroend
