/**
 * dsh-desktop-bridge 浏览器半边：在 DSH 设置页注册一个最小分区。
 *
 * 只读：读取同源 /dsh-bridge/status 显示已配对/未配对与 bridgeId 前 8 位；
 * 「打开配对页」在当前窗口跳转 /dsh-bridge/pairing。分区不可用时静默降级，
 * 不影响宿主桥。
 *
 * 该文件按 DSH client bundle 约定由 window.__ModuleLoader__ 加载。
 */
;(function (root) {
  if (root === null || root === undefined || !root.__ModuleLoader__ || typeof root.__ModuleLoader__.load !== 'function') return
  root.__ModuleLoader__.load({
  id: 'dsh-desktop-bridge',
  factory: function (require) {
    var module = { exports: {} }
    var exports = module.exports
    var React = null
    try {
      React = require('react')
    } catch (error) {
      React = null
    }
    var NS = 'dsh-desktop-bridge'
    var zh = {
      nav: 'DSH 桌面桥',
      title: 'DSH 桌面桥',
      blurb: 'DesktopTools 通过本机回环桥读取 DSH 的余额、会话与任务状态；配对码只在本页显示。',
      loading: '正在读取桥状态…',
      unavailable: '桥状态不可用（插件可能未启用）。',
      paired: '已配对',
      unpaired: '未配对',
      bridgeId: '桥身份',
      code: '配对码',
      noCode: '生成中',
      rotate: '换一个配对码',
      revoke: '撤销配对',
      expiresIn: '剩余',
      hint: '把配对码填进 Desktop Tools「设置 → 插件 → DSH 接入」。配对码只在 DSH 窗口内可见，不写日志。',
    }
    var en = {
      nav: 'DSH Desktop Bridge',
      title: 'DSH Desktop Bridge',
      blurb: 'DesktopTools reads balances, sessions and jobs from DSH through a loopback bridge. The pairing code is shown only on this page.',
      loading: 'Reading bridge status…',
      unavailable: 'Bridge status unavailable (plugin may be disabled).',
      paired: 'Paired',
      unpaired: 'Not paired',
      bridgeId: 'Bridge ID',
      code: 'Pairing code',
      noCode: 'generating',
      rotate: 'New pairing code',
      revoke: 'Revoke pairing',
      expiresIn: 'expires in',
      hint: 'Enter the code in Desktop Tools → Settings → Plugins → DSH 接入. The code is visible only inside the DSH window and is never logged.',
    }
    var PAIRING_URL = '/dsh-bridge/pairing.json'

    /**
     * 设置分区视图：轮询同源状态，提供打开配对页按钮。
     * @param {{ t: (key: string) => string }} props 注入属性。
     * @returns {any} React 元素。
     */
    function BridgeSection(props) {
      var t = props.t
      var stateHook = React.useState({ loading: true, ok: false, paired: false, bridgeId: '', code: '', expiresAt: 0, csrf: '', error: '' })
      var state = stateHook[0]
      var setState = stateHook[1]
      var busyHook = React.useState(false)
      var busy = busyHook[0]
      var setBusy = busyHook[1]
      React.useEffect(function () {
        var alive = true
        var load = function () {
          fetch(PAIRING_URL, { credentials: 'same-origin', cache: 'no-store' })
            .then(function (response) {
              if (!response.ok) throw new Error('pairing-' + response.status)
              return response.json()
            })
            .then(function (data) {
              if (alive) setState({ loading: false, ok: true, paired: data.paired === true, bridgeId: String(data.bridgeId || ''), code: String(data.code || ''), expiresAt: Number(data.expiresAt || 0), csrf: String(data.csrf || ''), error: '' })
            })
            .catch(function () {
              if (alive) setState({ loading: false, ok: false, paired: false, bridgeId: '', code: '', expiresAt: 0, csrf: '', error: '' })
            })
        }
        load()
        var timer = setInterval(load, 4000)
        return function () {
          alive = false
          clearInterval(timer)
        }
      }, [])
      var act = function (action) {
        if (busy) return
        setBusy(true)
        fetch(PAIRING_URL, {
          method: 'POST',
          credentials: 'same-origin',
          cache: 'no-store',
          headers: { 'content-type': 'application/json' },
          body: JSON.stringify({ action: action, csrf: state.csrf }),
        })
          .then(function (response) { return response.json() })
          .then(function (data) {
            if (data && data.ok === true) {
              setState({ loading: false, ok: true, paired: data.paired === true, bridgeId: String(data.bridgeId || ''), code: String(data.code || ''), expiresAt: Number(data.expiresAt || 0), csrf: String(data.csrf || ''), error: '' })
            } else {
              setState({ loading: false, ok: false, paired: state.paired, bridgeId: state.bridgeId, code: '', expiresAt: 0, csrf: state.csrf, error: String((data && data.error) || 'action-failed') })
            }
          })
          .catch(function () {
            setState({ loading: false, ok: false, paired: state.paired, bridgeId: state.bridgeId, code: '', expiresAt: 0, csrf: state.csrf, error: 'network' })
          })
          .then(function () { setBusy(false) })
      }
      var remaining = state.expiresAt > 0 ? Math.max(0, Math.round((state.expiresAt - Date.now()) / 1000)) : 0
      var rows = []
      rows.push(React.createElement('strong', { key: 'title' }, t('title')))
      rows.push(React.createElement('p', { key: 'blurb', style: { margin: 0 } }, t('blurb')))
      if (state.loading) {
        rows.push(React.createElement('p', { key: 'loading', style: { margin: 0 } }, t('loading')))
      } else if (state.ok !== true) {
        rows.push(React.createElement('p', { key: 'unavailable', style: { margin: 0 } }, t('unavailable') + (state.error ? '（' + state.error + '）' : '')))
      } else if (state.paired === true) {
        rows.push(React.createElement('p', { key: 'paired', style: { margin: 0 } }, t('paired') + ' · ' + t('bridgeId') + '：' + (state.bridgeId ? state.bridgeId.slice(0, 8) : '—')))
        rows.push(React.createElement('button', { key: 'revoke', type: 'button', disabled: busy, onClick: function () { act('revoke') } }, t('revoke')))
      } else {
        var expiry = remaining > 0 ? '（' + t('expiresIn') + ' ' + Math.floor(remaining / 60) + ':' + ('0' + (remaining % 60)).slice(-2) + '）' : ''
        rows.push(React.createElement('p', { key: 'code', style: { margin: 0 } }, t('code') + '：' + (state.code ? state.code : t('noCode')) + expiry))
        rows.push(React.createElement('button', { key: 'rotate', type: 'button', disabled: busy, onClick: function () { act('rotate') } }, t('rotate')))
      }
      rows.push(React.createElement('p', { key: 'hint', style: { margin: 0, color: 'var(--dsw-alias-label-tertiary)' } }, t('hint')))
      return React.createElement('div', { style: { display: 'flex', flexDirection: 'column', gap: '10px', maxWidth: '520px' } }, rows)
    }

    /**
     * 客户端插件入口：注册 settings.section。
     * @param {any} ctx 客户端上下文。
     * @returns {void}
     */
    function apply(ctx) {
      try {
        if (React === null || React === undefined || typeof React.createElement !== 'function') return
        if (!ctx || !ctx.slots || typeof ctx.slots.inject !== 'function') return
        var t = function (key) { return key }
        if (ctx.locale && typeof ctx.locale.register === 'function' && typeof ctx.locale.bind === 'function' && typeof ctx.effect === 'function') {
          ctx.effect(function () {
            return ctx.locale.register(NS, { zh: zh, en: en })
          }, 'dsh-desktop-bridge: settings dictionaries')
          t = ctx.locale.bind(NS)
        }
        ctx.slots.inject('settings.section', function () {
          return ctx.slots.register({
            name: 'settings.section',
            id: 'dsh-desktop-bridge',
            order: 60,
            label: function () { return t('nav') },
            locale: NS,
            inject: function () { return { t: t } },
          }, function () {
            return React.createElement(BridgeSection, { t: t })
          })
        })
      } catch (error) {
        try {
          console.warn('[dsh-desktop-bridge] settings section unavailable', error)
        } catch (ignored) {
          // 没有 console 的宿主保持静默。
        }
      }
    }

    exports.inject = ['locale', 'slots']
    exports.apply = apply
    return module.exports
  },
  })
})(typeof window !== 'undefined' ? window : typeof self !== 'undefined' ? self : null)
