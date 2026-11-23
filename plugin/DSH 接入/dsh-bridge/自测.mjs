/**
 * dsh-desktop-bridge 自测。
 *
 * 用假的 ctx 桩启动桥，覆盖：health/pair/snapshot/401/405/404、并发单次配对、
 * 过期 401、撤销 401、凭据层故障 500、Host/Content-Type 边界、稳定 bridgeId、
 * revision 覆盖余额/待决/job、余额缓存与超时、同源配对页的 owner 认证与撤销。
 *
 * 运行：node dsh-bridge/自测.mjs
 */

import { readFileSync } from 'node:fs'
import { createServer as createHttpServer, request as httpRequest } from 'node:http'
import { runInNewContext } from 'node:vm'
import { apply, createBridge, registerPairingPage } from './lib/index.js'

/** 测试用 owner 会话 Cookie；不涉及真实凭据。 */
const OWNER_COOKIE = 'owner-session-cookie'

/** 已通过的步骤数。 */
let passed = 0
/** 失败的步骤数。 */
let failed = 0

/**
 * 步骤断言。
 * @param {string} label 步骤名。
 * @param {boolean} condition 断言条件。
 * @param {string} [detail] 失败说明。
 * @returns {void}
 */
function check(label, condition, detail = '') {
  if (condition) {
    passed += 1
    console.log(`[pass] ${label}`)
  } else {
    failed += 1
    console.log(`[FAIL] ${label}${detail ? ` — ${detail}` : ''}`)
  }
}

/**
 * 构造可控制故障的假 DSH ctx。
 * @returns {any} 假 ctx 与可观察状态。
 */
function makeFakeCtx() {
  const records = new Map()
  const logs = []
  const warnings = []
  const state = {
    failRead: false,
    failWrite: false,
    failDelete: false,
    balanceStatus: 'ready',
    balanceValue: '12.34',
    hangBalance: false,
    jobRunning: true,
    balanceCalls: 0,
  }
  const ctx = {
    logger: {
      debug() {},
      info: message => logs.push(String(message)),
      warn: message => warnings.push(String(message)),
      error(message) {
        warnings.push(String(message))
      },
    },
    credentials: {
      async readRecord(key) {
        if (state.failRead) throw new Error('credential-read-failed')
        return records.get(key)
      },
      async modifyRecord(key, mutate) {
        if (state.failWrite) throw new Error('credential-write-failed')
        const next = await mutate(records.get(key))
        if (next !== undefined) records.set(key, next)
        return next
      },
      async deleteRecord(key) {
        if (state.failDelete) throw new Error('credential-delete-failed')
        records.delete(key)
      },
    },
    sessions: {
      list() {
        return [{ header: { id: 's1', cwd: 'E:\\Project\\WebView\\DesktopTools' } }]
      },
    },
    agents: {
      list() {
        return [{
          id: 's1',
          session: { header: { id: 's1', cwd: 'E:\\Project\\WebView\\DesktopTools' } },
          status: 'idle',
        }]
      },
    },
    jobs: {
      list(sessionId) {
        if (sessionId === undefined) return [{ id: 'job-unowned', label: 'sync', status: 'completed' }]
        if (sessionId === 's1') return [{ id: 'job-1', label: 'npm test', status: state.jobRunning ? 'running' : 'completed' }]
        return []
      },
    },
    deepseekAccount: {
      async getState() {
        return { status: 'credential-stored' }
      },
      async getBalance() {
        state.balanceCalls += 1
        if (state.hangBalance) return new Promise(() => {})
        return { status: state.balanceStatus, value: [{ currency: 'CNY', balance: state.balanceValue }] }
      },
    },
    on() {
      return () => {}
    },
    effect(run) {
      const dispose = run()
      return typeof dispose === 'function' ? dispose : () => {}
    },
    get(name) {
      return ctx[name]
    },
  }
  return { ctx, records, logs, warnings, state }
}

/**
 * 发一个 HTTP 请求。
 * @param {string} base 基地址。
 * @param {string} path 路径。
 * @param {RequestInit} [init] fetch 选项。
 * @returns {Promise<{ status: number, body: any, raw: string, headers: Headers, redirected: boolean }>} 响应。
 */
async function request(base, path, init = {}) {
  const response = await fetch(`${base}${path}`, init)
  const raw = await response.text()
  let body = null
  try {
    body = JSON.parse(raw)
  } catch {
    body = null
  }
  return { status: response.status, body, raw, headers: response.headers, redirected: response.redirected }
}

/**
 * 用原始 node:http 请求（可以伪造 Host 头）。
 * @param {number} port 端口。
 * @param {string} path 路径。
 * @param {Record<string,string>} headers 头。
 * @returns {Promise<{ status: number, body: string }>} 响应。
 */
function rawGet(port, path, headers = {}) {
  return new Promise((resolve, reject) => {
    const req = httpRequest({ host: '127.0.0.1', port, path, method: 'GET', headers }, response => {
      const chunks = []
      response.on('data', chunk => chunks.push(chunk))
      response.on('end', () => resolve({ status: response.statusCode, body: Buffer.concat(chunks).toString('utf8') }))
    })
    req.on('error', reject)
    req.end()
  })
}

/**
 * 发配对请求。
 * @param {string} base 基地址。
 * @param {string} code 配对码。
 * @param {Record<string,string>} [headers] 额外头。
 * @returns {Promise<any>} 响应。
 */
function pairRequest(base, code, headers = {}) {
  return request(base, '/pair', {
    method: 'POST',
    headers: { 'content-type': 'application/json', ...headers },
    body: JSON.stringify({ code }),
  })
}

/**
 * 从配对页 HTML 里取 CSRF。
 * @param {string} html 页面。
 * @returns {string} CSRF 值。
 */
function csrfOf(html) {
  const match = /name="csrf" value="([^"]+)"/.exec(html)
  return match === null ? '' : match[1]
}

/**
 * 主流程。
 * @returns {Promise<void>} 完成。
 */
async function main() {
  const main = makeFakeCtx()
  const bridge = createBridge({
    ctx: main.ctx,
    config: { port: 0, pairingCodeTtlMs: 10 * 60 * 1000, rotateCheckMs: 60 * 1000, balanceCacheMs: 0, balanceTimeoutMs: 500 },
  })
  await bridge.start()
  const base = `http://127.0.0.1:${bridge.currentPort()}`

  // 1. health：含 bridgeId 与 bridgeInstanceId
  const health = await request(base, '/health')
  console.log(`[1] GET /health -> ${health.status}`)
  check('health 200', health.status === 200)
  check('health protocolVersion 1', health.body?.protocolVersion === 1)
  check('health bridgeId 稳定字段', typeof health.body?.bridgeId === 'string' && health.body.bridgeId.length > 0)
  check('health bridgeInstanceId', typeof health.body?.bridgeInstanceId === 'string' && health.body.bridgeInstanceId !== health.body.bridgeId)
  check('health since', typeof health.body?.since === 'string' && health.body.since.length > 0)

  // 2. Host 必须精确是 127.0.0.1:<port>
  const badHost = await rawGet(bridge.currentPort(), '/health', { Host: 'evil.example' })
  console.log(`[2] GET /health (Host: evil.example) -> ${badHost.status}`)
  check('错误 Host 421', badHost.status === 421)

  // 3. 未配对 snapshot
  const unauth = await request(base, '/snapshot')
  console.log(`[3] GET /snapshot (no token) -> ${unauth.status}`)
  check('未配对 401', unauth.status === 401)

  // 4. /pair 必须 application/json
  const badType = await request(base, '/pair', {
    method: 'POST',
    headers: { 'content-type': 'text/plain' },
    body: JSON.stringify({ code: bridge.currentPairingCode() }),
  })
  console.log(`[4] POST /pair (text/plain) -> ${badType.status}`)
  check('错误 Content-Type 415', badType.status === 415)

  // 5. 错误配对码
  const wrong = await pairRequest(base, 'AAAAA-AAAAA-AAAAA-AAAAA')
  console.log(`[5] POST /pair (wrong code) -> ${wrong.status}`)
  check('错误配对码 401', wrong.status === 401)

  // 6. 并发同一码：最多一个 200
  const code = bridge.currentPairingCode()
  const [first, second] = await Promise.all([pairRequest(base, code), pairRequest(base, code)])
  const okCount = [first, second].filter(result => result.status === 200).length
  console.log(`[6] concurrent POST /pair -> ${first.status} / ${second.status}`)
  check('并发配对最多一个 200', okCount === 1)
  const winner = first.status === 200 ? first : second
  const token = winner.body?.token
  check('并发配对返回 token', typeof token === 'string' && token.length >= 32)
  const auth = { authorization: `Bearer ${token}` }

  // 7. 配对日志不含码，也不含 token
  const logText = main.logs.join('\n')
  check('日志不含配对码', !logText.includes(code))
  check('日志不含 token', typeof token === 'string' && !logText.includes(token))

  // 8. 快照字段与 bridgeId
  const snapshot = await request(base, '/snapshot', { headers: auth })
  console.log(`[8] GET /snapshot (token) -> ${snapshot.status}`)
  check('快照 200', snapshot.status === 200)
  check('快照 bridgeId', snapshot.body?.bridgeId === health.body?.bridgeId)
  check('快照 bridgeInstanceId 与 health 同源', snapshot.body?.bridgeInstanceId === health.body?.bridgeInstanceId)
  check('快照 protocolVersion 1', snapshot.body?.protocolVersion === 1)
  check('快照 revision 为数字', typeof snapshot.body?.revision === 'number')
  check('余额 ready', snapshot.body?.balance?.status === 'ready')
  check('余额 CNY 12.34', snapshot.body?.balance?.wallets?.[0]?.currency === 'CNY' && snapshot.body?.balance?.wallets?.[0]?.amount === '12.34')
  check('会话 s1 idle', snapshot.body?.sessions?.some(row => row.id === 's1' && row.status === 'idle'))
  check('job-1 running', snapshot.body?.jobs?.some(row => row.id === 'job-1' && row.status === 'running'))
  check('job-unowned done', snapshot.body?.jobs?.some(row => row.id === 'job-unowned' && row.status === 'done'))

  // 9. 错误 token / 方法 / 未知路径
  const badToken = await request(base, '/snapshot', { headers: { authorization: 'Bearer not-the-token' } })
  console.log(`[9] GET /snapshot (bad token) -> ${badToken.status}`)
  check('错误 token 401', badToken.status === 401)
  const wrongMethod = await request(base, '/health', { method: 'POST' })
  console.log(`[10] POST /health -> ${wrongMethod.status}`)
  check('方法错误 405', wrongMethod.status === 405)
  const notFound = await request(base, '/nope')
  console.log(`[11] GET /nope -> ${notFound.status}`)
  check('未知路径 404', notFound.status === 404)

  // 12. revision 覆盖余额变化（无会话事件）
  const beforeBalance = snapshot.body?.revision
  main.state.balanceValue = '99.00'
  const afterBalance = await request(base, '/snapshot', { headers: auth })
  check('余额变化推进 revision', afterBalance.body?.revision > beforeBalance)
  check('余额值已更新', afterBalance.body?.balance?.wallets?.[0]?.amount === '99.00')

  // 13. revision 覆盖待决增删与 job 变化
  bridge.handleSessionEvent(
    { header: { id: 's1', cwd: 'E:\\Project\\WebView\\DesktopTools' } },
    { type: 'approval/asked', data: { id: 'ap-1', toolName: 'bash', callId: 'call-1' } },
  )
  const withPending = await request(base, '/snapshot', { headers: auth })
  check('待决登记 approval', withPending.body?.pending?.some(item => item.id === 'ap-1' && item.kind === 'approval'))
  check('会话变为 waiting', withPending.body?.sessions?.some(row => row.id === 's1' && row.status === 'waiting' && row.pending === 'approval'))
  check('待决推进 revision', withPending.body?.revision > afterBalance.body?.revision)
  bridge.handleSessionEvent(
    { header: { id: 's1' } },
    { type: 'approval/decided', data: { id: 'ap-1', outcome: 'rejected' } },
  )
  const afterDecided = await request(base, '/snapshot', { headers: auth })
  check('decided 后清除待决', afterDecided.body?.pending?.length === 0)
  check('清除待决推进 revision', afterDecided.body?.revision > withPending.body?.revision)
  main.state.jobRunning = false
  const afterJob = await request(base, '/snapshot', { headers: auth })
  check('job 变化推进 revision', afterJob.body?.revision > afterDecided.body?.revision)
  check('job-1 已 done', afterJob.body?.jobs?.some(row => row.id === 'job-1' && row.status === 'done'))

  // 14. 撤销 token 后旧 Bearer 401
  await bridge.revokeTokenForPage()
  const revoked = await request(base, '/snapshot', { headers: auth })
  console.log(`[14] GET /snapshot (revoked token) -> ${revoked.status}`)
  check('撤销后 401', revoked.status === 401)

  // 15. 配对码过期 -> 401（不再 410）
  const expiring = createBridge({ ctx: main.ctx, config: { port: 0, pairingCodeTtlMs: 80, rotateCheckMs: 60 * 1000 } })
  await expiring.start()
  const expiringBase = `http://127.0.0.1:${expiring.currentPort()}`
  await new Promise(resolve => setTimeout(resolve, 200))
  const expired = await pairRequest(expiringBase, expiring.currentPairingCode() ?? 'AAAAA-AAAAA-AAAAA-AAAAA')
  console.log(`[15] POST /pair (expired code) -> ${expired.status}`)
  check('过期配对码 401', expired.status === 401)
  await expiring.stop()

  // 16. 凭据层故障：无假成功
  const failure = makeFakeCtx()
  const failureBridge = createBridge({ ctx: failure.ctx, config: { port: 0 } })
  await failureBridge.start()
  const failureBase = `http://127.0.0.1:${failureBridge.currentPort()}`
  const failureCode = failureBridge.currentPairingCode()
  failure.state.failWrite = true
  const writeFailed = await pairRequest(failureBase, failureCode)
  console.log(`[16] POST /pair (credential write failure) -> ${writeFailed.status}`)
  check('凭据写失败 500', writeFailed.status === 500)
  check('写失败不返回 token', typeof writeFailed.body?.token !== 'string')
  failure.state.failWrite = false
  const afterFailure = await pairRequest(failureBase, failureCode)
  console.log(`[17] POST /pair (same code after write failure) -> ${afterFailure.status}`)
  check('写失败后码已被消费 401', afterFailure.status === 401)
  failure.state.failRead = true
  const readFailed = await pairRequest(failureBase, 'AAAAA-AAAAA-AAAAA-AAAAA')
  console.log(`[18] POST /pair (credential read failure) -> ${readFailed.status}`)
  check('凭据读失败 500', readFailed.status === 500)
  failure.state.failRead = false
  await failureBridge.stop()

  // 19. 稳定 bridgeId 跨重启
  const stable = makeFakeCtx()
  const stableA = createBridge({ ctx: stable.ctx, config: { port: 0 } })
  await stableA.start()
  const idA = stableA.currentBridgeId()
  const instanceA = stableA.bridgeInstanceId
  await stableA.stop()
  const stableB = createBridge({ ctx: stable.ctx, config: { port: 0 } })
  await stableB.start()
  const idB = stableB.currentBridgeId()
  console.log(`[19] bridgeId ${idA?.slice(0, 8)} -> ${idB?.slice(0, 8)}`)
  check('bridgeId 跨重启稳定', idA === idB && typeof idA === 'string')
  check('bridgeInstanceId 每次变化', stableB.bridgeInstanceId !== instanceA)
  await stableB.stop()

  // 20. 余额缓存 5 秒内只查一次
  const cached = makeFakeCtx()
  const cachedBridge = createBridge({ ctx: cached.ctx, config: { port: 0, balanceCacheMs: 5000 } })
  await cachedBridge.start()
  const cachedBase = `http://127.0.0.1:${cachedBridge.currentPort()}`
  const cachedCode = cachedBridge.currentPairingCode()
  const cachedPair = await pairRequest(cachedBase, cachedCode)
  const cachedAuth = { authorization: `Bearer ${cachedPair.body.token}` }
  await request(cachedBase, '/snapshot', { headers: cachedAuth })
  await request(cachedBase, '/snapshot', { headers: cachedAuth })
  console.log(`[20] balance getBalance calls within cache window -> ${cached.state.balanceCalls}`)
  check('余额缓存生效', cached.state.balanceCalls === 1)
  await cachedBridge.stop()

  // 21. 平台慢：超时只让余额 error/stale，不阻塞快照
  const slow = makeFakeCtx()
  slow.state.hangBalance = true
  const slowBridge = createBridge({ ctx: slow.ctx, config: { port: 0, balanceTimeoutMs: 50, balanceCacheMs: 0 } })
  await slowBridge.start()
  const slowBase = `http://127.0.0.1:${slowBridge.currentPort()}`
  const slowPair = await pairRequest(slowBase, slowBridge.currentPairingCode())
  const slowAuth = { authorization: `Bearer ${slowPair.body.token}` }
  const started = Date.now()
  const slowSnapshot = await request(slowBase, '/snapshot', { headers: slowAuth })
  const elapsed = Date.now() - started
  console.log(`[21] GET /snapshot with hanging balance -> ${slowSnapshot.status} in ${elapsed}ms`)
  check('慢平台快照仍 200', slowSnapshot.status === 200)
  check('慢平台余额 error', slowSnapshot.body?.balance?.status === 'error')
  check('慢平台快照 stale', slowSnapshot.body?.stale === true)
  check('慢平台不阻塞快照', elapsed < 1500)
  await slowBridge.stop()

  // 22. 同源配对页：owner 认证、展示码、撤销闭环
  const page = makeFakeCtx()
  const pageBridge = createBridge({ ctx: page.ctx, config: { port: 0 } })
  await pageBridge.start()
  const pageBase = `http://127.0.0.1:${pageBridge.currentPort()}`
  const pagePair = await pairRequest(pageBase, pageBridge.currentPairingCode())
  const pageAuth = { authorization: `Bearer ${pagePair.body.token}` }
  let pageHandler = null
  const pageHandlers = new Map()
  page.ctx.inject = (names, callback) => {
    if (!names.includes('webServer')) return
    callback({
      webServer: {
        register: config => {
          pageHandlers.set(config.path, config.handler)
          return () => {}
        },
      },
      get(name) {
        if (name === 'connection') {
          return { browserAuth: { isAuthenticated: request => request.headers.cookie === OWNER_COOKIE } }
        }
        if (name === 'webServer') return { protocol: 'http:' }
        return undefined
      },
      effect(run) {
        run()
        return () => {}
      },
    })
  }
  check('配对页注册', registerPairingPage(page.ctx, pageBridge) === true)
  pageHandler = pageHandlers.get('/dsh-bridge/pairing')
  const pageServer = createHttpServer((req, res) => {
    const handler = pageHandlers.get(String(req.url ?? '').split('?')[0])
    if (handler === undefined) {
      res.writeHead(404)
      res.end()
      return
    }
    void handler(req, res)
  })
  await new Promise(resolve => pageServer.listen(0, '127.0.0.1', resolve))
  const pagePort = pageServer.address().port
  const pageUrl = `http://127.0.0.1:${pagePort}/dsh-bridge/pairing`
  const statusUrl = `http://127.0.0.1:${pagePort}/dsh-bridge/status`
  const unauthStatus = await fetch(statusUrl)
  console.log(`[22a] GET /dsh-bridge/status (no cookie) -> ${unauthStatus.status}`)
  check('状态接口未认证 401', unauthStatus.status === 401)
  const authStatus = await fetch(statusUrl, { headers: { cookie: OWNER_COOKIE } })
  const authStatusBody = await authStatus.json()
  console.log(`[22b] GET /dsh-bridge/status (owner cookie) -> ${authStatus.status}`)
  check('状态接口已认证 200', authStatus.status === 200)
  check('状态接口 bridgeId', authStatusBody?.bridgeId === pageBridge.currentBridgeId())
  check('状态接口显示已配对', authStatusBody?.paired === true)
  check('状态接口不返回配对码', !JSON.stringify(authStatusBody).includes(pageBridge.currentPairingCode() ?? '不可能出现的码'))
  const unauthPage = await fetch(pageUrl)
  console.log(`[22] GET pairing page (no cookie) -> ${unauthPage.status}`)
  check('配对页未认证 401', unauthPage.status === 401)
  const unauthBody = await unauthPage.text()
  check('未认证页面不含码', !unauthBody.includes(pageBridge.currentPairingCode() ?? '不可能出现的码'))
  const authPage = await fetch(pageUrl, { headers: { cookie: OWNER_COOKIE } })
  const authBody = await authPage.text()
  console.log(`[23] GET pairing page (owner cookie) -> ${authPage.status}`)
  check('配对页已认证 200', authPage.status === 200)
  check('配对页显示已配对', authBody.includes('已配对'))
  const csrf = csrfOf(authBody)
  check('配对页含 CSRF', csrf.length > 0)
  const rotateWhilePaired = await fetch(pageUrl, {
    method: 'POST',
    headers: { cookie: OWNER_COOKIE, 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ action: 'rotate', csrf }),
  })
  console.log(`[24] POST pairing page rotate (paired) -> ${rotateWhilePaired.status}`)
  check('已配对时轮换 409', rotateWhilePaired.status === 409)
  const revokePage = await fetch(pageUrl, {
    method: 'POST',
    headers: { cookie: OWNER_COOKIE, 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ action: 'revoke', csrf }),
  })
  console.log(`[25] POST pairing page revoke -> ${revokePage.status}`)
  check('配对页撤销 200', revokePage.status === 200)
  const pageRevoked = await request(pageBase, '/snapshot', { headers: pageAuth })
  check('配对页撤销后旧 Bearer 401', pageRevoked.status === 401)
  const revokedStatus = await fetch(statusUrl, { headers: { cookie: OWNER_COOKIE } })
  const revokedStatusBody = await revokedStatus.json()
  check('状态接口撤销后未配对', revokedStatusBody?.paired === false)
  const revokeBody = await revokePage.text()
  const rotateCsrf = csrfOf(revokeBody)
  const rotateUnpaired = await fetch(pageUrl, {
    method: 'POST',
    headers: { cookie: OWNER_COOKIE, 'content-type': 'application/x-www-form-urlencoded' },
    body: new URLSearchParams({ action: 'rotate', csrf: rotateCsrf }),
  })
  console.log(`[26] POST pairing page rotate (unpaired) -> ${rotateUnpaired.status}`)
  check('未配对时轮换 200', rotateUnpaired.status === 200)
  check('配对页不重定向', authPage.redirected === false && rotateUnpaired.redirected === false)
  const noCookieHeader = [...revokePage.headers.keys()].filter(name => name === 'set-cookie')
  check('配对页不设 Cookie', noCookieHeader.length === 0)
  // 内嵌配对面板（JSON 路由）：未认证、已配对不返码、撤销与轮换。
  const jsonUrl = `http://127.0.0.1:${pagePort}/dsh-bridge/pairing.json`
  const unauthJson = await fetch(jsonUrl)
  check('内嵌配对面板未认证 401', unauthJson.status === 401)
  const jsonPair = await pairRequest(pageBase, pageBridge.currentPairingCode())
  const jsonAuth = { authorization: `Bearer ${jsonPair.body.token}` }
  const authJson = await fetch(jsonUrl, { headers: { cookie: OWNER_COOKIE } })
  const authJsonBody = await authJson.json()
  check('内嵌配对面板已认证 200', authJson.status === 200)
  check('内嵌配对面板已配对不返回码', authJsonBody?.code === null)
  check('内嵌配对面板不泄露码', !JSON.stringify(authJsonBody).includes(pageBridge.currentPairingCode() ?? '不可能出现的码'))
  const jsonRevoke = await fetch(jsonUrl, {
    method: 'POST',
    headers: { cookie: OWNER_COOKIE, 'content-type': 'application/json' },
    body: JSON.stringify({ action: 'revoke', csrf: authJsonBody?.csrf }),
  })
  const jsonRevokeBody = await jsonRevoke.json()
  console.log(`[26a] POST pairing.json revoke -> ${jsonRevoke.status}`)
  check('内嵌配对面板撤销 200', jsonRevoke.status === 200)
  check('内嵌配对面板撤销后未配对', jsonRevokeBody?.paired === false)
  const jsonRevokedBearer = await request(pageBase, '/snapshot', { headers: jsonAuth })
  check('内嵌配对面板撤销后旧 Bearer 401', jsonRevokedBearer.status === 401)
  const jsonCode = String(jsonRevokeBody?.code ?? '')
  check('内嵌配对面板撤销后返回新码', jsonCode.length > 0)
  const jsonBadCsrf = await fetch(jsonUrl, {
    method: 'POST',
    headers: { cookie: OWNER_COOKIE, 'content-type': 'application/json' },
    body: JSON.stringify({ action: 'rotate', csrf: 'wrong' }),
  })
  check('内嵌配对面板错误 CSRF 400', jsonBadCsrf.status === 400)
  const jsonRotate = await fetch(jsonUrl, {
    method: 'POST',
    headers: { cookie: OWNER_COOKIE, 'content-type': 'application/json' },
    body: JSON.stringify({ action: 'rotate', csrf: jsonRevokeBody?.csrf }),
  })
  const jsonRotateBody = await jsonRotate.json()
  console.log(`[26b] POST pairing.json rotate -> ${jsonRotate.status}`)
  check('内嵌配对面板轮换 200', jsonRotate.status === 200)
  check('内嵌配对面板轮换换码', String(jsonRotateBody?.code ?? '') !== jsonCode)
  const crossSchemeOrigin = await fetch(jsonUrl, { headers: { cookie: OWNER_COOKIE, origin: `https://127.0.0.1:${pagePort}` } })
  check('内嵌配对面板拒绝异协议同源', crossSchemeOrigin.status === 401)
  const sameSchemeOrigin = await fetch(jsonUrl, { headers: { cookie: OWNER_COOKIE, origin: `http://127.0.0.1:${pagePort}` } })
  check('内嵌配对面板接受同协议同源', sameSchemeOrigin.status === 200)
  const crossHostOrigin = await fetch(jsonUrl, { headers: { cookie: OWNER_COOKIE, origin: 'http://127.0.0.1:1' } })
  check('内嵌配对面板拒绝异主机', crossHostOrigin.status === 401)
  await new Promise(resolve => pageServer.close(resolve))
  await pageBridge.stop()

  // 26. 生产合同：条数/长度截断、部分失败、在途合并、job 扫描有界
  const bound = makeFakeCtx()
  bound.ctx.sessions.list = () => Array.from({ length: 70 }, (_, index) => ({
    header: { id: `s${String(index).padStart(3, '0')}`, cwd: `/w/${index}` },
  }))
  bound.ctx.agents.list = () => []
  bound.ctx.jobs.list = sessionId => sessionId === undefined
    ? []
    : Array.from({ length: 5 }, (_, index) => ({
      id: `j-${sessionId}-${index}`,
      label: 'L'.repeat(400),
      status: 'running',
    }))
  bound.ctx.deepseekAccount.getBalance = async () => ({
    status: 'ready',
    value: Array.from({ length: 20 }, (_, index) => ({ currency: `CUR${index}`, balance: '1.00' })),
  })
  let jobCalls = 0
  const baseJobList = bound.ctx.jobs.list
  bound.ctx.jobs.list = sessionId => {
    jobCalls += 1
    return baseJobList(sessionId)
  }
  const boundBridge = createBridge({ ctx: bound.ctx, config: { port: 0, balanceCacheMs: 0, balanceTimeoutMs: 500 } })
  await boundBridge.start()
  const boundBase = `http://127.0.0.1:${boundBridge.currentPort()}`
  const boundPair = await pairRequest(boundBase, boundBridge.currentPairingCode())
  const boundAuth = { authorization: `Bearer ${boundPair.body.token}` }
  boundBridge.handleSessionEvent(
    { header: { id: 's000' } },
    { type: 'session/title', data: { title: `T${String.fromCharCode(0x85)}`.repeat(400) } },
  )
  for (let index = 0; index < 40; index += 1) {
    boundBridge.handleSessionEvent(
      { header: { id: 's000' } },
      { type: 'approval/asked', data: { id: `ap-${index}`, toolName: 'bash', callId: `call-${index}` } },
    )
  }
  const boundedSnapshot = await request(boundBase, '/snapshot', { headers: boundAuth })
  console.log(`[27] contract bounds: sessions=${boundedSnapshot.body?.sessions?.length} jobs=${boundedSnapshot.body?.jobs?.length} pending=${boundedSnapshot.body?.pending?.length} wallets=${boundedSnapshot.body?.balance?.wallets?.length}`)
  check('会话 ≤ 64', boundedSnapshot.body?.sessions?.length <= 64)
  check('job ≤ 32', boundedSnapshot.body?.jobs?.length <= 32)
  check('待决 ≤ 32', boundedSnapshot.body?.pending?.length <= 32)
  check('钱包 ≤ 16', boundedSnapshot.body?.balance?.wallets?.length <= 16)
  check('标题截断 ≤ 256', boundedSnapshot.body?.sessions?.every(row => row.title.length <= 256))
  check('标题无控制字符', boundedSnapshot.body?.sessions?.every(row => !/[\u0000-\u001f\u007f-\u009f]/.test(row.title)))
  check('job 名称截断 ≤ 256', boundedSnapshot.body?.jobs?.every(row => row.label.length <= 256))
  check('活待决优先保留', boundedSnapshot.body?.pending?.some(item => item.id === 'ap-39'))
  check('job 扫描覆盖全部可见会话（1 次全局 + 最多 64 次按会话）', jobCalls === 65)
  await boundBridge.stop()

  const brokenJobs = makeFakeCtx()
  brokenJobs.ctx.jobs.list = () => {
    throw new Error('jobs-down')
  }
  const brokenBridge = createBridge({ ctx: brokenJobs.ctx, config: { port: 0 } })
  await brokenBridge.start()
  const brokenBase = `http://127.0.0.1:${brokenBridge.currentPort()}`
  const brokenPair = await pairRequest(brokenBase, brokenBridge.currentPairingCode())
  const brokenSnapshot = await request(brokenBase, '/snapshot', {
    headers: { authorization: `Bearer ${brokenPair.body.token}` },
  })
  console.log(`[28] GET /snapshot (jobs service failure) -> ${brokenSnapshot.status}, stale=${brokenSnapshot.body?.stale}`)
  check('job 读取失败仍返回快照', brokenSnapshot.status === 200)
  check('job 读取失败标 stale', brokenSnapshot.body?.stale === true)
  check('job 读取失败保留会话', Array.isArray(brokenSnapshot.body?.sessions) && brokenSnapshot.body.sessions.length > 0)
  await brokenBridge.stop()

  const merge = makeFakeCtx()
  let releaseBalance = null
  const balanceGate = new Promise(resolve => {
    releaseBalance = resolve
  })
  merge.ctx.deepseekAccount.getBalance = async () => {
    merge.state.balanceCalls += 1
    await balanceGate
    return { status: 'ready', value: [{ currency: 'CNY', balance: '1.00' }] }
  }
  const mergeBridge = createBridge({ ctx: merge.ctx, config: { port: 0, balanceCacheMs: 0, balanceTimeoutMs: 500 } })
  await mergeBridge.start()
  const mergeBase = `http://127.0.0.1:${mergeBridge.currentPort()}`
  const mergePair = await pairRequest(mergeBase, mergeBridge.currentPairingCode())
  const mergeAuth = { authorization: `Bearer ${mergePair.body.token}` }
  const firstSnapshot = request(mergeBase, '/snapshot', { headers: mergeAuth })
  const secondSnapshot = request(mergeBase, '/snapshot', { headers: mergeAuth })
  await new Promise(resolve => setTimeout(resolve, 20))
  releaseBalance()
  const [firstResult, secondResult] = await Promise.all([firstSnapshot, secondSnapshot])
  console.log(`[29] concurrent balance calls -> ${merge.state.balanceCalls} (${firstResult.status}/${secondResult.status})`)
  check('余额在途请求合并', merge.state.balanceCalls === 1)
  check('合并后两份快照都成功', firstResult.status === 200 && secondResult.status === 200)
  await mergeBridge.stop()

  // 30. client bundle：注册 settings.section，不执行网络
  const clientSource = readFileSync(new URL('./lib/client.js', import.meta.url), 'utf8')
  let clientFactory = null
  const sandbox = {
    window: { __ModuleLoader__: { load: config => { clientFactory = config.factory } } },
    console: { warn() {} },
    require: () => ({ createElement: (type, props, ...children) => ({ type, props, children }) }),
    setInterval: () => 0,
    clearInterval: () => {},
  }
  runInNewContext(clientSource, sandbox)
  const clientExports = clientFactory(sandbox.require)
  let registered = null
  const clientCtx = {
    slots: {
      inject(name, callback) {
        if (name === 'settings.section') callback()
      },
      register(meta, render) {
        registered = { meta, render }
        return () => {}
      },
    },
    locale: { register: () => () => {}, bind: () => key => key },
    effect(run) {
      run()
    },
  }
  clientExports.apply(clientCtx)
  console.log(`[30] client settings section -> ${registered?.meta?.id ?? 'missing'}`)
  check('client 注册 settings.section', registered?.meta?.id === 'dsh-desktop-bridge')
  check('client 分区 label 可用', registered?.meta?.label() === 'nav')
  check('client 分区有渲染函数', typeof registered?.render === 'function')
  let clientDegraded = true
  try {
    clientExports.apply({})
    clientExports.apply({ slots: {} })
    clientExports.apply({ slots: { inject() {} } })
  } catch {
    clientDegraded = false
  }
  check('client 无 slots 静默降级', clientDegraded)
  let pageDegraded = true
  try {
    pageDegraded = registerPairingPage({}, {}) === false
  } catch {
    pageDegraded = false
  }
  check('无 webServer 时配对路由静默降级', pageDegraded)

  // 31. 停机后不可达
  await bridge.stop()
  let refused = false
  try {
    await request(base, '/health')
  } catch {
    refused = true
  }
  console.log(`[26] GET /health after stop -> ${refused ? 'connection refused' : 'still reachable'}`)
  check('停机后不可达', refused)

  // 27. 端口占用时明确失败
  const holder = createBridge({ ctx: main.ctx, config: { port: 0 } })
  await holder.start()
  const contested = createBridge({ ctx: main.ctx, config: { port: holder.currentPort() } })
  let conflict = null
  try {
    await contested.start()
  } catch (error) {
    conflict = error
  }
  console.log(`[27] second bridge on port ${holder.currentPort()} -> ${conflict ? 'rejected' : 'started'}`)
  check('端口占用明确失败', conflict instanceof Error && /already in use/.test(conflict.message))
  await holder.stop()

  // 28. 插件 apply 注册 effect，disposer 关闭服务
  const applyFixture = makeFakeCtx()
  let cleanup = null
  applyFixture.ctx.effect = run => {
    cleanup = run()
    return () => {}
  }
  applyFixture.ctx.inject = () => {}
  await apply(applyFixture.ctx, { port: 0 })
  const listening = applyFixture.logs.join('\n').match(/listening on 127\.0\.0\.1:(\d+)/)
  check('apply 输出监听日志', listening !== null)
  const applyBase = `http://127.0.0.1:${listening?.[1]}`
  const applyHealth = await request(applyBase, '/health')
  console.log(`[28] apply -> GET /health ${applyHealth.status}`)
  check('apply 后 health 200', applyHealth.status === 200)
  await cleanup?.()
  let applyRefused = false
  try {
    await request(applyBase, '/health')
  } catch {
    applyRefused = true
  }
  check('apply disposer 关闭服务', applyRefused)

  // 29. taskStats：任务单位=可见会话；running job 计入 runCount，直接用户消息清未读，待决计入未读。
  const 启动 = async ctx => {
    const bridge = createBridge({ ctx, config: { port: 0 } })
    await bridge.start()
    const base = `http://127.0.0.1:${bridge.currentPort()}`
    const pair = await pairRequest(base, bridge.currentPairingCode())
    return { bridge, base, auth: { authorization: `Bearer ${pair.body.token}` } }
  }
  const 契约 = stats =>
    Number.isSafeInteger(stats?.runCount) && stats.runCount >= 0
    && Number.isSafeInteger(stats?.unreadCount) && stats.unreadCount >= 0
    && Number.isSafeInteger(stats?.completedUnreadCount) && stats.completedUnreadCount >= 0
    && Number.isSafeInteger(stats?.unionCount) && stats.unionCount >= 0
    && stats.completedUnreadCount <= stats.unreadCount
    && stats.unreadCount <= stats.unionCount
    && stats.unionCount <= stats.runCount + stats.unreadCount

  const 基础 = await 启动(makeFakeCtx().ctx)
  const 基础快照 = await request(基础.base, '/snapshot', { headers: 基础.auth })
  const 基础统计 = 基础快照.body?.taskStats
  console.log(`[29] taskStats: ${JSON.stringify(基础统计)}`)
  check('快照产出 taskStats', 基础统计 !== undefined && 基础统计 !== null)
  check('taskStats 计数满足插件契约', 契约(基础统计) === true)
  check('running job 的会话计入 runCount', 基础统计?.runCount === 1)
  check('无用户消息且无完成事件时未读为 0', 基础统计?.unreadCount === 0 && 基础统计?.completedUnreadCount === 0)
  check('taskStats completeness=complete', 基础统计?.completeness === 'complete')
  check('sampledAt 为 ISO 字符串', typeof 基础统计?.sampledAt === 'string' && !Number.isNaN(Date.parse(基础统计.sampledAt)))
  const 采样差 = Math.abs(Date.parse(基础统计?.sampledAt ?? '') - Date.parse(基础快照.body?.fetchedAt ?? ''))
  check('sampledAt 与 fetchedAt 相差 ≤ 30s', Number.isFinite(采样差) && 采样差 <= 30000)

  // 直接用户消息清未读；turn/end 计入完成未读；再来一条用户消息又清掉。
  基础.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'user/message', time: 1000, data: { source: { kind: 'user' } } })
  基础.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'turn/end', time: 2000, data: { reason: 'completed' } })
  const 完成后 = (await request(基础.base, '/snapshot', { headers: 基础.auth })).body?.taskStats
  console.log(`[29] after turn/end: ${JSON.stringify(完成后)}`)
  check('turn/end 计入未读', 完成后?.unreadCount === 1)
  check('turn/end 计入完成未读', 完成后?.completedUnreadCount === 1)
  check('union 覆盖未读会话', 完成后?.unionCount === 1)
  基础.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'user/message', time: 3000, data: { source: { kind: 'user' } } })
  const 已读后 = (await request(基础.base, '/snapshot', { headers: 基础.auth })).body?.taskStats
  console.log(`[29] after user message: ${JSON.stringify(已读后)}`)
  check('直接用户消息清未读', 已读后?.unreadCount === 0 && 已读后?.completedUnreadCount === 0)
  // 合成注入不算用户回到会话：不清未读。
  基础.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'turn/end', time: 4000, data: { reason: 'completed' } })
  基础.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'user/message', time: 5000, data: { source: { kind: 'cron' } } })
  const 合成后 = (await request(基础.base, '/snapshot', { headers: 基础.auth })).body?.taskStats
  check('合成注入不清未读', 合成后?.unreadCount === 1)
  // 待决到达计入未读（无完成事件）。
  基础.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'user/message', time: 6000, data: { source: { kind: 'user' } } })
  基础.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'approval/asked', time: 7000, data: { id: 'ap-9', toolName: 'bash', callId: 'call-9' } })
  const 待决后 = (await request(基础.base, '/snapshot', { headers: 基础.auth })).body?.taskStats
  console.log(`[29] after approval: ${JSON.stringify(待决后)}`)
  check('待决计入未读', 待决后?.unreadCount === 1)
  check('待决不计入完成未读', 待决后?.completedUnreadCount === 0)
  check('待决场景仍满足契约', 契约(待决后) === true)
  await 基础.bridge.stop()

  // 无会话来源：省略 taskStats，不造 0 冒充“没有任务”。
  const 无源 = makeFakeCtx()
  delete 无源.ctx.sessions
  const 无源桥 = await 启动(无源.ctx)
  const 无源快照 = (await request(无源桥.base, '/snapshot', { headers: 无源桥.auth })).body
  console.log(`[29] no sessions source -> taskStats=${JSON.stringify(无源快照?.taskStats)}`)
  check('无会话来源时省略 taskStats', 无源快照?.taskStats === undefined)
  await 无源桥.bridge.stop()

  // 空会话列表：真实的全 0，completeness 仍是 complete。
  const 空会话 = makeFakeCtx()
  空会话.ctx.sessions.list = () => []
  空会话.ctx.agents.list = () => []
  空会话.ctx.jobs.list = () => []
  const 空桥 = await 启动(空会话.ctx)
  const 空统计 = (await request(空桥.base, '/snapshot', { headers: 空桥.auth })).body?.taskStats
  console.log(`[29] empty sessions: ${JSON.stringify(空统计)}`)
  check('无会话时四项计数为 0', 空统计?.runCount === 0 && 空统计?.unreadCount === 0 && 空统计?.completedUnreadCount === 0 && 空统计?.unionCount === 0)
  check('空会话仍标 complete', 空统计?.completeness === 'complete')
  await 空桥.bridge.stop()

  // 陈旧：job 读取失败 → stale，completeness=partial（插件据此降级）。
  const 陈旧 = makeFakeCtx()
  陈旧.ctx.jobs.list = () => {
    throw new Error('jobs-down')
  }
  const 陈旧桥 = await 启动(陈旧.ctx)
  const 陈旧快照 = (await request(陈旧桥.base, '/snapshot', { headers: 陈旧桥.auth })).body
  console.log(`[29] stale -> taskStats=${JSON.stringify(陈旧快照?.taskStats)}`)
  check('数据陈旧时 taskStats 标 partial', 陈旧快照?.taskStats?.completeness === 'partial')
  check('陈旧但仍满足计数契约', 契约(陈旧快照?.taskStats) === true)
  await 陈旧桥.bridge.stop()

  // 30. taskStats 边界：17 会话扫描、多 job 去重、迟到水位取 max、revision 稳定。
  const 多会话 = makeFakeCtx()
  // 前 16 个 idle 会话按 id 排序在前；第 17 个用 "zz-late" 保证落在扫描末尾。
  多会话.ctx.sessions.list = () => [
    ...Array.from({ length: 16 }, (_, index) => ({ header: { id: `s${String(index + 1).padStart(2, '0')}` } })),
    { header: { id: 'zz-late' } },
  ]
  多会话.ctx.agents.list = () => []
  多会话.ctx.jobs.list = sessionId => {
    if (sessionId === undefined) return []
    if (sessionId === 's01') return [
      { id: 'job-a', label: 'a', status: 'running' },
      { id: 'job-b', label: 'b', status: 'running' },
    ]
    if (sessionId === 'zz-late') return [{ id: 'job-17', label: 'late', status: 'running' }]
    return []
  }
  const 多桥 = await 启动(多会话.ctx)
  const 多统计 = (await request(多桥.base, '/snapshot', { headers: 多桥.auth })).body?.taskStats
  console.log(`[30] 17 sessions: ${JSON.stringify(多统计)}`)
  check('第 17 个会话的 running job 计入 runCount', 多统计?.runCount === 2)
  check('第 17 个会话计入 unionCount', 多统计?.unionCount === 2)
  check('同一会话多个 running job 只计一次', 多统计?.runCount === 2 && 契约(多统计) === true)
  // revision 稳定：没有变化时连续两次快照 revision 不变（sampledAt 不进 digest）。
  const 稳定一 = (await request(多桥.base, '/snapshot', { headers: 多桥.auth })).body
  await new Promise(resolve => setTimeout(resolve, 10)) // 让两次采样至少跨 1ms（ISO 时间戳精度）。
  const 稳定二 = (await request(多桥.base, '/snapshot', { headers: 多桥.auth })).body
  console.log(`[30] revision ${稳定一?.revision} -> ${稳定二?.revision}；sampledAt ${稳定一?.taskStats?.sampledAt} -> ${稳定二?.taskStats?.sampledAt}`)
  check('sampledAt 不进 digest：无变化 revision 不变', 稳定一?.revision === 稳定二?.revision)
  check('两次采样时间确实不同（真实比较）', typeof 稳定一?.taskStats?.sampledAt === 'string' && typeof 稳定二?.taskStats?.sampledAt === 'string' && 稳定一.taskStats.sampledAt !== 稳定二.taskStats.sampledAt)
  await 多桥.bridge.stop()

  // 迟到水位：user t100 → turn/end t200 → 迟到 approval t90，必须 completedUnread=1 且 unread=1。
  const 迟到 = makeFakeCtx()
  迟到.ctx.sessions.list = () => [{ header: { id: 's1', createdAt: 1 } }]
  迟到.ctx.agents.list = () => []
  迟到.ctx.jobs.list = () => []
  const 迟到桥 = await 启动(迟到.ctx)
  迟到桥.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'user/message', time: 100, data: { source: { kind: 'user' } } })
  迟到桥.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'turn/end', time: 200, data: { reason: 'completed' } })
  迟到桥.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'approval/asked', time: 90, data: { id: 'ap-late', toolName: 'bash', callId: 'call-late' } })
  const 迟到统计 = (await request(迟到桥.base, '/snapshot', { headers: 迟到桥.auth })).body?.taskStats
  console.log(`[30] late approval: ${JSON.stringify(迟到统计)}`)
  check('迟到审批不回拨水位', 迟到统计?.unreadCount === 1)
  check('迟到审批下 completedUnread 仍 ⊆ unread', 迟到统计?.completedUnreadCount === 1 && 契约(迟到统计) === true)
  // 早于 cutoff 的完成事件不得产生完成未读。
  迟到桥.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'user/message', time: 300, data: { source: { kind: 'user' } } })
  迟到桥.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'job/settled', time: 250, data: { jobId: 'job-late', status: 'completed' } })
  const 早完成统计 = (await request(迟到桥.base, '/snapshot', { headers: 迟到桥.auth })).body?.taskStats
  console.log(`[30] early finish: ${JSON.stringify(早完成统计)}`)
  check('早于 cutoff 的完成事件不产生完成未读', 早完成统计?.completedUnreadCount === 0 && 早完成统计?.unreadCount === 0)
  await 迟到桥.bridge.stop()

  // 有界缓存淘汰：填 300 个会话让最早的被淘汰，快照仍有界且计数契约不变。
  // （时间表与 job 归属的清理在 forgetSessionFacts 里，属内部状态；这里验证淘汰后行为正确。）
  const 淘汰 = makeFakeCtx()
  淘汰.ctx.sessions.list = () => []
  淘汰.ctx.agents.list = () => []
  淘汰.ctx.jobs.list = () => []
  const 淘汰桥 = await 启动(淘汰.ctx)
  for (let index = 0; index < 300; index += 1) {
    淘汰桥.bridge.handleSessionEvent(
      { header: { id: `e${String(index).padStart(3, '0')}` } },
      { type: 'session/title', data: { title: `T${index}` } },
    )
  }
  const 淘汰快照 = (await request(淘汰桥.base, '/snapshot', { headers: 淘汰桥.auth })).body
  console.log(`[30] evicted cache: sessions=${淘汰快照?.sessions?.length} taskStats=${JSON.stringify(淘汰快照?.taskStats)}`)
  check('缓存淘汰后快照仍有界', Array.isArray(淘汰快照?.sessions) && 淘汰快照.sessions.length <= 64)
  check('缓存淘汰后 taskStats 仍满足契约', 契约(淘汰快照?.taskStats) === true)
  check('缓存淘汰后计数不误报运行', 淘汰快照?.taskStats?.runCount === 0)
  await 淘汰桥.bridge.stop()

  // 31. 反例：sessions.list 仍提供的会话不被 pruneSessions 清水位；job 由 running 转 completed 后 runCount 归零。
  const churn = makeFakeCtx()
  churn.ctx.sessions.list = () => [{ header: { id: 'a-live', createdAt: 1 } }]
  churn.ctx.agents.list = () => []
  churn.ctx.jobs.list = () => []
  const churnBridge = await 启动(churn.ctx)
  churnBridge.bridge.handleSessionEvent({ header: { id: 'a-live' } }, { type: 'user/message', time: 100, data: { source: { kind: 'user' } } })
  churnBridge.bridge.handleSessionEvent({ header: { id: 'a-live' } }, { type: 'turn/end', time: 200, data: { reason: 'completed' } })
  const churn前 = (await request(churnBridge.base, '/snapshot', { headers: churnBridge.auth })).body?.taskStats
  check('a-live 初始完成未读=1', churn前?.unreadCount === 1 && churn前?.completedUnreadCount === 1)
  // 300 个其他会话发标题，推动事件缓存超过 SESSION_KEEP 触发 pruneSessions。
  for (let index = 0; index < 300; index += 1) {
    churnBridge.bridge.handleSessionEvent(
      { header: { id: `z${String(index).padStart(3, '0')}` } },
      { type: 'session/title', data: { title: `Z${index}` } },
    )
  }
  const churn后 = (await request(churnBridge.base, '/snapshot', { headers: churnBridge.auth })).body?.taskStats
  console.log(`[31] after churn: ${JSON.stringify(churn后)}`)
  check('300 会话 churn 后 a-live 仍可见', (await request(churnBridge.base, '/snapshot', { headers: churnBridge.auth })).body?.sessions?.some(row => row.id === 'a-live') === true)
  check('churn 不误删 sessions.list 仍提供会话的水位', churn后?.completedUnreadCount === 1 && churn后?.unreadCount === 1)
  check('churn 后 completeness 仍 complete', churn后?.completeness === 'complete')
  await churnBridge.bridge.stop()

  // job 状态转换：全局读 running，会话读 completed → 最终 done，runCount 必须归零。
  const 转换 = makeFakeCtx()
  转换.ctx.sessions.list = () => [{ header: { id: 's1', createdAt: 1 } }]
  转换.ctx.agents.list = () => []
  转换.ctx.jobs.list = sessionId => sessionId === undefined
    ? [{ id: 'job-J', label: 'J', status: 'running' }]
    : [{ id: 'job-J', label: 'J', status: 'completed' }]
  const 转换桥 = await 启动(转换.ctx)
  const 转换快照 = (await request(转换桥.base, '/snapshot', { headers: 转换桥.auth })).body
  console.log(`[31] running->completed: taskStats=${JSON.stringify(转换快照?.taskStats)} jobs=${JSON.stringify(转换快照?.jobs)}`)
  check('会话读到 completed 后 job 显示 done', 转换快照?.jobs?.some(row => row.id === 'job-J' && row.status === 'done') === true)
  check('running→completed 后 runCount 归零', 转换快照?.taskStats?.runCount === 0)
  check('转换后 unionCount 归零', 转换快照?.taskStats?.unionCount === 0)
  await 转换桥.bridge.stop()

  // 32. 反例：33 会话计数不截断、registered 后 done 不加回、zz-live 挤出再入榜、list 抛错不清水位。
  const 多 = makeFakeCtx()
  多.ctx.sessions.list = () => Array.from({ length: 33 }, (_, index) => ({ header: { id: `c${String(index).padStart(2, '0')}`, createdAt: index } }))
  多.ctx.agents.list = () => []
  多.ctx.jobs.list = sessionId => {
    if (sessionId === undefined) return []
    return [{ id: `job-${sessionId}`, label: sessionId, status: 'running' }]
  }
  const 多桥2 = await 启动(多.ctx)
  const 多快照 = (await request(多桥2.base, '/snapshot', { headers: 多桥2.auth })).body
  console.log(`[32] 33 sessions: run=${多快照?.taskStats?.runCount} jobs=${多快照?.jobs?.length}`)
  check('33 会话各 1 running → runCount=33（计数不截断）', 多快照?.taskStats?.runCount === 33)
  check('unionCount=33', 多快照?.taskStats?.unionCount === 33)
  check('展示仍按 32 截断', 多快照?.jobs?.length <= 32)
  check('33 会话仍满足契约', 契约(多快照?.taskStats) === true)
  await 多桥2.bridge.stop()

  // registered 后会话读 done：旧事件不得把 done 加回 running。
  const 加回 = makeFakeCtx()
  加回.ctx.sessions.list = () => [{ header: { id: 's1', createdAt: 1 } }]
  加回.ctx.agents.list = () => []
  加回.ctx.jobs.list = sessionId => sessionId === undefined
    ? []
    : [{ id: 'job-J', label: 'J', status: 'completed' }]
  const 加回桥 = await 启动(加回.ctx)
  加回桥.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'job/registered', data: { jobId: 'job-J', label: 'J' } })
  const 加回快照 = (await request(加回桥.base, '/snapshot', { headers: 加回桥.auth })).body
  console.log(`[32] registered->done: taskStats=${JSON.stringify(加回快照?.taskStats)} jobs=${JSON.stringify(加回快照?.jobs)}`)
  check('registered 后最终 done 的 job 显示 done', 加回快照?.jobs?.some(row => row.id === 'job-J' && row.status === 'done') === true)
  check('done 不被旧 eventJobs 加回 running', 加回快照?.taskStats?.runCount === 0 && 加回快照?.taskStats?.unionCount === 0)
  await 加回桥.bridge.stop()

  // zz-live：先未读 1，300 标题 churn 把它挤出事件缓存与展示 64，重新入榜后水位仍在。
  const 挤出 = makeFakeCtx()
  const 挤出源 = Array.from({ length: 70 }, (_, index) => ({ header: { id: `d${String(index).padStart(2, '0')}`, createdAt: index } }))
  挤出源.push({ header: { id: 'zz-live', createdAt: 999 } })
  挤出.ctx.sessions.list = () => 挤出源
  挤出.ctx.agents.list = () => []
  挤出.ctx.jobs.list = () => []
  const 挤出桥 = await 启动(挤出.ctx)
  挤出桥.bridge.handleSessionEvent({ header: { id: 'zz-live' } }, { type: 'user/message', time: 100, data: { source: { kind: 'user' } } })
  挤出桥.bridge.handleSessionEvent({ header: { id: 'zz-live' } }, { type: 'turn/end', time: 200, data: { reason: 'completed' } })
  const 挤出前 = (await request(挤出桥.base, '/snapshot', { headers: 挤出桥.auth })).body
  // 计数跟随「可见会话」（展示 ≤64）：zz-live 排在第 65 位时不计入，但水位必须保留下来。
  check('zz-live 在展示外时不计入未读（计数跟随可见会话）', 挤出前?.taskStats?.unreadCount === 0)
  check('zz-live 初始不在展示 64 内', (挤出前?.sessions ?? []).every(row => row.id !== 'zz-live'))
  for (let index = 0; index < 300; index += 1) {
    挤出桥.bridge.handleSessionEvent({ header: { id: `y${String(index).padStart(3, '0')}` } }, { type: 'session/title', data: { title: `Y${index}` } })
  }
  const 挤出后 = (await request(挤出桥.base, '/snapshot', { headers: 挤出桥.auth })).body
  console.log(`[32] zz-live after churn: ${JSON.stringify(挤出后?.taskStats)}`)
  check('churn 后 zz-live 仍在展示外、水位不丢', 挤出后?.taskStats?.unreadCount === 0)
  // 重新入榜：churn 后事件缓存仍占着展示名额，用一条活待决把 zz-live 顶回可见行，
  // 若水位被误删，completedUnread 会是 0（approval 只写未读、不写完成）。
  挤出桥.bridge.handleSessionEvent({ header: { id: 'zz-live' } }, { type: 'approval/asked', time: 900, data: { id: 'ap-zz', toolName: 'bash', callId: 'call-zz' } })
  const 重入 = (await request(挤出桥.base, '/snapshot', { headers: 挤出桥.auth })).body
  console.log(`[32] zz-live back: ${JSON.stringify(重入?.taskStats)}`)
  check('zz-live 重新入榜仍可见', (重入?.sessions ?? []).some(row => row.id === 'zz-live') === true)
  check('重新入榜后完成未读仍为 1（水位未被展示截断删除）', 重入?.taskStats?.completedUnreadCount === 1)
  check('重新入榜后未读=1', 重入?.taskStats?.unreadCount === 1)
  await 挤出桥.bridge.stop()

  // sessions.list 抛错：不得清水位；恢复后仍未读=1。
  const 抛错 = makeFakeCtx()
  let 抛错中 = false
  抛错.ctx.sessions.list = () => {
    if (抛错中) throw new Error('sessions-down')
    return [{ header: { id: 's1', createdAt: 1 } }]
  }
  抛错.ctx.agents.list = () => []
  抛错.ctx.jobs.list = () => []
  const 抛错桥 = await 启动(抛错.ctx)
  抛错桥.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'user/message', time: 100, data: { source: { kind: 'user' } } })
  抛错桥.bridge.handleSessionEvent({ header: { id: 's1' } }, { type: 'turn/end', time: 200, data: { reason: 'completed' } })
  const 抛错前 = (await request(抛错桥.base, '/snapshot', { headers: 抛错桥.auth })).body?.taskStats
  check('抛错前未读=1', 抛错前?.unreadCount === 1)
  抛错中 = true
  const 抛错快照 = (await request(抛错桥.base, '/snapshot', { headers: 抛错桥.auth })).body
  check('list 抛错时快照标 stale', 抛错快照?.stale === true)
  抛错中 = false
  const 抛错后 = (await request(抛错桥.base, '/snapshot', { headers: 抛错桥.auth })).body?.taskStats
  console.log(`[32] list error then recover: ${JSON.stringify(抛错后)}`)
  check('list 抛错不得清水位（恢复后仍未读=1）', 抛错后?.unreadCount === 1 && 抛错后?.completedUnreadCount === 1)
  await 抛错桥.bridge.stop()

  console.log(`\n通过 ${passed} 项，失败 ${failed} 项`)
  if (failed > 0) process.exitCode = 1
}

main().catch(error => {
  console.error('[FAIL] 自测异常：', error)
  process.exitCode = 1
})
