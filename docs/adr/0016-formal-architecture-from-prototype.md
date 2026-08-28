---
status: accepted
triage: ready-for-agent
type: architecture-spec
---

# 从原型验证结果演进正式项目架构

## Problem Statement

Desktop Tools 已通过 `native-windowed-adapter` 原型验证透明桌面宿主、逐屏 WebView2、混合 DPI、桌面接管、交互命中、围栏操作、Explorer 重启和 guardian 恢复等关键行为。原型代码把宿主、Shell 访问、布局状态、前端呈现和恢复流程集中在验证路径中，正式项目需要一套按功能职责拆分的结构承接这些已确认行为。

当前架构需要同时满足以下事实：

- Desktop Tools 拥有桌面项目的呈现与交互，Windows Explorer 提供桌面命名空间、Shell 操作和恢复兜底。
- React/WebView2 负责视觉状态和交互意图，Rust 负责领域规则、Windows 能力和应用生命周期。
- Shell 项目身份与 Desktop Tools 自有布局分别维护，Shell 返回结果作为文件操作事实来源。
- 每台显示器拥有独立透明桌面宿主，宿主需要维护物理工作区、DPI、WebView2 栅格化比例和交互命中图。
- 启动失败、正常退出、Explorer 重启、主进程崩溃和宿主失效都需要进入统一恢复契约。

## Solution

正式项目采用单应用双边界架构：

1. React 前端按产品功能组织工作区、桌面项目、围栏、文件夹门户、组件和设置，并通过类型化 bridge 表达命令与事件。
2. Rust 后端按桌面宿主、Shell、Workspace Domain、Persistence、Component Runtime 和 Recovery 组织功能模块。
3. Tauri command/event 作为前后端最高测试 seam，承担参数校验、快照发布和结果回传；平台实现留在 Rust 的 Windows 适配层。
4. Shell Adapter 在专用 STA Broker 中运行，宿主线程持有对应 HWND 和 OLE 生命周期。
5. 独立 Rust guardian 负责接管前的状态登记、运行期间观察和异常恢复；业务状态、布局和 Shell 操作结果由主进程持有。
6. 原型目录保持冻结，原型中的宿主行为、状态门槛和日志事件作为正式实现的迁移参考。

正式项目的功能数据流为：

```text
React Feature
  -> typed bridge
  -> Tauri command/event
  -> application command boundary
  -> Workspace / Shell / Persistence / Component Runtime
  -> Windows platform adapter
  -> Explorer / WebView2 / filesystem
  -> immutable snapshot + fact event
  -> React Feature
```

## User Stories

1. As an end user, I want Desktop Tools to present my complete desktop project collection, so that I can organize the Windows desktop from one workspace.
2. As an end user with multiple monitors, I want each monitor to have an independent transparent desktop surface, so that wallpaper and taskbar work areas remain correct on every display.
3. As an end user changing display scaling, I want each host to apply the target monitor DPI and WebView2 rasterization scale, so that content and input coordinates remain aligned.
4. As an end user of the desktop, I want icons, names, and system images to come from Windows Shell, so that displayed projects match the system namespace.
5. As an end user whose project is temporarily unavailable, I want its reference and layout membership preserved, so that the project can recover when Windows resolves it again.
6. As an end user, I want to create, move, resize, collapse, expand, lock, sort, and dissolve fences, so that desktop projects remain organized around my workflow.
7. As an end user, I want fence membership to be a logical relationship, so that moving a project between fences keeps its source location stable.
8. As an end user, I want every Windows virtual desktop to have an independent layout, so that each virtual desktop can present the same projects with its own fence and component arrangement.
9. As an end user switching virtual desktops, I want the active layout, selection collection, focus, and drag state to switch coherently, so that a desktop transition cannot leave stale interaction state behind.
10. As an end user, I want single selection, multi-selection, range selection, keyboard navigation, and focus containers to follow Windows interaction conventions, so that the workspace feels predictable.
11. As an end user dragging projects, I want previews, insertion targets, auto-scroll, group movement, and Escape cancellation, so that layout edits remain reversible and understandable.
12. As an end user dragging a project into a fence, I want the logical membership update and visual placement to commit as one transaction, so that partial layout changes do not appear.
13. As an end user using the desktop context menu, I want Windows Shell commands and Desktop Tools commands to appear in the correct target menu, so that native operations and product operations share one interaction entry.
14. As an end user renaming, deleting, copying, moving, or restoring a project, I want Windows Shell results to determine the final state, so that the workspace reflects actual file-system and Shell outcomes.
15. As an end user using clipboard cut and paste, I want cut state to appear across active virtual desktops, so that the same project has one visible transfer state.
16. As an end user receiving an external Shell change, I want created, deleted, renamed, and property-updated projects to reconcile into the active layouts, so that the workspace follows Windows changes.
17. As an end user, I want automatic snapshots, manual backups, Schema migration, and restore, so that layout work can survive application restarts and recover from configuration errors.
18. As an end user restoring a backup, I want the current Windows virtual desktop preserved and matching layouts restored by stable identity, so that restoration does not unexpectedly change my desktop context.
19. As an end user, I want folder portals to present directory contents as an interactive workspace view, so that high-volume project groups remain accessible from the desktop surface.
20. As an end user, I want official components to run inside the workspace capability model, so that extensions receive explicit host capabilities and remain isolated from internal host implementation.
21. As an extension author, I want a stable component contract and validation boundary, so that components can be developed independently from the desktop host.
22. As an end user, I want Desktop Tools to preserve the last visible frame and hit map during a renderer restart, so that a transient WebView2 failure does not immediately destroy the desktop surface.
23. As an end user, I want Explorer restart to trigger rediscovery and reattachment, so that desktop takeover can recover from Shell process lifecycle changes.
24. As an end user, I want the guardian to restore Explorer visibility after startup failure, normal exit, Ctrl+C, or main-process crash, so that desktop takeover always has a recovery path.
25. As an end user, I want Windows 10 22H2 and Windows 11 25H2 to expose the same Milestone 1 behavior, so that the supported Windows versions share one product contract.
26. As an application maintainer, I want frontend display code, bridge code, domain rules, Shell adapters, and platform code in separate ownership boundaries, so that each feature can evolve without leaking implementation details across layers.
27. As an application maintainer, I want serialized command, event, and snapshot contracts to be explicit, so that frontend and backend changes can be reviewed at the process boundary.
28. As an application maintainer, I want the prototype behavior preserved as an acceptance baseline, so that formal implementation work can be compared against verified desktop-layer behavior.

## Implementation Decisions

- The product is one Tauri desktop application with a React frontend, a Rust main process, and an independent Rust guardian binary.
- The frontend is organized by product capability: workspace surface, desktop items, fences, folder portals, components, settings, and system menu.
- The frontend bridge is the only native-call entry used by feature code. It owns command invocation, event subscription, request shaping, response decoding, and frontend contract types.
- Frontend state is split into rendered state, interaction state, and view-model conversion. Domain rules, Shell facts, and persistence decisions remain backend concerns.
- The backend command boundary validates serialized input, invokes a functional module, and publishes immutable snapshots or fact events.
- Workspace Domain owns virtual desktop layouts, fence membership, selection collections, focus containers, drag transactions, ordering, scrolling, and layout undo history.
- Shell Adapter owns desktop namespace enumeration, stable Shell item references, system images, Shell capabilities, native context menus, clipboard operations, OLE drag and drop, and change notifications.
- Shell Adapter runs through a dedicated STA Broker. Window-host threads own their HWND, WebView2 controller, hit-test registration, and OLE drop target lifetime.
- Host Runtime owns one HostContext per monitor. HostContext tracks physical work area, target DPI, effective rasterization scale, actual HWND state, WebView2 controller state, visibility, z-order, and the current interaction hit map.
- Host Runtime stages all hosts before desktop takeover. Explorer icon visibility changes after page readiness, icon probes, pixel probes, input probes, hit-map publication, guardian readiness, and command-channel readiness succeed.
- Display topology, work-area, DPI, or Explorer host changes trigger host reconciliation. Existing takeover state remains active across DPI-only layout changes; Explorer identity changes trigger a complete reattachment cycle.
- Persistence owns versioned JSON snapshots, atomic writes, manual backups, Schema migration, read validation, restore transactions, and layout archives.
- Component Runtime owns component manifests, registration, capability declarations, lifecycle, persisted component state, and host-facing events. The public Component SDK remains a later milestone surface.
- Recovery owns guardian registration, health milestones, renderer restart coordination, Explorer rediscovery, takeover rollback, and recovery status reporting.
- Windows platform adapters contain Win32, WebView2, monitor, DPI, Shell compatibility, virtual desktop, keyboard, menu, and OLE implementation details.
- Contracts are versioned at the serialized command, event, and snapshot boundary. Frontend DTOs and Rust DTOs use matching names and field semantics; transport changes require compatibility review.
- Shell facts and Desktop Tools layout facts remain separate. Layout stores stable project references and presentation state; Shell provides current namespace truth and operation results.
- Layout changes form explicit transactions. Failed, cancelled, or state-preserving operations leave no undo entry; successful user operations create one reversible layout unit.
- Shell operations that support compensation record the confirmed native result before creating a reversible operation entry. Permanent deletion remains outside the reversible operation set.
- The formal migration proceeds through Shell identity and STA Broker, Workspace Domain and snapshots, Host Runtime, full interaction and Shell operations, virtual desktop reconciliation, and component contracts.
- The prototype remains a frozen behavioral reference. Formal modules absorb verified mechanisms and keep prototype-specific probe controls and diagnostic shortcuts out of the product boundary.

## Testing Decisions

- Tests assert externally observable behavior at the highest available seam. The primary seam is the typed Tauri command/event boundary between the React bridge and Rust command handlers.
- Contract tests cover command validation, event payload shape, immutable snapshot publication, error mapping, and lifecycle event ordering.
- Workspace tests cover fence membership, selection and focus rules, drag transaction commit/cancel, ordering, virtual desktop layout isolation, snapshot restore, and undo timeline behavior.
- Shell tests cover stable identity reconciliation, rename/delete/property change handling, operation result mapping, menu capability selection, clipboard state, and OLE transfer outcomes through adapter-level fakes.
- Host tests cover monitor reconciliation, DPI changes, WebView2 bounds and rasterization configuration, hit-map publication, takeover gates, z-order, Explorer reattachment, and recovery transitions.
- Persistence tests cover atomic snapshot writes, schema migration, corrupted or newer backups, restore rollback, and layout archive retention.
- Recovery tests cover guardian handshake, startup timeout, normal shutdown, Ctrl+C, main-process crash, Explorer restart, renderer restart, and host adapter failure.
- Frontend tests cover view-model projection, interaction state transitions, command dispatch, event subscription, and accessible rendering states.
- Windows acceptance tests cover Windows 10 22H2 and Windows 11 25H2, single and multiple monitors, mixed DPI, lock/unlock, sleep/resume, virtual desktop switching, Win+D, Explorer restart, normal exit, and crash recovery.
- Prior art comes from the prototype's Rust unit tests, frontend self-checks, probe logs, Windows evidence matrices, and the existing ADR acceptance boundaries. Runtime screenshots and logs remain part of desktop-layer acceptance.
- Static verification includes Rust formatting, Rust tests, Clippy with warnings treated as errors, frontend type checking, frontend linting, frontend tests, build verification, and `git diff --check`.

## Out of Scope

- Third-party component distribution, public Component SDK packaging, capability marketplace, and component sandbox policy.
- Folder portal search, cross-directory batch workflows, and advanced Shell workflows beyond the Milestone 1 interaction contract.
- macOS and Linux desktop takeover, Shell integration, native menus, and platform acceptance.
- Replacing Windows taskbar, Start menu, application switching, or general window management.
- Reimplementing Windows Shell file-operation semantics inside React or the workspace domain.
- Prototype probe controls, diagnostic-only command channels, and prototype-specific static sample data in the formal product surface.
- Installer signing, update channels, release branding, and distribution infrastructure covered by later milestones.

## Further Notes

The load-bearing assumption is that Tauri 2, WebView2, the independent Rust guardian, and the verified native Windowed host remain the product foundation. A change to that foundation moves the host, bridge, recovery, and platform boundaries together.

The architecture preserves the approved domain vocabulary: desktop content workspace, desktop takeover, transparent desktop host, interaction hit region, interaction hit map, Shell operation proxy, own layout, virtual desktop layout, fence, folder portal, component, and component SDK.

This specification is ready for implementation work under the `ready-for-agent` triage label. Each implementation phase should leave the application structurally usable and keep the prototype available for behavioral comparison.
