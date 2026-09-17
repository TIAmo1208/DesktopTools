---
status: accepted
---

# 通过 Shell 身份维护桌面项目引用

## 已落实的桌面项目来源

当前 `platform/windows/shell.rs` 从 `FOLDERID_Desktop` 与 `FOLDERID_PublicDesktop` 对应的真实目录枚举项目，系统桌面项目由 `desktop_sources.rs` 按配置和策略决定显隐。当前覆盖计算机、用户文件、网络、回收站和控制面板五类系统图标，全部缺省时仅显示回收站；读取错误不发布部分来源结果。该覆盖范围不应表述为支持任意第三方桌面命名空间扩展。

来源筛选决定实际桌面项目集合，Shell 身份解析负责项目引用与操作能力；桌面集合由真实目录来源和系统图标显隐策略共同确定。名称与布局协调的提交规则见 [ADR-0016](0016-正式项目架构.md)。

### 官方契约参考

- [Shell Namespace](https://learn.microsoft.com/en-us/windows/win32/shell/namespace-intro)：PIDL 由所属 Shell 文件夹解释，文件路径不能覆盖全部项目身份。
- [SIGDN](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/ne-shobjidl_core-sigdn)：显示名、解析名和文件系统路径的区别。
- [IFileOperation](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperation) 与 [IFileOperationProgressSink](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-ifileoperationprogresssink)：操作及实际结果确认。
- [IContextMenu3](https://learn.microsoft.com/en-us/windows/win32/api/shobjidl_core/nn-shobjidl_core-icontextmenu3)：Shell 菜单消息处理。
- [Shell Data Object](https://learn.microsoft.com/en-us/windows/win32/shell/dataobject) 与 [RegisterDragDrop](https://learn.microsoft.com/en-us/windows/win32/api/ole2/nf-ole2-registerdragdrop)：原生传输与 OLE 线程消息循环要求。
- [SHChangeNotifyRegister](https://learn.microsoft.com/en-us/windows/win32/api/shlobj_core/nf-shlobj_core-shchangenotifyregister)：命名空间变化通知，通知后重新查询事实。

Shell 操作代理在运行期持有 opaque Shell 项目标识，持久化层保存可重新解析的桌面项目身份和补充信息。重命名通知协调旧、新项目身份，文件路径作为文件系统项目的补充属性，暂时无法解析的项目引用保持可恢复状态，并继续参与 Desktop Tools 逻辑拖动、围栏归属、排序和布局事务；进入 Shell 投放目标时使用共同 Shell 能力和系统操作结果。F2 编辑期间保持项目当前身份与位置，Shell 确认后协调新旧身份并触发布局重排，选择、焦点和可见性继续跟随新身份。自动排序属性暂缺时，项目位于已具备属性的项目之后并保持已有稳定顺序，属性就绪后按完整规则重新排序；Shell 属性变化通知在 `250 ms` 窗口内合并后触发一次重排。拖动期间的 Shell 变化通知进入队列，拖动完成或取消后按最终 Shell 事实更新参与容器。Shell 确认项目删除或移出桌面命名空间后，该引用退出活动布局，历史快照继续保留恢复信息；项目由外部程序重新进入桌面命名空间时作为新项目进入未归类区域。Desktop Tools 撤销移入回收站或移出桌面的操作时，恢复操作前保存的共享围栏归属，以及工作区中的顺序和位置。布局撤销遇到外部状态变化时，仍在活动布局或暂时失效的引用参与恢复，Shell 已确认离开命名空间的项目保留历史引用，其他项目完成恢复并记录部分恢复结果。Shell 返回结果是项目能力和操作结果的权威来源。
