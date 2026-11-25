/**
 * 插件挂载会话：每个实例在任一时刻只有一个当前会话。
 * 旧视图的清理只能结束自己创建的会话，不能释放换位后新视图的会话。
 */
export interface PluginMountSessions
{
    sequence: number; // 单调递增的会话号。
    current: Map<string, number>; // 实例 → 当前会话。
}

/** 新建一份挂载会话表。 */
export const createPluginMountSessions = (): PluginMountSessions => ({ sequence: 0, current: new Map() });

/** 为实例登记新会话，旧会话自动失效。 */
export const beginPluginMountSession = (sessions: PluginMountSessions, instanceId: string): number =>
{
    const session = ++sessions.sequence;
    sessions.current.set(instanceId, session);
    return session;
};

/** 当前会话是否仍属于给定会话号。 */
export const ownsPluginMountSession = (sessions: PluginMountSessions, instanceId: string, session: number): boolean =>
    sessions.current.get(instanceId) === session;

/** 只有会话持有者才能结束；旧会话的迟到清理返回 false 且不影响新会话。 */
export const endPluginMountSession = (sessions: PluginMountSessions, instanceId: string, session: number): boolean =>
{
    if (!ownsPluginMountSession(sessions, instanceId, session)) return false;
    sessions.current.delete(instanceId);
    return true;
};
