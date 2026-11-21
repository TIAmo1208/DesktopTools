/**
 * dsh-desktop-bridge — DSH Host 只读桥插件。
 *
 * 只读：在 127.0.0.1 上暴露快照服务，供 DesktopTools 的 Rust 适配层读取余额、
 * 会话、job 与待决摘要；不注册审批应答者、不代批、不调用任何 DSH 写方法。
 *
 * 配对凭据只出现在 DSH 同源配对页（/dsh-bridge/pairing）里，不写日志、不在
 * 47832 侧暴露；桥身份 bridgeId 持久化到 ctx.credentials，每次启动的
 * bridgeInstanceId 只用于作废旧快照。
 */

import { createHash, randomBytes, randomUUID, timingSafeEqual } from 'node:crypto'
import { createServer } from 'node:http'
import { readFileSync } from 'node:fs'

/** 协议版本；DesktopTools 按它判断兼容性。 */
export const PROTOCOL_VERSION = 1

/** 插件名（同时也是 cordis 行 id 与凭据作用域）。 */
export const name = 'dsh-desktop-bridge'

/** 必需服务：凭据存储用于保存配对码与令牌摘要；未声明时访问 ctx.credentials 会被 cordis 拒绝。 */
export const inject = ['credentials']

/** 配对页的同源路径（挂在 DSH Web 服务上，不是 47832）。 */
export const PAIRING_PATH = '/dsh-bridge/pairing'

/** 配对页读取的只读状态路径（同源、owner 认证、绝不返回配对码）。 */
export const STATUS_PATH = '/dsh-bridge/status'

/** 设置页内嵌配对面板的数据路径（同源、owner 认证、只返回配对码与状态）。 */
export const PAIRING_JSON_PATH = '/dsh-bridge/pairing.json'

/** 默认监听端口；只允许 127.0.0.1。 */
const DEFAULT_PORT = 47832

/** 配对码默认有效期：10 分钟。 */
const DEFAULT_PAIRING_TTL_MS = 10 * 60 * 1000

/** 过期检查间隔：15 秒。 */
const DEFAULT_ROTATE_CHECK_MS = 15 * 1000

/** 余额缓存：5 秒。 */
const DEFAULT_BALANCE_CACHE_MS = 5000

/** 平台余额调用超时：3 秒。 */
const DEFAULT_BALANCE_TIMEOUT_MS = 3000

/** 配对请求体上限。 */
const MAX_BODY_BYTES = 4096

/** 配对页表单体上限。 */
const MAX_FORM_BYTES = 2048

/** Rust 合同的展示上限：超限整份拒绝，所以生产者先裁剪。 */
const MAX_SESSIONS = 64
const MAX_JOBS = 32
const MAX_PENDING = 32
const MAX_WALLETS = 16
const MAX_TEXT_CHARS = 256
const MAX_ID_CHARS = 128

/** job 枚举只查活跃/最近的会话，避免全量扫描。 */
const JOB_SCAN_SESSIONS = 16

/** 会话/job 缓存的有界保留量（超出后回收已结束对象）。 */
const SESSION_KEEP = 256
const JOB_KEEP = 128

/** 配对码随机字节数（96 bit，base32 后 20 字符）。 */
const CODE_BYTES = 12

/** 配对 token 随机字节数（256 bit）。 */
const TOKEN_BYTES = 32

/** 配对尝试限流：60 秒内最多 5 次。 */
const PAIR_WINDOW_MS = 60 * 1000
const PAIR_MAX_ATTEMPTS = 5

/** 凭据记录的 key（`<scope>/<id>`，运行时只是字符串）。 */
const IDENTITY_CREDENTIAL = `${name}/identity`
const CODE_CREDENTIAL = `${name}/pairing-code`
const TOKEN_CREDENTIAL = `${name}/pairing-token`

/** 从包清单读取版本，用作账户查询的 client version。 */
const VERSION = (() => {
  try {
    const parsed = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'))
    return typeof parsed.version === 'string' ? parsed.version : '0.0.0'
  } catch {
    return '0.0.0'
  }
})()

/** base32 字母表（RFC 4648，无填充）。 */
const BASE32 = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'

/**
 * 把字节编码为 base32。
 * @param {Buffer} buffer 随机字节。
 * @returns {string} 大写 base32 文本。
 */
function base32(buffer) {
  let bits = 0
  let value = 0
  let out = ''
  for (const byte of buffer) {
    value = (value << 8) | byte
    bits += 8
    while (bits >= 5) {
      out += BASE32[(value >>> (bits - 5)) & 31]
      bits -= 5
    }
  }
  if (bits > 0) out += BASE32[(value << (5 - bits)) & 31]
  return out
}

/**
 * 生成用户可复制的配对码（5 组 × 4 字符）。
 * @returns {string} 形如 `ABCDE-FGHIJ-KLMNO-PQRST`。
 */
function makePairingCode() {
  const raw = base32(randomBytes(CODE_BYTES))
  return [raw.slice(0, 5), raw.slice(5, 10), raw.slice(10, 15), raw.slice(15, 20)].join('-')
}

/**
 * 归一化用户输入的配对码。
 * @param {unknown} value 任意输入。
 * @returns {string} 去掉分隔符并大写后的文本；非法输入为空串。
 */
function normalizePairingCode(value) {
  if (typeof value !== 'string') return ''
  const normalized = value.toUpperCase().replace(/[^A-Z2-7]/g, '')
  return normalized.length === 20 ? normalized : ''
}

/**
 * 计算 SHA-256 十六进制摘要。
 * @param {string} text 输入文本。
 * @returns {string} 64 位小写十六进制。
 */
function sha256Hex(text) {
  return createHash('sha256').update(text, 'utf8').digest('hex')
}

/**
 * 常量时间比较两个十六进制摘要。
 * @param {unknown} left 待比较值。
 * @param {unknown} right 待比较值。
 * @returns {boolean} 是否一致。
 */
function safeEqualHex(left, right) {
  if (typeof left !== 'string' || typeof right !== 'string') return false
  if (!/^[0-9a-f]{64}$/i.test(left) || !/^[0-9a-f]{64}$/i.test(right)) return false
  const a = Buffer.from(left, 'hex')
  const b = Buffer.from(right, 'hex')
  if (a.length !== b.length) return false
  return timingSafeEqual(a, b)
}

/**
 * 转义 HTML 文本。
 * @param {unknown} value 任意值。
 * @returns {string} 转义后的文本。
 */
function escapeHtml(value) {
  return String(value ?? '')
    .replaceAll('&', '&amp;')
    .replaceAll('<', '&lt;')
    .replaceAll('>', '&gt;')
    .replaceAll('"', '&quot;')
    .replaceAll("'", '&#39;')
}

/**
 * 去掉 Rust `char::is_control` 会拒绝的控制字符（C0、DEL、C1）。
 * @param {unknown} value 原值。
 * @returns {string} 清理后的文本。
 */
function stripControl(value) {
  return String(value ?? '').replace(/[\u0000-\u001f\u007f-\u009f]/g, ' ')
}

/**
 * 按 Rust 合同裁剪文本：去控制字符、非空、按字符数截断到 256。
 * @param {unknown} value 原值。
 * @param {string} fallback 空值兜底。
 * @returns {string} 合同内文本。
 */
function clampText(value, fallback) {
  const primary = stripControl(value).trim()
  const secondary = stripControl(fallback).trim()
  const text = primary.length > 0 ? primary : secondary.length > 0 ? secondary : '?'
  return [...text].slice(0, MAX_TEXT_CHARS).join('')
}

/**
 * 探测宿主服务：cordis 对未声明 inject 的 `ctx.<name>` 属性访问会抛错，
 * 用 `ctx.get(name)` 才能在不声明依赖的前提下判断服务是否存在。
 * @param {any} ctx 插件上下文。
 * @param {string} name 服务名。
 * @returns {any} 服务实例或 undefined。
 */
function readService(ctx, name) {
  try {
    return typeof ctx?.get === 'function' ? ctx.get(name) : undefined
  } catch {
    return undefined
  }
}

/**
 * 判断 id 是否满足 Rust 合同（非空、≤128 字符、无控制字符）。
 * @param {unknown} value 原值。
 * @returns {boolean} 是否可保留。
 */
function validId(value) {
  const text = String(value ?? '')
  return text.length > 0 && text.length <= MAX_ID_CHARS && !/[\u0000-\u001f\u007f-\u009f]/.test(text)
}

/**
 * 把钱包映射成 Rust 合同接受的形式：currency ≤16、amount 为纯数字文本且 ≤32。
 * @param {unknown} value getBalance 的 value。
 * @returns {Array<{ currency: string, amount: string }>} 合法钱包，最多 16 条。
 */
function sanitizeWallets(value) {
  if (!Array.isArray(value)) return []
  const wallets = []
  for (const wallet of value) {
    if (wallets.length >= MAX_WALLETS) break
    const currency = clampText(wallet?.currency, '?').slice(0, 16)
    const amount = String(wallet?.balance ?? '')
    if (currency.length === 0 || !/^-?\d+(?:\.\d+)?$/.test(amount) || amount.length > 32) continue
    wallets.push({ currency, amount })
  }
  return wallets
}

/**
 * 校验并归一化插件配置。
 * @param {unknown} raw cordis 行 config。
 * @returns {{ port: number, pairingCodeTtlMs: number, rotateCheckMs: number, balanceCacheMs: number, balanceTimeoutMs: number }} 有效配置。
 */
function normalizeConfig(raw) {
  const config = raw !== null && typeof raw === 'object' ? raw : {}
  const port = Number.isInteger(config.port) && config.port >= 0 && config.port <= 65535
    ? config.port
    : DEFAULT_PORT
  const ttl = Number.isFinite(config.pairingCodeTtlMs) && config.pairingCodeTtlMs > 0
    ? Math.min(config.pairingCodeTtlMs, 24 * 60 * 60 * 1000)
    : DEFAULT_PAIRING_TTL_MS
  const rotate = Number.isFinite(config.rotateCheckMs) && config.rotateCheckMs > 0
    ? Math.max(config.rotateCheckMs, 1000)
    : DEFAULT_ROTATE_CHECK_MS
  const cache = Number.isFinite(config.balanceCacheMs) && config.balanceCacheMs >= 0
    ? config.balanceCacheMs
    : DEFAULT_BALANCE_CACHE_MS
  const timeout = Number.isFinite(config.balanceTimeoutMs) && config.balanceTimeoutMs > 0
    ? config.balanceTimeoutMs
    : DEFAULT_BALANCE_TIMEOUT_MS
  return { port, pairingCodeTtlMs: ttl, rotateCheckMs: rotate, balanceCacheMs: cache, balanceTimeoutMs: timeout }
}

/**
 * 在超时后放弃一个 Promise（避免平台调用挂住快照）。
 * @param {Promise<unknown>} promise 上游 Promise。
 * @param {number} ms 超时毫秒。
 * @returns {Promise<unknown>} 结果或超时拒绝。
 */
function withTimeout(promise, ms) {
  let timer = null
  const timeout = new Promise((_, reject) => {
    timer = setTimeout(() => reject(Object.assign(new Error('timeout'), { code: 'timeout' })), ms)
    timer.unref?.()
  })
  return Promise.race([
    Promise.resolve(promise).finally(() => {
      if (timer !== null) clearTimeout(timer)
    }),
    timeout,
  ])
}

/**
 * 从会话头推导一个显示名（无标题时）。
 * @param {any} session DSH Session。
 * @returns {string} 显示名。
 */
function fallbackTitle(session) {
  const cwd = session?.header?.cwd
  if (typeof cwd === 'string' && cwd.length > 0) {
    const parts = cwd.split(/[\\/]+/).filter(Boolean)
    if (parts.length > 0) return parts[parts.length - 1]
  }
  const id = String(session?.header?.id ?? session?.id ?? '')
  return id.length > 8 ? `Session ${id.slice(0, 8)}` : 'Session'
}

/**
 * 创建桥实例。
 * @param {{ ctx: any, config?: unknown, logger?: any, now?: () => number }} options 依赖。
 * @returns {any} 桥实例（含测试访问器）。
 */
export function createBridge(options) {
  const ctx = options.ctx
  const logger = options.logger ?? ctx?.logger ?? {}
  const config = normalizeConfig(options.config)
  const now = typeof options.now === 'function' ? options.now : () => Date.now()

  const bridgeInstanceId = randomUUID()
  const since = new Date(now()).toISOString()
  const startedAt = now()
  /** 配对页 CSRF 令牌；只随页面下发，不写日志。 */
  const pageCsrfToken = randomBytes(18).toString('base64url')

  /** 稳定的桥身份（首次生成后持久化）。 */
  let bridgeId = null

  /** 会话：id -> { session }；事件先到、快照时再补。 */
  const sessions = new Map()
  /** 会话创建时间：id -> epoch ms，用于 job 扫描的“最近”排序。 */
  const sessionCreatedAt = new Map()
  /** 标题：id -> title。 */
  const titles = new Map()
  /** agent 状态：sessionId -> 'idle'|'running'。 */
  const agentStatus = new Map()
  /** 待决：id -> { id, kind, sessionId, since }。 */
  const pending = new Map()
  /** 事件兜底 job：jobId -> { id, label, status }。 */
  const eventJobs = new Map()

  let revision = 0
  let lastSnapshotDigest = null
  let lastBalanceSignature = null
  let pairingCode = null
  let pairingCodeExpiresAt = 0
  let rotateTimer = null
  let server = null
  let port = 0
  let stopped = false
  /** 余额缓存：值 + 采样时间 + 是否因平台错误而陈旧。 */
  let balanceCache = { at: Number.NEGATIVE_INFINITY, value: null, stale: false }
  /** 缓存 miss 时共享的在途余额查询；并发快照不会重复打平台。 */
  let balanceInFlight = null
  /** 配对处理串行队列：同一时刻只允许一个读码→校验→消费→签发。 */
  let pairingChain = Promise.resolve()

  /**
   * 取得凭据服务；缺失即抛错（不允许静默回退到内存）。
   * @returns {any} ctx.credentials。
   */
  function requireCredentials() {
    const credentials = ctx?.credentials
    if (credentials === null || credentials === undefined
      || typeof credentials.readRecord !== 'function'
      || typeof credentials.modifyRecord !== 'function'
      || typeof credentials.deleteRecord !== 'function') {
      throw Object.assign(new Error('credentials-unavailable'), { code: 'credentials-unavailable' })
    }
    return credentials
  }

  /**
   * 读取一条凭据记录；失败向上抛。
   * @param {string} key `<scope>/<id>`。
   * @returns {Promise<any>} 记录或 undefined。
   */
  async function readCredential(key) {
    return requireCredentials().readRecord(key)
  }

  /**
   * 写入一条 grant 记录；失败向上抛。
   * @param {string} key `<scope>/<id>`。
   * @param {any} payload 记录内容。
   * @returns {Promise<void>} 完成。
   */
  async function writeCredential(key, payload) {
    await requireCredentials().modifyRecord(key, async () => ({ kind: 'grant', payload }))
  }

  /**
   * 删除一条凭据记录；失败向上抛。
   * @param {string} key `<scope>/<id>`。
   * @returns {Promise<void>} 完成。
   */
  async function deleteCredential(key) {
    await requireCredentials().deleteRecord(key)
  }

  /**
   * 首次启动生成 bridgeId 并持久化；已有则沿用。
   * @returns {Promise<void>} 完成；凭据不可用时抛错。
   */
  async function ensureBridgeId() {
    const existing = await readCredential(IDENTITY_CREDENTIAL)
    const stored = existing?.payload?.bridgeId
    if (typeof stored === 'string' && stored.length >= 16 && stored.length <= 128) {
      bridgeId = stored
      return
    }
    const fresh = randomUUID()
    await writeCredential(IDENTITY_CREDENTIAL, { bridgeId: fresh, createdAt: now() })
    bridgeId = fresh
    if (existing !== undefined) logger.warn?.(`[${name}] identity record was malformed; a new bridgeId was persisted`)
  }

  /**
   * 生成/轮换配对码，并把哈希写入凭据。
   * @param {string} reason 触发原因（只用于日志，不带码）。
   * @returns {Promise<void>} 完成。
   */
  async function rotatePairingCode(reason) {
    if (stopped) return
    pairingCode = makePairingCode()
    pairingCodeExpiresAt = now() + config.pairingCodeTtlMs
    await writeCredential(CODE_CREDENTIAL, {
      // 存归一化后的哈希：用户输入可以带分隔符，校验前会去掉。
      hash: sha256Hex(normalizePairingCode(pairingCode)),
      createdAt: now(),
      expiresAt: pairingCodeExpiresAt,
    })
    const seconds = Math.max(1, Math.round((pairingCodeExpiresAt - now()) / 1000))
    logger.info?.(`[${name}] pairing ${reason === 'start' ? 'ready' : 'refreshed'}; open ${PAIRING_PATH} in the DSH window (code expires in ${seconds}s)`)
  }

  /**
   * 检查配对码是否过期，过期则换新（已配对则不再展示码）。
   * @returns {Promise<void>} 完成。
   */
  function refreshPairingCode() {
    return enqueuePairing(async () => {
      if (stopped) return
      if (pairingCode === null) return
      if (now() < pairingCodeExpiresAt) return
      pairingCode = null
      pairingCodeExpiresAt = 0
      await deleteCredential(CODE_CREDENTIAL)
      logger.info?.(`[${name}] pairing code expired; open ${PAIRING_PATH} to get a new one`)
    })
  }

  /**
   * 串行执行一次配对处理。
   * @param {() => Promise<any>} task 处理函数。
   * @returns {Promise<any>} 处理结果。
   */
  function enqueuePairing(task) {
    const run = pairingChain.then(task, task)
    pairingChain = run.then(() => undefined, () => undefined)
    return run
  }

  /**
   * 记录一条待决项。
   * @param {string} id 审批 id / tool call id。
   * @param {'approval'|'question'|'plan'} kind 类型。
   * @param {string} sessionId 会话 id。
   * @returns {void}
   */
  function markPending(id, kind, sessionId) {
    if (!id || !sessionId) return
    pending.set(id, { id, kind, sessionId, since: new Date(now()).toISOString() })
  }

  /**
   * 维护会话登记。
   * @param {string} sessionId 会话 id。
   * @param {any} session Session。
   * @returns {void}
   */
  function rememberSession(sessionId, session) {
    if (!sessionId) return
    const existing = sessions.get(sessionId) ?? {}
    sessions.set(sessionId, { session: session ?? existing.session })
    const createdAt = session?.header?.createdAt
    if (typeof createdAt === 'number') sessionCreatedAt.set(sessionId, createdAt)
    pruneSessions()
  }

  /**
   * 该会话是否还有活待决。
   * @param {string} sessionId 会话 id。
   * @returns {boolean} 是否存在待决。
   */
  function hasLivePending(sessionId) {
    for (const item of pending.values()) {
      if (item.sessionId === sessionId) return true
    }
    return false
  }

  /**
   * 有界回收已结束会话：优先保留有活待决或 running 的会话。
   * @returns {void}
   */
  function pruneSessions() {
    if (sessions.size <= SESSION_KEEP) return
    for (const [id] of sessions) {
      if (sessions.size <= SESSION_KEEP) break
      if (hasLivePending(id) || agentStatus.get(id) === 'running') continue
      sessions.delete(id)
      titles.delete(id)
    }
  }

  /**
   * 有界回收已结束 job：保留 running，按注册顺序丢弃旧的已结束记录。
   * @returns {void}
   */
  function pruneJobs() {
    if (eventJobs.size <= JOB_KEEP) return
    for (const [id, job] of eventJobs) {
      if (eventJobs.size <= JOB_KEEP) break
      if (job.status !== 'running') eventJobs.delete(id)
    }
  }

  /**
   * 处理一条会话事件：只读维护 revision/待决/job 摘要。
   * @param {any} session DSH Session。
   * @param {any} event 会话事件。
   * @returns {void}
   */
  function handleSessionEvent(session, event) {
    try {
      const sessionId = session?.header?.id ?? session?.id
      if (!sessionId) return
      revision += 1
      rememberSession(sessionId, session)
      const type = event?.type
      const data = event?.data
      if (type === 'session/title') {
        if (typeof data?.title === 'string' && data.title.length > 0) titles.set(sessionId, data.title)
        return
      }
      if (type === 'approval/asked') {
        markPending(String(data?.id ?? ''), 'approval', sessionId)
        return
      }
      if (type === 'approval/decided') {
        pending.delete(String(data?.id ?? ''))
        return
      }
      if (type === 'tool/call') {
        if (data?.name === 'ask_user_question') markPending(String(data.callId ?? ''), 'question', sessionId)
        else if (data?.name === 'exit_plan_mode') markPending(String(data.callId ?? ''), 'plan', sessionId)
        return
      }
      if (type === 'tool/result') {
        const callId = data?.message?.callId
        if (callId !== undefined) pending.delete(String(callId))
        return
      }
      if (type === 'job/registered') {
        const id = String(data?.jobId ?? '')
        if (id) eventJobs.set(id, { id, label: String(data?.label ?? ''), status: 'running' })
        pruneJobs()
        return
      }
      if (type === 'job/settled') {
        const id = String(data?.jobId ?? '')
        if (id) eventJobs.set(id, { id, label: String(data?.label ?? ''), status: mapJobStatus(String(data?.status ?? '')) })
        pruneJobs()
      }
    } catch (error) {
      logger.warn?.(`[${name}] session/event handler failed: ${String(error?.message ?? error)}`)
    }
  }

  /**
   * 处理 agent 状态变化。
   * @param {{ agent?: any, status?: string }} payload 事件载荷。
   * @returns {void}
   */
  function handleAgentStatus(payload) {
    try {
      const sessionId = payload?.agent?.session?.id ?? payload?.agent?.id
      if (!sessionId) return
      revision += 1
      agentStatus.set(String(sessionId), String(payload?.status ?? 'idle'))
      rememberSession(String(sessionId), payload?.agent?.session)
    } catch (error) {
      logger.warn?.(`[${name}] agent/status handler failed: ${String(error?.message ?? error)}`)
    }
  }

  /**
   * 把 DSH job 状态映射为快照状态。
   * @param {string} status JobStatus。
   * @returns {'running'|'done'|'failed'} 快照状态。
   */
  function mapJobStatus(status) {
    if (status === 'running' || status === 'stopping') return 'running'
    if (status === 'completed') return 'done'
    return 'failed'
  }

  /**
   * 真正查询平台余额（带 3 秒超时）。
   * @returns {Promise<{ value: any, stale: boolean }>} 余额视图与陈旧标记。
   */
  async function fetchBalance() {
    const account = readService(ctx, 'deepseekAccount')
    if (account === undefined || account === null) return { value: { status: 'error', wallets: [] }, stale: true }
    const client = {
      version: VERSION,
      locale: 'zh-CN',
      timezoneOffsetSeconds: -new Date().getTimezoneOffset() * 60,
    }
    // 平台调用总预算 3 秒：getState 与 getBalance 共享同一个 deadline。
    const deadline = now() + config.balanceTimeoutMs
    const remaining = () => Math.max(1, deadline - now())
    try {
      const state = typeof account.getState === 'function'
        ? await withTimeout(account.getState(), remaining())
        : undefined
      if (state === undefined || state?.status === 'signed-out') {
        return { value: { status: 'logged-out', wallets: [] }, stale: false }
      }
      const detail = await withTimeout(account.getBalance(client), remaining())
      if (detail === null || detail === undefined) {
        return { value: { status: 'unauthorized', wallets: [] }, stale: false }
      }
      if (detail.status === 'ready') {
        return { value: { status: 'ready', wallets: sanitizeWallets(detail.value) }, stale: false }
      }
      return { value: { status: 'error', wallets: [] }, stale: true }
    } catch (error) {
      const code = String(error?.code ?? error?.name ?? '')
      if (code === 'expired' || code === 'unauthorized' || code === 'AccountUnauthorizedError') {
        return { value: { status: 'unauthorized', wallets: [] }, stale: false }
      }
      return { value: { status: 'error', wallets: [] }, stale: true }
    }
  }

  /**
   * 读取余额（5 秒缓存；并发 miss 共享同一在途请求；平台慢只影响余额状态）。
   * @returns {Promise<{ status: string, wallets: Array<{ currency: string, amount: string }> }>} 余额视图。
   */
  function readBalance() {
    if (balanceCache.value !== null && now() - balanceCache.at < config.balanceCacheMs) {
      return Promise.resolve(balanceCache.value)
    }
    if (balanceInFlight !== null) return balanceInFlight
    balanceInFlight = (async () => {
      try {
        const fresh = await fetchBalance()
        const signature = JSON.stringify(fresh.value)
        if (signature !== lastBalanceSignature) {
          lastBalanceSignature = signature
          revision += 1
        }
        balanceCache = { at: now(), value: fresh.value, stale: fresh.stale }
        return fresh.value
      } finally {
        balanceInFlight = null
      }
    })()
    return balanceInFlight
  }

  /**
   * 聚合会话列表（含 agent 状态与待决摘要）。
   * @returns {Promise<Array<any>>} 会话快照。
   */
  async function readSessions() {
    const known = new Map()
    const sessionsService = readService(ctx, 'sessions')
    const liveSessions = typeof sessionsService?.list === 'function' ? sessionsService.list() : []
    for (const session of liveSessions) {
      const id = session?.header?.id ?? session?.id
      if (id) {
        known.set(String(id), { session, agent: null })
        if (typeof session?.header?.createdAt === 'number') sessionCreatedAt.set(String(id), session.header.createdAt)
      }
    }
    const agentsService = readService(ctx, 'agents')
    const agents = typeof agentsService?.list === 'function' ? agentsService.list() : []
    for (const agent of agents) {
      const id = agent?.session?.id ?? agent?.id
      if (!id) continue
      const existing = known.get(String(id)) ?? {}
      known.set(String(id), { ...existing, agent, session: agent?.session ?? existing.session })
      if (typeof agent?.session?.header?.createdAt === 'number') sessionCreatedAt.set(String(id), agent.session.header.createdAt)
    }
    for (const [id, entry] of sessions) {
      const existing = known.get(id) ?? {}
      known.set(id, { ...existing, ...entry })
    }
    const rows = []
    for (const [id, entry] of known) {
      if (!validId(id)) continue
      const status = agentStatus.get(id) ?? entry?.agent?.status ?? 'idle'
      const pendingKinds = [...pending.values()].filter(item => item.sessionId === id).map(item => item.kind)
      const pendingKind = pendingKinds.includes('approval')
        ? 'approval'
        : pendingKinds.includes('question')
          ? 'question'
          : pendingKinds.includes('plan')
            ? 'plan'
            : 'none'
      const fallback = fallbackTitle(entry?.session)
      rows.push({
        id,
        title: clampText(titles.get(id) ?? fallback, fallback),
        status: status === 'running' ? 'running' : pendingKind !== 'none' ? 'waiting' : 'idle',
        pending: pendingKind,
      })
    }
    // 优先保留活待决，其次 running；其余按 id 稳定排序，最后按合同截到 64。
    const rank = row => (row.pending !== 'none' ? 0 : row.status === 'running' ? 1 : 2)
    rows.sort((a, b) => rank(a) - rank(b) || a.id.localeCompare(b.id))
    return rows.slice(0, MAX_SESSIONS)
  }

  /**
   * 聚合 job 列表：事件兜底叠加 ctx.jobs 的实时行；只扫有限会话。
   * @param {string[]} sessionIds 需要查询的会话 id（已由调用方限制数量）。
   * @returns {Promise<{ rows: Array<any>, failed: boolean }>} job 快照与读取失败标记。
   */
  async function readJobs(sessionIds) {
    const map = new Map()
    let failed = false
    for (const [id, job] of eventJobs) map.set(id, { ...job })
    const jobsService = readService(ctx, 'jobs')
    if (jobsService?.list) {
      const collect = rows => {
        if (!Array.isArray(rows)) return
        for (const row of rows) {
          if (!validId(row?.id)) continue
          map.set(String(row.id), {
            id: String(row.id),
            label: String(row.label ?? ''),
            status: mapJobStatus(String(row.status ?? '')),
          })
        }
      }
      // 单个会话读取失败不应丢掉其它 job，但要如实标记“不可知/陈旧”。
      const reads = [() => jobsService.list()]
      for (const sessionId of sessionIds) reads.push(() => jobsService.list(sessionId))
      for (const read of reads) {
        try {
          collect(await read())
        } catch (error) {
          failed = true
          logger.warn?.(`[${name}] jobs.list failed: ${String(error?.message ?? error)}`)
        }
      }
    } else {
      // 没有 job 服务时无法断言“没有任务”，按不可知处理。
      failed = true
    }
    const rows = [...map.values()]
      .filter(job => validId(job.id))
      .map(job => ({ id: job.id, label: clampText(job.label, job.id), status: job.status }))
      .sort((a, b) => (a.status === 'running' ? 0 : 1) - (b.status === 'running' ? 0 : 1) || a.id.localeCompare(b.id))
      .slice(0, MAX_JOBS)
    return { rows, failed }
  }

  /**
   * 构造快照；revision 覆盖余额、待决、会话与 job 的全部变化。
   * @returns {Promise<any>} 快照 JSON。
   */
  async function buildSnapshot() {
    let stale = false
    const balance = await readBalance()
    let sessionRows = []
    let jobRows = []
    try {
      sessionRows = await readSessions()
    } catch (error) {
      stale = true
      logger.warn?.(`[${name}] session aggregation failed: ${String(error?.message ?? error)}`)
    }
    try {
      // 只扫活跃/最近的有限会话集合，避免每次快照全量扫描 job。
      const jobSessionIds = [...sessionRows]
        .sort((a, b) => {
          const rank = row => (row.pending !== 'none' ? 0 : row.status === 'running' ? 1 : 2)
          return rank(a) - rank(b)
            || (sessionCreatedAt.get(b.id) ?? 0) - (sessionCreatedAt.get(a.id) ?? 0)
            || a.id.localeCompare(b.id)
        })
        .slice(0, JOB_SCAN_SESSIONS)
        .map(row => row.id)
      const jobResult = await readJobs(jobSessionIds)
      jobRows = jobResult.rows
      if (jobResult.failed) stale = true
    } catch (error) {
      stale = true
      logger.warn?.(`[${name}] job aggregation failed: ${String(error?.message ?? error)}`)
    }
    if (balanceCache.stale) stale = true
    const pendingRows = [...pending.values()]
      .filter(item => validId(item.id) && validId(item.sessionId))
      .map(item => ({ id: item.id, kind: item.kind, sessionId: item.sessionId, since: item.since }))
      .sort((a, b) => a.since.localeCompare(b.since))
      // 优先保留活的待决（全部都是未决），按时间保留最近 32 条。
      .slice(-MAX_PENDING)
    const digest = sha256Hex(JSON.stringify({ stale, balance, sessions: sessionRows, jobs: jobRows, pending: pendingRows }))
    if (digest !== lastSnapshotDigest) {
      lastSnapshotDigest = digest
      revision += 1
    }
    return {
      protocolVersion: PROTOCOL_VERSION,
      bridgeId,
      bridgeInstanceId,
      revision,
      fetchedAt: new Date(now()).toISOString(),
      stale,
      balance,
      sessions: sessionRows,
      jobs: jobRows,
      pending: pendingRows,
    }
  }

  /**
   * 写 JSON 响应。
   * @param {any} res 响应。
   * @param {number} status 状态码。
   * @param {any} body 响应体。
   * @param {Record<string,string>} [headers] 额外头。
   * @returns {void}
   */
  function respond(res, status, body, headers = {}) {
    const payload = JSON.stringify(body)
    res.writeHead(status, {
      'content-type': 'application/json; charset=utf-8',
      'cache-control': 'no-store',
      'content-length': Buffer.byteLength(payload),
      ...headers,
    })
    res.end(payload)
  }

  /**
   * 读取 JSON 请求体。
   * @param {any} req 请求。
   * @returns {Promise<any>} 解析后的对象；失败抛错。
   */
  /** 配对尝试限流表：ip -> { count, resetAt }。 */
  const pairAttempts = new Map()

  /**
   * 判断该来源是否超过配对尝试上限。
   * @param {string} ip 来源地址。
   * @returns {boolean} 是否限流。
   */
  function pairRateLimited(ip) {
    const current = pairAttempts.get(ip)
    if (current === undefined || now() >= current.resetAt) {
      pairAttempts.set(ip, { count: 1, resetAt: now() + PAIR_WINDOW_MS })
      return false
    }
    current.count += 1
    return current.count > PAIR_MAX_ATTEMPTS
  }

  /**
   * 校验 Bearer token。
   * @param {any} req 请求。
   * @returns {Promise<boolean>} 是否通过。
   */
  async function authorized(req) {
    const header = req.headers.authorization
    if (typeof header !== 'string') return false
    const match = /^Bearer ([A-Za-z0-9_-]{16,})$/.exec(header.trim())
    if (match === null) return false
    const record = await readCredential(TOKEN_CREDENTIAL)
    const hash = record?.payload?.hash
    return safeEqualHex(sha256Hex(match[1]), hash)
  }

  /**
   * 处理一次配对请求：读码→校验→消费→签发→落盘，串行且失败明确。
   * @param {any} body 请求体。
   * @returns {Promise<{ status: number, body: any }>} 响应。
   */
  async function handlePair(body) {
    return enqueuePairing(async () => {
      const record = await readCredential(CODE_CREDENTIAL)
      if (record === undefined || record === null || record?.payload?.hash === undefined) {
        return { status: 401, body: { ok: false, error: 'pairing-code-expired' } }
      }
      if (now() >= Number(record.payload.expiresAt ?? 0)) {
        await deleteCredential(CODE_CREDENTIAL)
        pairingCode = null
        pairingCodeExpiresAt = 0
        logger.info?.(`[${name}] pairing code expired; open ${PAIRING_PATH} to get a new one`)
        return { status: 401, body: { ok: false, error: 'pairing-code-expired' } }
      }
      const code = normalizePairingCode(body?.code)
      if (code === '' || !safeEqualHex(sha256Hex(code), record.payload.hash)) {
        return { status: 401, body: { ok: false, error: 'invalid-pairing-code' } }
      }
      const token = randomBytes(TOKEN_BYTES).toString('base64url')
      // 先消费码，再签发 token：签发失败时不会留下“码还在、token 已发”的分叉。
      await deleteCredential(CODE_CREDENTIAL)
      pairingCode = null
      pairingCodeExpiresAt = 0
      await writeCredential(TOKEN_CREDENTIAL, { hash: sha256Hex(token), pairedAt: now() })
      return { status: 200, body: { ok: true, token } }
    })
  }

  /**
   * 处理一次 HTTP 请求。
   * @param {any} req 请求。
   * @param {any} res 响应。
   * @returns {Promise<void>} 完成。
   */
  async function handleRequest(req, res) {
    const expectedHost = `127.0.0.1:${port}`
    if (req.headers.host !== expectedHost) {
      return respond(res, 421, { ok: false, error: 'misdirected-request' })
    }
    const url = new URL(req.url ?? '/', 'http://127.0.0.1')
    const path = url.pathname
    if (path === '/health') {
      if (req.method !== 'GET') return respond(res, 405, { ok: false, error: 'method-not-allowed' }, { allow: 'GET' })
      return respond(res, 200, { ok: true, protocolVersion: PROTOCOL_VERSION, bridgeId, bridgeInstanceId, since })
    }
    if (path === '/pair') {
      if (req.method !== 'POST') return respond(res, 405, { ok: false, error: 'method-not-allowed' }, { allow: 'POST' })
      const contentType = String(req.headers['content-type'] ?? '').toLowerCase()
      if (!/^application\/json(?:\s*;|$)/.test(contentType)) {
        return respond(res, 415, { ok: false, error: 'unsupported-media-type' })
      }
      const ip = String(req.socket?.remoteAddress ?? '')
      if (pairRateLimited(ip)) return respond(res, 429, { ok: false, error: 'too-many-attempts' })
      let body
      try {
        body = await readJsonBody(req)
      } catch (error) {
        return respond(res, error?.statusCode ?? 400, { ok: false, error: error?.message ?? 'invalid-json' })
      }
      try {
        const result = await handlePair(body)
        return respond(res, result.status, result.body)
      } catch (error) {
        logger.warn?.(`[${name}] pairing failed: ${String(error?.message ?? error)}`)
        return respond(res, 500, { ok: false, error: 'pairing-failed' })
      }
    }
    if (path === '/snapshot') {
      if (req.method !== 'GET') return respond(res, 405, { ok: false, error: 'method-not-allowed' }, { allow: 'GET' })
      let ok = false
      try {
        ok = await authorized(req)
      } catch (error) {
        logger.warn?.(`[${name}] token read failed: ${String(error?.message ?? error)}`)
        return respond(res, 500, { ok: false, error: 'credentials-unavailable' })
      }
      if (!ok) return respond(res, 401, { ok: false, error: 'unauthorized' })
      try {
        return respond(res, 200, await buildSnapshot())
      } catch (error) {
        logger.warn?.(`[${name}] snapshot failed: ${String(error?.message ?? error)}`)
        return respond(res, 500, { ok: false, error: 'snapshot-failed' })
      }
    }
    return respond(res, 404, { ok: false, error: 'not-found' })
  }

  /**
   * 启动监听。
   * @returns {Promise<void>} 监听成功后完成；端口占用等错误会拒绝。
   */
  async function start() {
    if (server !== null) return
    await ensureBridgeId()
    await rotatePairingCode('start')
    server = createServer((req, res) => {
      // 只接受本机来源；服务已绑定 127.0.0.1，这里再做一次防御。
      const remote = String(req.socket?.remoteAddress ?? '')
      if (!['127.0.0.1', '::1', '::ffff:127.0.0.1'].includes(remote)) {
        respond(res, 403, { ok: false, error: 'loopback-only' })
        return
      }
      void handleRequest(req, res).catch(error => {
        logger.warn?.(`[${name}] request failed: ${String(error?.message ?? error)}`)
        if (!res.headersSent) respond(res, 500, { ok: false, error: 'internal-error' })
        else res.end()
      })
    })
    await new Promise((resolve, reject) => {
      const onError = error => {
        server?.off('listening', onListening)
        reject(error)
      }
      const onListening = () => {
        server?.off('error', onError)
        resolve()
      }
      server.once('error', onError)
      server.once('listening', onListening)
      server.listen(config.port, '127.0.0.1')
    }).catch(error => {
      const address = `127.0.0.1:${config.port}`
      const reason = error?.code === 'EADDRINUSE'
        ? `port ${config.port} on 127.0.0.1 is already in use; change the plugin config "port"`
        : String(error?.message ?? error)
      throw new Error(`[${name}] cannot listen on ${address}: ${reason}`)
    })
    port = server.address()?.port ?? config.port
    logger.info?.(`[${name}] listening on 127.0.0.1:${port} (bridge ${bridgeId}, instance ${bridgeInstanceId})`)
    rotateTimer = setInterval(() => {
      void refreshPairingCode().catch(error => {
        logger.warn?.(`[${name}] pairing-code rotation failed: ${String(error?.message ?? error)}`)
      })
    }, config.rotateCheckMs)
    rotateTimer.unref?.()
  }

  /**
   * 停止监听并清理定时器。
   * @returns {Promise<void>} 完成。
   */
  async function stop() {
    stopped = true
    if (rotateTimer !== null) {
      clearInterval(rotateTimer)
      rotateTimer = null
    }
    const active = server
    server = null
    if (active === null) return
    await new Promise(resolve => {
      active.close(() => resolve())
      active.closeAllConnections?.()
    })
  }

  return {
    bridgeInstanceId,
    since,
    startedAt,
    start,
    stop,
    handleSessionEvent,
    handleAgentStatus,
    buildSnapshot,
    /** 配对页视图：码只给已认证的同源页面，不进入 47832。 */
    pairingView: async () => {
      let paired = false
      let error = null
      try {
        const record = await readCredential(TOKEN_CREDENTIAL)
        paired = record !== undefined && record !== null && record?.payload?.hash !== undefined
      } catch (failure) {
        error = 'credentials-unavailable'
      }
      return { bridgeId, code: pairingCode, expiresAt: pairingCodeExpiresAt, paired, error }
    },
    /** 配对页“轮换”动作：已配对时拒绝。 */
    rotatePairingCodeForPage: async () => enqueuePairing(async () => {
      const record = await readCredential(TOKEN_CREDENTIAL)
      if (record !== undefined && record !== null && record?.payload?.hash !== undefined) {
        throw Object.assign(new Error('already-paired'), { code: 'already-paired' })
      }
      await rotatePairingCode('manual')
    }),
    /** 配对页“撤销”动作：清 token；旧 Bearer 立即失效。 */
    revokeTokenForPage: async () => enqueuePairing(async () => {
      await deleteCredential(TOKEN_CREDENTIAL)
      if (pairingCode === null) await rotatePairingCode('after-revoke')
    }),
    /** 配对页 CSRF 令牌（只随页面下发）。 */
    pageCsrfToken: () => pageCsrfToken,
    /** 测试访问器：当前配对码（仅内存，不对外暴露）。 */
    currentPairingCode: () => pairingCode,
    /** 测试访问器：当前监听端口。 */
    currentPort: () => port,
    /** 测试访问器：强制让配对码过期。 */
    expirePairingCodeForTest: () => {
      pairingCodeExpiresAt = 0
    },
    /** 测试访问器：清空 token（模拟撤销）。 */
    revokeTokenForTest: async () => {
      await deleteCredential(TOKEN_CREDENTIAL)
    },
    /** 关闭状态。 */
    isStopped: () => stopped,
    /** 稳定桥身份（start 前为 null）。 */
    currentBridgeId: () => bridgeId,
  }
}

/** 配对页 HTML 响应辅助。 */
function respondHtml(response, status, body) {
  const payload = Buffer.from(body, 'utf8')
  response.writeHead(status, {
    'content-type': 'text/html; charset=utf-8',
    'cache-control': 'no-store',
    'content-security-policy': "default-src 'none'; style-src 'unsafe-inline'; form-action 'self'; base-uri 'none'",
    'content-length': payload.length,
  })
  response.end(payload)
}

/**
 * 同源 JSON 响应（状态接口用；不包含任何凭据）。
 * @param {any} response 响应。
 * @param {number} status 状态码。
 * @param {any} body JSON 体。
 * @param {Record<string,string>} [headers] 额外头。
 * @returns {void}
 */
function readJsonBody(req) {
  return new Promise((resolve, reject) => {
    const chunks = []
    let size = 0
    req.on('data', chunk => {
      size += chunk.length
      if (size > MAX_BODY_BYTES) {
        reject(Object.assign(new Error('body-too-large'), { statusCode: 413 }))
        req.destroy()
        return
      }
      chunks.push(chunk)
    })
    req.on('end', () => {
      const text = Buffer.concat(chunks).toString('utf8').trim()
      if (text.length === 0) return resolve({})
      try {
        const parsed = JSON.parse(text)
        if (parsed === null || typeof parsed !== 'object' || Array.isArray(parsed)) {
          reject(Object.assign(new Error('invalid-json'), { statusCode: 400 }))
          return
        }
        resolve(parsed)
      } catch {
        reject(Object.assign(new Error('invalid-json'), { statusCode: 400 }))
      }
    })
    req.on('error', reject)
  })
}

function respondJson(response, status, body, headers = {}) {
  const payload = JSON.stringify(body)
  response.writeHead(status, {
    'content-type': 'application/json; charset=utf-8',
    'cache-control': 'no-store',
    'content-length': Buffer.byteLength(payload),
    ...headers,
  })
  response.end(payload)
}

/**
 * 渲染配对页。
 * @param {any} view 配对视图。
 * @param {string} csrf CSRF 令牌。
 * @param {string} [message] 操作提示。
 * @returns {string} HTML。
 */
function renderPairingPage(view, csrf, message = '') {
  const bridge = escapeHtml(view.bridgeId ?? '未初始化')
  const status = view.error
    ? '<p class="err">凭据层不可用，无法读取或写入配对记录。</p>'
    : view.paired
      ? '<p class="ok">已配对。若要重新配对，先撤销当前 token。</p>'
      : view.code
        ? `<p>配对码：<code>${escapeHtml(view.code)}</code></p>
           <p>到期：${escapeHtml(new Date(view.expiresAt).toISOString())}</p>`
        : '<p>当前没有有效配对码。</p>'
  const rotate = !view.error && !view.denied
    ? `<form method="post" action="/dsh-bridge/pairing">
         <input type="hidden" name="csrf" value="${escapeHtml(csrf)}">
         <input type="hidden" name="action" value="rotate">
         <button type="submit">轮换配对码</button>
       </form>`
    : ''
  const revoke = !view.error && !view.denied
    ? `<form method="post" action="/dsh-bridge/pairing">
         <input type="hidden" name="csrf" value="${escapeHtml(csrf)}">
         <input type="hidden" name="action" value="revoke">
         <button type="submit">撤销已配对</button>
       </form>`
    : ''
  return `<!doctype html>
<html lang="zh-CN">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>DSH Desktop Bridge 配对</title>
<style>
  body { font: 14px/1.6 system-ui, sans-serif; margin: 2rem auto; max-width: 42rem; padding: 0 1rem; color: #222; }
  code { background: #f2f2f2; padding: .1rem .35rem; border-radius: 4px; }
  form { display: inline-block; margin: .4rem .6rem .4rem 0; }
  button { padding: .35rem .8rem; }
  .ok { color: #1a7f37; }
  .err { color: #b42318; }
  .muted { color: #666; }
</style>
</head>
<body>
<h1>DSH Desktop Bridge 配对</h1>
<p>桥身份：<code>${bridge}</code></p>
${status}
${message ? `<p class="err">${escapeHtml(message)}</p>` : ''}
${rotate}
${revoke}
<p class="muted">此页面只在 DSH 窗口内可用；配对码不会写入日志，也不会从 47832 端口返回。配对成功后旧快照由 bridgeInstanceId 作废。</p>
</body>
</html>`
}

/**
 * 构造 owner 认证检查：返回 true/false/null（null 表示当前组合没有 Connection）。
 * @param {any} scoped 注入 webServer 后的上下文。
 * @param {any} ctx 插件上下文（兜底取 connection）。
 * @returns {(request: any) => boolean | null} 检查函数。
 */
function makeOwnerCheck(scoped, ctx) {
  const connectionService = () => {
    for (const scope of [scoped, ctx]) {
      try {
        const found = scope?.get?.('connection')
        if (found !== null && found !== undefined) return found
      } catch {
        // 没有该服务的组合是正常的。
      }
    }
    return null
  }
  const isSecure = () => {
    try {
      return scoped?.get?.('webServer')?.protocol === 'https:'
    } catch {
      return false
    }
  }
  return request => {
    if (!sameOriginRequest(request, isSecure)) return false
    const auth = connectionService()?.browserAuth
    if (auth === null || auth === undefined || typeof auth.isAuthenticated !== 'function') return null
    try {
      return auth.isAuthenticated(request, isSecure()) === true
    } catch {
      return false
    }
  }
}

/**
 * 浏览器同源准入：带 Origin 的请求必须与受信监听协议和请求 Host 完整同源，挡掉跨站页面。
 * 不带 Origin 的请求（同窗口导航、curl）继续交给 owner 认证判断。
 * @param {any} request 请求。
 * @param {() => boolean} isSecure 监听是否 HTTPS。
 * @returns {boolean} 是否同源。
 */
function sameOriginRequest(request, isSecure) {
  const origin = request?.headers?.origin
  if (typeof origin !== 'string' || origin.length === 0) return true
  try {
    const parsed = new URL(origin)
    const expected = isSecure() ? 'https:' : 'http:'
    return parsed.protocol === expected && parsed.host === String(request?.headers?.host ?? '')
  } catch {
    return false
  }
}

/**
 * 创建配对页请求处理器。
 * @param {any} scoped 注入 webServer 后的上下文。
 * @param {any} ctx 插件上下文（兜底取 connection）。
 * @param {any} bridge 桥实例。
 * @returns {(request: any, response: any) => Promise<void>} 处理器。
 */
function createPairingPageHandler(scoped, ctx, bridge) {
  const ownerAuthenticated = makeOwnerCheck(scoped, ctx)
  const readForm = request => new Promise((resolve, reject) => {
    const chunks = []
    let size = 0
    request.on('data', chunk => {
      size += chunk.length
      if (size > MAX_FORM_BYTES) {
        reject(Object.assign(new Error('form-too-large'), { statusCode: 413 }))
        request.destroy()
        return
      }
      chunks.push(chunk)
    })
    request.on('end', () => resolve(new URLSearchParams(Buffer.concat(chunks).toString('utf8'))))
    request.on('error', reject)
  })
  return async (request, response) => {
    const authenticated = ownerAuthenticated(request)
    if (authenticated === null) {
      respondHtml(response, 503, renderPairingPage({ bridgeId: bridge.currentBridgeId(), code: null, expiresAt: 0, paired: false, error: 'owner-auth-unavailable' }, bridge.pageCsrfToken(), '当前组合没有 Connection 服务，无法验证 DSH 窗口身份。'))
      return
    }
    if (authenticated !== true) {
      respondHtml(response, 401, renderPairingPage({ bridgeId: bridge.currentBridgeId(), code: null, expiresAt: 0, paired: false, error: null, denied: true }, bridge.pageCsrfToken(), '需要 DSH 窗口身份（请从 DSH 设置页打开）。'))
      return
    }
    if (request.method === 'GET') {
      const view = await bridge.pairingView()
      respondHtml(response, 200, renderPairingPage(view, bridge.pageCsrfToken()))
      return
    }
    if (request.method !== 'POST') {
      respondHtml(response, 405, renderPairingPage(await bridge.pairingView(), bridge.pageCsrfToken(), '只支持 GET/POST。'))
      return
    }
    const contentType = String(request.headers['content-type'] ?? '').toLowerCase()
    if (!contentType.startsWith('application/x-www-form-urlencoded')) {
      respondHtml(response, 415, renderPairingPage(await bridge.pairingView(), bridge.pageCsrfToken(), '表单必须使用 application/x-www-form-urlencoded。'))
      return
    }
    let form
    try {
      form = await readForm(request)
    } catch (error) {
      respondHtml(response, error?.statusCode ?? 400, renderPairingPage(await bridge.pairingView(), bridge.pageCsrfToken(), '请求体无法读取。'))
      return
    }
    if (form.get('csrf') !== bridge.pageCsrfToken()) {
      respondHtml(response, 400, renderPairingPage(await bridge.pairingView(), bridge.pageCsrfToken(), 'CSRF 校验失败，请刷新页面重试。'))
      return
    }
    const action = form.get('action')
    try {
      if (action === 'rotate') await bridge.rotatePairingCodeForPage()
      else if (action === 'revoke') await bridge.revokeTokenForPage()
      else {
        respondHtml(response, 400, renderPairingPage(await bridge.pairingView(), bridge.pageCsrfToken(), '未知操作。'))
        return
      }
    } catch (error) {
      const alreadyPaired = error?.code === 'already-paired'
      const message = alreadyPaired ? '已配对，先撤销当前 token 再轮换。' : '操作失败：凭据层不可用。'
      respondHtml(response, alreadyPaired ? 409 : 500, renderPairingPage(await bridge.pairingView(), bridge.pageCsrfToken(), message))
      return
    }
    respondHtml(response, 200, renderPairingPage(await bridge.pairingView(), bridge.pageCsrfToken(), action === 'revoke' ? '已撤销当前 token。' : '已轮换配对码。'))
  }
}

/**
 * 创建只读状态处理器：给设置页显示已配对/未配对与 bridgeId，不返回配对码。
 * @param {any} scoped 注入 webServer 后的上下文。
 * @param {any} ctx 插件上下文。
 * @param {any} bridge 桥实例。
 * @returns {(request: any, response: any) => Promise<void>} 处理器。
 */
function createStatusHandler(scoped, ctx, bridge) {
  const ownerAuthenticated = makeOwnerCheck(scoped, ctx)
  return async (request, response) => {
    const authenticated = ownerAuthenticated(request)
    if (authenticated === null) {
      respondJson(response, 503, { ok: false, error: 'owner-auth-unavailable' })
      return
    }
    if (authenticated !== true) {
      respondJson(response, 401, { ok: false, error: 'unauthorized' })
      return
    }
    if (request.method !== 'GET') {
      respondJson(response, 405, { ok: false, error: 'method-not-allowed' }, { allow: 'GET' })
      return
    }
    const view = await bridge.pairingView()
    if (view.error) {
      respondJson(response, 500, { ok: false, error: 'credentials-unavailable' })
      return
    }
    respondJson(response, 200, {
      ok: true,
      protocolVersion: PROTOCOL_VERSION,
      bridgeId: view.bridgeId,
      bridgeInstanceId: bridge.bridgeInstanceId,
      paired: view.paired,
    })
  }
}

/**
 * 创建设置页内嵌配对面板的数据接口：owner 认证后返回配对码与状态，
 * POST 用同一 CSRF 令牌触发轮换或撤销。只在同源、已认证的 DSH 页面可用。
 * @param {any} scoped 注入 webServer 后的上下文。
 * @param {any} ctx 插件上下文。
 * @param {any} bridge 桥实例。
 * @returns {(request: any, response: any) => Promise<void>} 处理器。
 */
function createPairingJsonHandler(scoped, ctx, bridge) {
  const ownerAuthenticated = makeOwnerCheck(scoped, ctx)
  const noStore = { 'cache-control': 'no-store' }
  return async (request, response) => {
    /** 返回当前配对视图；已配对时不返回配对码。 */
    const sendView = async statusCode => {
      const view = await bridge.pairingView()
      if (view.error) {
        respondJson(response, 500, { ok: false, error: 'credentials-unavailable' }, noStore)
        return
      }
      respondJson(response, statusCode ?? 200, {
        ok: true,
        protocolVersion: PROTOCOL_VERSION,
        bridgeId: view.bridgeId,
        bridgeInstanceId: bridge.bridgeInstanceId,
        paired: view.paired,
        code: view.paired ? null : view.code,
        expiresAt: view.paired ? 0 : view.expiresAt,
        csrf: bridge.pageCsrfToken(),
      }, noStore)
    }
    const authenticated = ownerAuthenticated(request)
    if (authenticated === null) {
      respondJson(response, 503, { ok: false, error: 'owner-auth-unavailable' }, noStore)
      return
    }
    if (authenticated !== true) {
      respondJson(response, 401, { ok: false, error: 'unauthorized' }, noStore)
      return
    }
    if (request.method === 'GET') {
      await sendView()
      return
    }
    if (request.method !== 'POST') {
      respondJson(response, 405, { ok: false, error: 'method-not-allowed' }, { ...noStore, allow: 'GET, POST' })
      return
    }
    let body
    try {
      body = await readJsonBody(request)
    } catch (error) {
      respondJson(response, error?.statusCode ?? 400, { ok: false, error: 'invalid-json' }, noStore)
      return
    }
    if (body?.csrf !== bridge.pageCsrfToken()) {
      respondJson(response, 400, { ok: false, error: 'csrf-mismatch' }, noStore)
      return
    }
    try {
      if (body.action === 'rotate') await bridge.rotatePairingCodeForPage()
      else if (body.action === 'revoke') await bridge.revokeTokenForPage()
      else {
        respondJson(response, 400, { ok: false, error: 'unknown-action' }, noStore)
        return
      }
    } catch (error) {
      const alreadyPaired = error?.code === 'already-paired'
      respondJson(response, alreadyPaired ? 409 : 500, { ok: false, error: alreadyPaired ? 'already-paired' : 'credential-unavailable' }, noStore)
      return
    }
    await sendView()
  }
}

/**
 * 在 DSH Web 服务上注册同源配对页与只读状态接口。
 * @param {any} ctx 插件上下文。
 * @param {any} bridge 桥实例。
 * @returns {boolean} 是否已注册。
 */
export function registerPairingPage(ctx, bridge) {
  let registered = false
  const setup = scoped => {
    try {
      const webServer = scoped?.webServer ?? ctx?.webServer
      if (webServer === null || webServer === undefined || typeof webServer.register !== 'function') return
      const pageHandler = createPairingPageHandler(scoped ?? ctx, ctx, bridge)
      const statusHandler = createStatusHandler(scoped ?? ctx, ctx, bridge)
      const pairingJsonHandler = createPairingJsonHandler(scoped ?? ctx, ctx, bridge)
      const register = () => [
        webServer.register({ kind: 'exact', path: PAIRING_PATH, handler: pageHandler }),
        webServer.register({ kind: 'exact', path: STATUS_PATH, handler: statusHandler }),
        webServer.register({ kind: 'exact', path: PAIRING_JSON_PATH, handler: pairingJsonHandler }),
      ]
      if (typeof scoped?.effect === 'function') scoped.effect(register, `${name}: pairing page`)
      else if (typeof ctx?.effect === 'function') ctx.effect(register, `${name}: pairing page`)
      else register()
      registered = true
    } catch (error) {
      ctx.logger?.warn?.(`[${name}] pairing page registration failed: ${String(error?.message ?? error)}`)
    }
  }
  if (typeof ctx?.inject === 'function') {
    try {
      ctx.inject(['webServer'], setup)
    } catch (error) {
      ctx.logger?.warn?.(`[${name}] webServer inject failed: ${String(error?.message ?? error)}`)
    }
  } else {
    setup(ctx)
  }
  return registered
}

/**
 * Cordis Host 插件入口：启动只读桥、注册事件订阅与同源配对页。
 * @param {any} ctx Cordis 上下文。
 * @param {unknown} config 行配置。
 * @returns {Promise<void>} 启动完成。
 */
export async function apply(ctx, config) {
  const bridge = createBridge({ ctx, config, logger: ctx?.logger })
  await bridge.start()
  registerPairingPage(ctx, bridge)
  try {
    ctx.effect(() => {
      const offSessionEvent = ctx.on('session/event', (session, event) => bridge.handleSessionEvent(session, event))
      const offAgentStatus = ctx.on('agent/status', payload => bridge.handleAgentStatus(payload))
      return () => {
        offSessionEvent?.()
        offAgentStatus?.()
        return bridge.stop()
      }
    }, `${name}: read-only bridge`)
  } catch (error) {
    await bridge.stop()
    throw error
  }
}
