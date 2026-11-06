# -*- coding: utf-8 -*-
"""AstrBot 侧状态采集插件（契约 v1）。

- 在 AstrBot 进程内采集服务/平台/资源/消息/日志/Token 用量，按 `契约/snapshot.schema.json` 组装快照，
  周期 POST 到本机接收端（`push_url` 端口 0 表示不推）；
- 可选开一个只监听 `127.0.0.1` 的小 HTTP 口：`GET /v1/snapshot` 拉取兜底、`POST /v1/actions` 启停平台。

约定：时间戳一律 Unix 秒；拿不到的项给 null（不用 0 / "" / [] 冒充）；每段采集独立兜底，
失败置 null，绝不抛到 AstrBot 主循环；不读不记任何 AstrBot 凭据，不调 `/api/v1/system-config`；
日志不输出 token，比较 token 用 `secrets.compare_digest`。

线程模型（只有一种桥接写法）：`_loop` 是 AstrBot 的长期事件循环，由 `on_astrbot_loaded()` 绑定。
HTTP 线程要用异步能力时一律 `asyncio.run_coroutine_threadsafe(coro, self._loop).result(timeout=…)`；
采集线程已经在 `_loop` 线程里时，**不**回投事件循环（回投会自己等自己），直接跑同步查询。
"""

from __future__ import annotations

import asyncio
import collections
import copy
import json
import logging
import math
import os
import secrets
import socket
import threading
import time
import urllib.error
import urllib.request
from datetime import datetime, timedelta
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from typing import Any

from astrbot.api import AstrBotConfig, logger
from astrbot.api.event import filter
from astrbot.api.star import Context, Star

SCHEMA_VERSION = 1
MAX_PAYLOAD_BYTES = 1024 * 1024
LOG_TEXT_LIMIT = 500
ERROR_MESSAGE_LIMIT = 500
LOG_LEVELS = ("DEBUG", "INFO", "WARNING", "ERROR", "CRITICAL")
# 契约 §4.3：只允许这几种状态，未识别的映射为 unknown
PLATFORM_STATUSES = ("running", "error", "stopped")
MESSAGES_WINDOW_SEC = 24 * 3600
SERIES_POINTS = 24
DEFAULT_INTERVAL_SEC = 5
DEFAULT_KEYWORDS = ["Traceback", "ERROR", "Exception", "failed"]
# 桥接超时：采集一次最多等这么久，HTTP 线程动作等这么久；都远小于实测 394s 的来源
COLLECT_TIMEOUT_SEC = 10.0
ACTION_TIMEOUT_SEC = 15.0


# ---------------------------------------------------------------------------
# 纯函数：清洗与换算（selfcheck 直接覆盖）
# ---------------------------------------------------------------------------


def clean_text(value: Any, limit: int = ERROR_MESSAGE_LIMIT) -> str | None:
    """任意值转成截断后的字符串；空值给 None。"""
    if value is None:
        return None
    text = value if isinstance(value, str) else str(value)
    return text[:limit]


def to_int(value: Any) -> int | None:
    """宽松转 int（排除 bool）；转不了给 None，不给 0。"""
    if value is None or isinstance(value, bool):
        return None
    if isinstance(value, int):
        return value
    if isinstance(value, float):
        return int(value) if math.isfinite(value) else None
    if isinstance(value, str) and value.strip().lstrip("-").isdigit():
        return int(value.strip())
    return None


def to_float(value: Any, digits: int = 1) -> float | None:
    """宽松转 float 并保留小数位；转不了或非有限值给 None。"""
    if value is None or isinstance(value, bool):
        return None
    try:
        number = float(value)
    except (TypeError, ValueError):
        return None
    return round(number, digits) if math.isfinite(number) else None


def unix_sec(value: Any) -> int | None:
    """ISO 字符串 / epoch 数 / datetime 统一转 Unix 秒；不认识或明显是脏数据的给 None。"""
    if isinstance(value, datetime):
        result: int | None = int(value.timestamp())
    elif isinstance(value, (int, float)) and not isinstance(value, bool):
        result = int(value) if math.isfinite(float(value)) else None
    elif isinstance(value, str) and value.strip():
        text = value.strip()
        if text.lstrip("-").isdigit():
            result = int(text)
        else:
            try:
                result = int(datetime.fromisoformat(text).timestamp())
            except ValueError:
                result = None
    else:
        result = None
    if result is None:
        return None
    if result > 10**11:  # 毫秒误传，压回秒
        result //= 1000
    return result if result > 10**9 else None


def normalize_status(value: Any) -> str | None:
    """平台状态映射：未识别的给 unknown，空值给 None，不新增契约外取值。"""
    if value is None:
        return None
    if isinstance(value, bool):
        return "unknown"
    text = str(value).strip().lower()
    if not text:
        return None
    return text if text in PLATFORM_STATUSES else "unknown"


def log_level_name(name: Any) -> str:
    """日志级别名压到契约允许的五个值。"""
    text = str(name or "").upper()
    if text in ("WARN", "WARNING"):
        return "WARNING"
    if text in ("FATAL", "CRITICAL"):
        return "CRITICAL"
    if text in ("DEBUG", "TRACE"):
        return "DEBUG"
    if text.startswith("ERROR") or text == "EXCEPTION":
        return "ERROR"
    return "INFO"


def log_tail_capacity(value: Any) -> int:
    """log_tail 配置清洗：非正数按 200，上限 200（契约 C5）。"""
    number = to_int(value)
    if number is None or number <= 0:
        return 200
    return min(number, 200)


def match_keywords(text: str, keywords: list[str]) -> str | None:
    """返回第一个命中的告警关键词；未命中给 None。"""
    lowered = str(text).lower()
    for keyword in keywords:
        if keyword and str(keyword).lower() in lowered:
            return keyword
    return None


def series_from_rows(rows: Any, now: int) -> list[dict] | None:
    """[(ts, count), ...] 聚合成最近 24 个整点桶（ts 正序）；没有任何点给 None。"""
    buckets: dict[int, int] = {}
    for row in rows or []:
        try:
            ts_raw, count_raw = row[0], row[1]
        except (TypeError, IndexError, KeyError):
            continue
        ts_sec = to_int(ts_raw) if isinstance(ts_raw, (int, float)) else unix_sec(ts_raw)
        count = to_int(count_raw)
        if ts_sec is None or count is None:
            continue
        latest_bucket = now // 3600 * 3600
        bucket = ts_sec // 3600 * 3600
        if bucket > latest_bucket or bucket <= latest_bucket - SERIES_POINTS * 3600:
            continue
        buckets[bucket] = buckets.get(bucket, 0) + count
    if not buckets:
        return None
    return [{"ts": ts, "count": buckets[ts]} for ts in sorted(buckets)]


def cost_from_pricing(pricing: Any, providers: Any, output_tokens: int | None) -> dict:
    """按单价表折算金额（统一口径：输出 token × outputPerM / 1e6）。

    没有可用单价、或 `rangeOutputTokens` 为 null 时，total/currency 必须为 null 并写明 note。
    """
    no_price = {"total": None, "currency": None, "note": "未配置单价表"}
    if not isinstance(pricing, dict) or not pricing or not providers or output_tokens is None:
        return no_price
    total = 0.0
    currency: str | None = None
    matched = 0
    for item in providers:
        if not isinstance(item, dict):
            continue
        provider_id, tokens = item.get("providerId"), to_int(item.get("totalTokens"))
        rule = pricing.get(str(provider_id)) if provider_id else None
        if tokens is None or not isinstance(rule, dict):
            continue
        try:
            per_million = float(rule.get("outputPerM") or 0)
        except (TypeError, ValueError):
            continue
        if per_million <= 0:
            continue
        total += tokens * per_million / 1_000_000
        currency = rule.get("currency") or currency
        matched += 1
    if not matched or currency is None:
        return no_price
    return {"total": round(total, 4), "currency": str(currency), "note": "按配置单价表折算（仅输出 token）"}


# ---------------------------------------------------------------------------
# 日志环形缓冲
# ---------------------------------------------------------------------------


class RingBufferHandler(logging.Handler):
    """只进内存的日志环形缓冲：限速、防递归、不落盘。"""

    def __init__(self, capacity: int) -> None:
        super().__init__(level=logging.DEBUG)
        self.capacity = log_tail_capacity(capacity)
        self._buffer: collections.deque = collections.deque(maxlen=self.capacity)
        self._lock = threading.Lock()
        self._local = threading.local()

    def emit(self, record: logging.LogRecord) -> None:
        if getattr(self._local, "busy", False):
            return
        self._local.busy = True
        try:
            text = record.getMessage()
            if record.exc_info and not record.exc_text:
                # 只留异常类型，防止一个 traceback 占满缓冲
                text = f"{text} [{getattr(record.exc_info[0], '__name__', 'Exception')}]"
            # 不修改 record.args：那是共享 LogRecord，改掉会让排在后面的 handler 丢掉参数
            entry = {"ts": int(record.created), "level": log_level_name(record.levelname), "text": text[:LOG_TEXT_LIMIT]}
            with self._lock:
                self._buffer.append(entry)
        except Exception:  # noqa: BLE001 - 日志处理器内绝不外抛
            pass
        finally:
            self._local.busy = False

    def snapshot(self) -> list[dict]:
        """按时间正序返回副本。"""
        with self._lock:
            return list(self._buffer)


# ---------------------------------------------------------------------------
# 只监听 loopback 的 HTTP 服务
# ---------------------------------------------------------------------------


class LoopbackHTTPServer(ThreadingHTTPServer):
    """只绑 IPv4 127.0.0.1：`localhost` 双栈解析会把 0.0.0.0 一起暴露。"""

    address_family = socket.AF_INET
    daemon_threads = True


def make_server(port: int, handler_cls: type) -> LoopbackHTTPServer:
    """在 127.0.0.1:port 上建服务；端口被占用时抛 OSError，由调用方兜住。"""
    return LoopbackHTTPServer(("127.0.0.1", int(port)), handler_cls)


# ---------------------------------------------------------------------------
# 插件主体
# ---------------------------------------------------------------------------


class DesktopStatusPlugin(Star):
    """AstrBot 状态采集插件。"""

    def __init__(self, context: Context, config: AstrBotConfig | None = None) -> None:
        super().__init__(context)
        self._raw_config = config
        self._enabled = True
        self._loop: asyncio.AbstractEventLoop | None = None
        self._task: asyncio.Task | None = None
        self._stop_asyncio = False
        self._closed = False        # terminate 之后不再复活 handler / 不再采集
        self._stopping = False      # 终止流程中，循环尽快早退
        self._last_loop_error: str | None = None
        self._last_collect_errors: tuple[str, ...] | None = None
        self._token = ""
        self._push_url = ""
        self._interval = float(DEFAULT_INTERVAL_SEC)
        self._listen_port = 0
        self._log_tail = 200
        self._keywords: list[str] = list(DEFAULT_KEYWORDS)
        self._pricing: dict = {}
        self._log_handler: RingBufferHandler | None = None
        self._attach_targets: list[logging.Logger] = []
        self._server: LoopbackHTTPServer | None = None
        self._server_thread: threading.Thread | None = None
        self._last_good: dict | None = None
        self._last_push_ok: bool | None = None
        self._instance_id: str | None = None
        self._process = None
        self._prime_cpu = True
        self._load_config()

    # -- 配置 ---------------------------------------------------------------

    @property
    def enabled(self) -> bool:
        """总开关。"""
        return self._enabled

    def _load_config(self) -> None:
        """读配置；越界或类型不对的项回落到契约默认值。"""
        conf = self._raw_config if isinstance(self._raw_config, dict) else {}

        raw_enabled = conf.get("enabled", True)
        self._enabled = str(raw_enabled).strip().lower() not in ("false", "0", "off", "no", "none", "")

        interval = to_int(conf.get("collect_interval_sec"))
        self._interval = float(interval) if interval and interval > 0 else float(DEFAULT_INTERVAL_SEC)

        push_url = conf.get("push_url")
        self._push_url = push_url.strip() if isinstance(push_url, str) else ""

        port = to_int(conf.get("listen_port"))
        self._listen_port = port if port and 0 < port < 65536 else 0

        token = conf.get("token")
        self._token = token.strip() if isinstance(token, str) else ""
        if not self._token:
            self._token = secrets.token_urlsafe(32)
            self._persist_token(self._token)

        new_capacity = log_tail_capacity(conf.get("log_tail"))
        if new_capacity != self._log_tail:
            self._log_tail = new_capacity
            # 容量变了：下次采集前按新容量重挂（_ensure_log_handler 会重建）
            self._detach_log_handler()

        keywords = conf.get("error_keywords")
        if isinstance(keywords, str):
            keywords = keywords.split(",")
        if not isinstance(keywords, list):
            keywords = list(DEFAULT_KEYWORDS)
        self._keywords = [str(item).strip() for item in keywords if str(item).strip()]

        self._pricing = self._parse_pricing(conf.get("pricing"))

    def _persist_token(self, token: str) -> None:
        """把新生成的 token 写回插件配置；日志只报结果，不打 token。"""
        conf = self._raw_config
        try:
            if isinstance(conf, dict):
                conf["token"] = token
                save = getattr(conf, "save_config", None)
                if callable(save):
                    save()
                    logger.info("[desktop_status] 已生成共享密钥并写回插件配置。")
                    return
                path = self._config_file_path(conf)
                if path is not None:
                    path.write_text(json.dumps(dict(conf), ensure_ascii=False, indent=2), encoding="utf-8")
                    logger.info("[desktop_status] 已生成共享密钥并写回插件配置。")
                    return
            logger.warning("[desktop_status] 共享密钥未能写回配置，AstrBot 重启后会重新生成。")
        except Exception as exc:  # noqa: BLE001
            logger.warning("[desktop_status] 共享密钥写回失败：%s", type(exc).__name__)

    @staticmethod
    def _config_file_path(conf: dict) -> Path | None:
        """AstrBotConfig 把配置文件路径存在 `_config_file_path`；普通 dict 没有。"""
        raw = conf.get("_config_file_path") or getattr(conf, "config_path", None)
        if not raw:
            return None
        path = Path(str(raw))
        return path if path.name.endswith(".json") else None

    @staticmethod
    def _parse_pricing(raw: Any) -> dict:
        """pricing 是 JSON 对象文本（也容忍已经是 dict）。

        04 §5.4 的键是「`<provider_id>/<model>`」；这里把每项在原键与 provider 段两处各登记一份，
        查表时 `providerId` 与 `providerId/model` 都能命中。
        """
        parsed: Any = None
        if isinstance(raw, dict):
            parsed = raw
        elif isinstance(raw, str) and raw.strip():
            try:
                parsed = json.loads(raw)
            except json.JSONDecodeError:
                logger.warning("[desktop_status] pricing 不是合法 JSON，按未配置单价处理。")
                return {}
        if not isinstance(parsed, dict):
            return {}
        table: dict = {}
        for key, rule in parsed.items():
            if not isinstance(rule, dict):
                continue
            table.setdefault(str(key), rule)
            table.setdefault(str(key).split("/", 1)[0], rule)
        return table

    # -- 生命周期 -----------------------------------------------------------

    @filter.on_astrbot_loaded()
    async def on_astrbot_loaded(self) -> None:
        """AstrBot 加载完成后再启动，避免早于 platform_manager 就绪。"""
        if not self._enabled:
            logger.info("[desktop_status] 插件已禁用，不启动采集。")
            return
        self._loop = asyncio.get_running_loop()
        self._stop_asyncio = False
        self._closed = False
        self._stopping = False
        self._attach_log_handler()
        try:
            self._start_server()
        except Exception as exc:  # noqa: BLE001
            logger.warning("[desktop_status] 反向端口启动失败（%s），采集继续。", type(exc).__name__)
        if self._task is None or self._task.done():
            self._task = self._loop.create_task(self._run_loop())
        logger.info(
            "[desktop_status] 已启动：周期 %.0fs，推送 %s，反向端口 %s。",
            self._interval,
            "开" if self._push_url else "关",
            self._listen_port or "关",
        )

    async def terminate(self) -> None:
        """停循环、关端口、摘 handler；可重复调用。"""
        self._stopping = True
        self._stop_asyncio = True
        task, self._task = self._task, None
        if task is not None and not task.done():
            task.cancel()
            try:
                # to_thread 里的阻塞调用不可取消；有界等待，别让 terminate 被慢查询拖死
                await asyncio.wait_for(asyncio.shield(task), timeout=5.0)
            except (asyncio.CancelledError, asyncio.TimeoutError, Exception):  # noqa: BLE001
                pass
        self._closed = True
        self._stopping = False
        await self._stop_server()
        self._detach_log_handler()
        logger.info("[desktop_status] 已停止。")

    # -- 主循环 -------------------------------------------------------------

    async def _run_loop(self) -> None:
        """采集 + 推送主循环；任何异常都在这里吞掉。"""
        while not self._stop_asyncio and not self._stopping:
            try:
                self._load_config()
                # 采集放到线程里跑：DB/psutil 是阻塞调用，不占事件循环
                snapshot = await asyncio.to_thread(self.build_snapshot)
                self._last_good = snapshot
                if self._last_loop_error is not None:
                    logger.info("[desktop_status] 采集轮次已恢复。")
                    self._last_loop_error = None
                if self._push_url:
                    await self._push(snapshot)
            except asyncio.CancelledError:
                raise
            except Exception as exc:  # noqa: BLE001
                name = type(exc).__name__
                # 稳定失败时每轮都告警会把 logs.tail 冲掉：只在状态变化时报一次
                if self._last_loop_error != name:
                    logger.warning("[desktop_status] 采集轮次失败：%s", name)
                    self._last_loop_error = name
            await asyncio.sleep(self._interval)

    async def _push(self, snapshot: dict) -> None:
        """推送快照；失败只降级日志，不抛主循环。"""
        body = json.dumps(snapshot, ensure_ascii=False).encode("utf-8")
        status, reason = await asyncio.to_thread(self._post, body)
        if status is not None and 200 <= status < 300:
            if self._last_push_ok is not True:
                logger.info("[desktop_status] 推送恢复：接收端已接收快照。")
            self._last_push_ok = True
            return
        if self._last_push_ok is not False:
            detail = f"HTTP {status}" if status is not None else reason
            logger.warning("[desktop_status] 推送失败：%s（接收端未就绪时属正常）", detail)
        self._last_push_ok = False

    def _post(self, body: bytes) -> tuple[int | None, str]:
        """阻塞 POST；返回 (状态码或 None, 失败原因)。"""
        request = urllib.request.Request(
            self._push_url,
            data=body,
            method="POST",
            headers={
                "Content-Type": "application/json; charset=utf-8",
                "X-Status-Token": self._token,
                "User-Agent": "astrbot-plugin-desktop-status/0.1",
            },
        )
        try:
            with urllib.request.urlopen(request, timeout=max(2.0, self._interval)) as resp:
                return int(resp.status), ""
        except urllib.error.HTTPError as exc:
            return int(exc.code), "http_error"
        except (urllib.error.URLError, OSError, ValueError) as exc:
            return None, type(exc).__name__

    # -- 线程桥接（唯一写法） ------------------------------------------------

    def _call_async(self, coroutine, timeout: float = ACTION_TIMEOUT_SEC):
        """从**非事件循环线程**调度协程并等结果；循环不可用时直接抛给调用方兜。"""
        loop = self._loop
        if loop is None or loop.is_closed() or not loop.is_running():
            raise RuntimeError("astrbot event loop unavailable")
        return asyncio.run_coroutine_threadsafe(coroutine, loop).result(timeout=timeout)

    def _in_loop_thread(self) -> bool:
        """当前线程是否就是事件循环线程。"""
        loop = self._loop
        if loop is None or not loop.is_running():
            return False
        return _running_loop_is(loop)

    # -- 反向端口 -----------------------------------------------------------

    def _http_handler_class(self) -> type:
        """构造只认 `GET /v1/snapshot` 与 `POST /v1/actions` 的请求处理器。"""
        plugin = self

        class Handler(BaseHTTPRequestHandler):
            def log_message(self, format: str, *args: Any) -> None:  # noqa: A002
                """关掉默认 stderr 访问日志，避免刷屏与递归。"""
                return

            def _send_json(self, status: int, payload: dict, extra: dict | None = None) -> None:
                body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
                self.send_response(status)
                self.send_header("Content-Type", "application/json; charset=utf-8")
                self.send_header("Content-Length", str(len(body)))
                for key, value in (extra or {}).items():
                    self.send_header(key, value)
                self.end_headers()
                self.wfile.write(body)

            def _send_error(self, status: int, code: str, detail: Any = None, extra: dict | None = None) -> None:
                self._send_json(status, {"error": code, "detail": detail}, extra)

            def _authorized(self) -> bool:
                """先校验 token，失败直接 401；比较用 compare_digest。"""
                provided = self.headers.get("X-Status-Token") or ""
                expected = plugin._token or ""
                if not expected or not secrets.compare_digest(provided, expected):
                    self._send_error(401, "unauthorized")
                    return False
                return True

            def _path(self) -> str:
                return self.path.split("?", 1)[0]

            def _read_body(self) -> bytes | None:
                """读请求体；长度非法按 0，超过 1 MiB 回 413 并返回 None。"""
                try:
                    length = int(self.headers.get("Content-Length") or 0)
                except ValueError:
                    length = 0
                if length <= 0:
                    return b""
                if length > MAX_PAYLOAD_BYTES:
                    self._send_error(413, "payload_too_large")
                    return None
                data = self.rfile.read(length)
                if len(data) > MAX_PAYLOAD_BYTES:
                    self._send_error(413, "payload_too_large")
                    return None
                return data

            def _reject_method(self, allow: str) -> None:
                self._send_error(405, "method_not_allowed", None, {"Allow": allow})

            def do_GET(self) -> None:  # noqa: N802 - http.server 规定的名字
                try:
                    if not self._authorized():
                        return
                    path = self._path()
                    if path == "/v1/actions":
                        self._reject_method("POST")
                        return
                    if path != "/v1/snapshot":
                        self._send_error(404, "not_found")
                        return
                    if getattr(plugin, "_closed", False):
                        self._send_error(503, "service_unavailable")
                        return
                    self._send_json(200, plugin._last_good or plugin.build_snapshot())
                except Exception as exc:  # noqa: BLE001
                    self._send_error(500, "collect_failed", type(exc).__name__)

            def do_POST(self) -> None:  # noqa: N802
                try:
                    if not self._authorized():
                        return
                    if getattr(plugin, "_closed", False):
                        self._send_error(503, "service_unavailable")
                        return
                    path = self._path()
                    if path == "/v1/snapshot":
                        self._reject_method("GET")
                        return
                    if path != "/v1/actions":
                        self._send_error(404, "not_found")
                        return
                    raw = self._read_body()
                    if raw is None:
                        return
                    status, payload = plugin.handle_action_blocking(raw)
                    self._send_json(status, payload)
                except Exception as exc:  # noqa: BLE001
                    # 请求线程绝不把异常抛回 socketserver（会打 traceback 并断连）
                    self._send_error(500, "internal_error", type(exc).__name__)

            def _not_allowed(self) -> None:
                try:
                    path = self._path()
                    if path not in ("/v1/snapshot", "/v1/actions"):
                        self._send_error(404, "not_found")
                        return
                    if not self._authorized():
                        return
                    self._reject_method("GET" if path == "/v1/snapshot" else "POST")
                except Exception:  # noqa: BLE001
                    pass

            do_PUT = _not_allowed  # noqa: N815
            do_DELETE = _not_allowed  # noqa: N815
            do_PATCH = _not_allowed  # noqa: N815

        return Handler

    def _start_server(self) -> None:
        """起反向端口；已在跑或配置为 0 时不动。"""
        if self._server is not None or not self._listen_port:
            return
        server = make_server(self._listen_port, self._http_handler_class())
        thread = threading.Thread(target=server.serve_forever, name="desktop-status-http", daemon=True)
        thread.start()
        self._server, self._server_thread = server, thread
        logger.info("[desktop_status] 反向端口已监听 127.0.0.1:%s。", self._listen_port)

    def stop_server_sync(self) -> None:
        """同步关端口（可在任意线程调用）：shutdown 会阻塞到 serve_forever 返回。"""
        server, self._server = self._server, None
        thread, self._server_thread = self._server_thread, None
        if server is None:
            return
        try:
            server.shutdown()
        except Exception:  # noqa: BLE001
            pass
        try:
            server.server_close()
        except Exception:  # noqa: BLE001
            pass
        if thread is not None and thread.is_alive() and thread is not threading.current_thread():
            thread.join(timeout=2)

    async def _stop_server(self) -> None:
        """异步关端口：阻塞的 shutdown 放线程里等，不依赖事件循环是否还活着。"""
        if self._server is None:
            return
        await asyncio.to_thread(self.stop_server_sync)

    # -- 动作 ---------------------------------------------------------------

    def handle_action_blocking(self, raw: bytes) -> tuple[int, dict]:
        """校验并执行动作请求（HTTP 线程调用）；返回 (状态码, 响应体)。"""
        try:
            payload = json.loads(raw.decode("utf-8") or "null")
        except (UnicodeDecodeError, json.JSONDecodeError):
            return 400, {"error": "invalid_action", "detail": "请求体不是合法 JSON"}
        if not isinstance(payload, dict):
            return 400, {"error": "invalid_action", "detail": "请求体应为 JSON 对象"}
        if payload.get("action") != "set_enabled":
            return 400, {"error": "invalid_action", "detail": "不支持的 action"}
        platform_id = payload.get("id")
        enabled = payload.get("enabled")
        if not isinstance(platform_id, str) or not platform_id.strip():
            return 400, {"error": "invalid_action", "detail": "缺少 id"}
        if not isinstance(enabled, bool):
            return 400, {"error": "invalid_action", "detail": "enabled 应为布尔值"}
        coroutine = self.set_platform_enabled(platform_id.strip(), enabled)
        if self._in_loop_thread():
            return 500, {"error": "unavailable", "detail": "动作不能从事件循环线程发"}
        try:
            result = self._call_async(coroutine, timeout=ACTION_TIMEOUT_SEC)
        except Exception as exc:  # noqa: BLE001
            coroutine.close()
            logger.warning("[desktop_status] 动作执行失败：%s", type(exc).__name__)
            return 500, {"error": "action_failed", "detail": type(exc).__name__}
        if not result.get("ok"):
            if result.get("reason") == "unsupported_platform":
                return 409, {"error": "unsupported_platform", "detail": result}
            return 404, {"error": "action_failed", "detail": result}
        return 200, result

    async def set_platform_enabled(self, platform_id: str, enabled: bool) -> dict:
        """启停平台：先试 BotConfigService，失败回落 platform_manager.reload。"""
        errors: list[str] = []
        service = self._bot_config_service()
        if service is not None:
            try:
                await service.set_bot_enabled(platform_id, enabled)
                await self._reload_from_config(platform_id)
                return {"ok": True, "via": "BotConfigService", "id": platform_id, "enabled": enabled}
            except Exception as exc:  # noqa: BLE001
                errors.append(f"BotConfigService:{type(exc).__name__}")
        if platform_id not in self._platform_configs():
            # 不在 cmd_config.platform 里（如内置 webchat）：不是「不存在」，是不支持启停
            return {"ok": False, "reason": "unsupported_platform", "id": platform_id, "enabled": enabled}
        try:
            await self._toggle_and_reload(platform_id, enabled)
            return {"ok": True, "via": "platform_manager.reload", "id": platform_id, "enabled": enabled}
        except Exception as exc:  # noqa: BLE001
            logger.warning("[desktop_status] 启停平台失败：%s", type(exc).__name__)
            return {
                "ok": False,
                "via": None,
                "id": platform_id,
                "enabled": enabled,
                "errors": errors + [f"reload:{type(exc).__name__}"],
            }

    def _bot_config_service(self):
        """尝试构造 BotConfigService；插件侧一般拿不到 core_lifecycle，拿不到返回 None。"""
        lifecycle = self._find_core_lifecycle()
        if lifecycle is None:
            return None
        try:
            from astrbot.dashboard.services.config_service import BotConfigService

            return BotConfigService(lifecycle)
        except Exception:  # noqa: BLE001
            return None

    def _platform_configs(self) -> dict:
        """`platform_manager.platforms_config` 是 list（不是 platform_configs），按 id 建索引。"""
        configs = getattr(self.context.platform_manager, "platforms_config", None)
        if isinstance(configs, dict):
            return configs
        return {
            item["id"]: item
            for item in (configs or [])
            if isinstance(item, dict) and isinstance(item.get("id"), str)
        }

    async def _reload_from_config(self, platform_id: str) -> None:
        """set_bot_enabled 已写盘；用写盘后的内存配置让 platform_manager 立即生效。"""
        config = self._platform_configs().get(platform_id)
        if isinstance(config, dict):
            await self.context.platform_manager.reload(copy.deepcopy(config))

    async def _toggle_and_reload(self, platform_id: str, enabled: bool) -> None:
        """回落路径：改内存配置里的 enable（能写盘就写盘），再 reload 该平台。"""
        configs = self._platform_configs()
        config = configs.get(platform_id)
        if not isinstance(config, dict):
            raise KeyError(f"platform {platform_id} not found")
        new_config = copy.deepcopy(config)
        new_config["enable"] = enabled
        config["enable"] = enabled  # 同步 platform_manager 自己的列表，reload 的配置同步靠它
        self._persist_platform_enable(platform_id, enabled)
        await self.context.platform_manager.reload(new_config)

    def _persist_platform_enable(self, platform_id: str, enabled: bool) -> bool:
        """把 enable 写回 AstrBot 主配置（cmd_config.json），否则重启后被原值覆盖。"""
        try:
            config = self.context.get_config()
        except Exception:  # noqa: BLE001
            return False
        platforms = config.get("platform") if isinstance(config, dict) else None
        if not isinstance(platforms, list):
            return False
        changed = False
        for item in platforms:
            if isinstance(item, dict) and item.get("id") == platform_id:
                item["enable"] = enabled
                changed = True
        if changed:
            save = getattr(config, "save_config", None)
            if callable(save):
                save()
        return changed

    # -- 日志 handler -------------------------------------------------------

    def _log_logger_names(self) -> list[str]:
        """astrbot 根 logger + 本插件 logger。"""
        names = ["astrbot"]
        plugin_name = getattr(self, "name", None)
        if plugin_name:
            names.append(f"astrbot.plugin.{plugin_name}")
        return names

    def _attach_log_handler(self) -> None:
        """挂到根 logger 与插件 logger；重复调用不会重复挂。"""
        self._detach_log_handler()
        handler = RingBufferHandler(self._log_tail)
        targets = []
        for name in self._log_logger_names():
            target = logging.getLogger(name)
            if target not in targets:
                targets.append(target)
                target.addHandler(handler)
        self._log_handler, self._attach_targets = handler, targets

    def _detach_log_handler(self) -> None:
        """摘钩干净，只摘自己挂的；可重复调用。"""
        handler, targets = self._log_handler, self._attach_targets
        self._log_handler, self._attach_targets = None, []
        if handler is None:
            return
        for target in targets:
            try:
                target.removeHandler(handler)
            except Exception:  # noqa: BLE001
                pass
        try:
            handler.close()
        except Exception:  # noqa: BLE001
            pass

    def _ensure_log_handler(self) -> None:
        """handler 丢了（别的插件重载会重置根 logger）就补挂；终止之后一律不挂。"""
        if self._closed:
            return
        handler = self._log_handler
        if handler is not None and handler.capacity == self._log_tail:
            return
        self._attach_log_handler()

    # -- 采集：整份快照 -----------------------------------------------------

    def build_snapshot(self) -> dict:
        """同步采集一份完整快照；每段独立兜底，失败给 null。"""
        snapshot: dict = {
            "schemaVersion": SCHEMA_VERSION,
            "collectedAt": 0,
            "instance": None,
            "service": None,
            "platforms": None,
            "resources": None,
            "messages": None,
            "logs": None,
            "usage": None,
        }
        if self._closed:
            # 已 terminate：不再采集，也绝不重新挂 handler
            return self._last_good or snapshot
        errors: list[str] = []
        for key, collector in (
            ("instance", self.collect_instance),
            ("service", self.collect_service),
            ("platforms", self.collect_platforms),
            ("resources", self.collect_resources),
            ("messages", self.collect_messages),
            ("logs", self.collect_logs),
            ("usage", self.collect_usage),
        ):
            try:
                snapshot[key] = collector()
            except Exception as exc:  # noqa: BLE001
                errors.append(f"{key}:{type(exc).__name__}")
                snapshot[key] = None
        snapshot["collectedAt"] = int(time.time())
        try:
            self._report_alerts(snapshot)
        except Exception:  # noqa: BLE001
            pass
        self._ensure_log_handler()
        current_errors = tuple(errors)
        if current_errors and current_errors != self._last_collect_errors:
            logger.warning("[desktop_status] 本轮采集失败项：%s", ", ".join(errors))
        elif not current_errors and self._last_collect_errors:
            logger.info("[desktop_status] 采集失败项已全部恢复。")
        self._last_collect_errors = current_errors
        return snapshot

    def _report_alerts(self, snapshot: dict) -> None:
        """按 error_keywords 匹配平台错误与最近日志；命中写插件日志（快照不加字段）。"""
        if not self._keywords:
            return
        hits: list[str] = []
        for item in (snapshot.get("platforms") or {}).get("items") or []:
            message = (item.get("lastError") or {}).get("message")
            keyword = match_keywords(message, self._keywords) if message else None
            if keyword:
                hits.append(f"平台 {item.get('id')} 含 {keyword}")
        for entry in ((snapshot.get("logs") or {}).get("tail") or [])[-20:]:
            keyword = match_keywords(entry.get("text") or "", self._keywords)
            if keyword:
                hits.append(f"日志含 {keyword}")
        if hits:
            logger.debug("[desktop_status] 告警匹配 %d 条：%s", len(hits), "；".join(hits[:3]))

    # -- 采集：各段 ---------------------------------------------------------

    def collect_instance(self) -> dict:
        """实例 UUID 与 AstrBot 版本。"""
        return {"id": self._resolve_instance_id(), "astrbotVersion": self._package_version("astrbot")}

    def collect_service(self) -> dict:
        """在线状态与启动时刻来自免鉴权 `/api/stat/start-time`，版本来自 `/api/stat/versions`。"""
        online: bool | None = None
        start_time: int | None = None
        base = self._base_url()
        status, body = self._http_json(f"{base}/api/stat/start-time")
        if status is not None:
            if 200 <= status < 300:
                online = True
                start_time = unix_sec((_unwrap_data(body) or {}).get("start_time"))
            else:
                # 401/5xx 说明端口有人但不可用；与「端口没人」（status None）区分开
                online = False
        if start_time is None and online is True:
            start_time = self._process_start_time()
        return {"online": online, "startTime": start_time, "versions": self._collect_versions(base)}

    def _collect_versions(self, base: str) -> dict | None:
        """版本三字段来自 `/api/stat/versions`（本机返回 `{"status":"ok","data":{...}}`）。"""
        versions: dict = {"astrbot": self._package_version("astrbot"), "webui": None, "code": None}
        status, body = self._http_json(f"{base}/api/stat/versions")
        if status is not None and 200 <= status < 300:
            payload = _unwrap_data(body) or {}
            versions["astrbot"] = clean_text(payload.get("astrbot_version"), 64) or versions["astrbot"]
            versions["webui"] = clean_text(payload.get("webui_version"), 64)
            versions["code"] = clean_text(payload.get("astrbot_code_version"), 64)
        return versions if any(value is not None for value in versions.values()) else None

    def collect_platforms(self) -> dict | None:
        """平台状态：`platform_manager.get_all_stats()`，下划线键与 ISO 时间统一换算。"""
        manager = getattr(self.context, "platform_manager", None)
        if manager is None:
            return None
        raw = manager.get_all_stats()
        if not isinstance(raw, dict):
            return None
        items = [self._normalize_platform(item) for item in raw.get("platforms") or []]
        summary_raw = raw.get("summary")
        summary = None
        if isinstance(summary_raw, dict):
            summary = {
                "total": to_int(summary_raw.get("total")),
                "running": to_int(summary_raw.get("running")),
                "error": to_int(summary_raw.get("error")),
                "totalErrors": to_int(summary_raw.get("total_errors", summary_raw.get("totalErrors"))),
            }
        return {"summary": summary, "items": items}

    @staticmethod
    def _normalize_platform(item: Any) -> dict:
        """单个平台：AstrBot 给 `display_name` / `started_at`（ISO）/ `error_count` / `last_error`。"""
        blank = {
            "id": None,
            "type": None,
            "displayName": None,
            "status": "unknown",
            "startedAt": None,
            "errorCount": None,
            "lastError": None,
        }
        if not isinstance(item, dict):
            return blank
        last_error = item.get("last_error")
        normalized_error = None
        if isinstance(last_error, dict) and last_error.get("message") is not None:
            normalized_error = {
                "message": clean_text(last_error.get("message")) or "",
                "timestamp": unix_sec(last_error.get("timestamp")),
            }
        return {
            "id": clean_text(item.get("id"), 128),
            "type": clean_text(item.get("type"), 64),
            "displayName": clean_text(item.get("display_name"), 128),
            "status": normalize_status(item.get("status")),
            "startedAt": unix_sec(item.get("started_at")),
            "errorCount": to_int(item.get("error_count")),
            "lastError": normalized_error,
        }

    def _psutil_process(self):
        """进程句柄懒加载；psutil 不可用时返回 None。"""
        if self._process is not None:
            return self._process
        try:
            import psutil

            self._process = psutil.Process(os.getpid())
        except Exception:  # noqa: BLE001
            return None
        return self._process

    def collect_resources(self) -> dict | None:
        """进程 CPU / 内存 / 线程；CPU 首帧必给 null（预热）。"""
        process = self._psutil_process()
        if process is None:
            return None
        import psutil

        cpu: float | None = None
        if self._prime_cpu:
            process.cpu_percent(None)  # 第一次调用只用于预热，返回值无意义
            self._prime_cpu = False
        else:
            cpu = to_float(process.cpu_percent(None))
        memory = None
        try:
            memory = {
                "processMb": round(process.memory_info().rss / (1024 * 1024), 1),
                "systemMb": round(psutil.virtual_memory().total / (1024 * 1024), 1),
            }
        except Exception:  # noqa: BLE001
            memory = None
        try:
            thread_count = to_int(process.num_threads())
        except Exception:  # noqa: BLE001
            thread_count = None
        return {"cpuPercent": cpu, "memory": memory, "threadCount": thread_count}

    def collect_logs(self) -> dict:
        """日志环形缓冲（已截断 500 字符、按时间正序）。"""
        handler = self._log_handler
        return {"tail": handler.snapshot() if handler is not None else None}

    def collect_messages(self) -> dict | None:
        """消息总量与最近 24 个整点趋势。

        插件侧拿不到 `core_lifecycle`（04 §5.3 要求的实测结论：StatService 构造依赖它），
        直接读 `platform_stats` 表：趋势按 24h 窗口，总量按全表。
        """
        try:
            return self._run_sync(self._messages_query)
        except Exception:  # noqa: BLE001
            return None

    def collect_usage(self) -> dict | None:
        """Token 用量：优先 `StatService.get_provider_token_stats(days)`，构造不起来回落 DB。"""
        try:
            stats = self._run_sync(self._usage_query)
        except Exception:  # noqa: BLE001
            return None
        if not isinstance(stats, dict):
            return None
        by_provider = stats.get("byProvider")
        return {
            "rangeDays": stats.get("rangeDays", 1),
            "rangeTotalTokens": stats.get("rangeTotalTokens"),
            "rangeOutputTokens": stats.get("rangeOutputTokens"),
            "todayTotalTokens": stats.get("todayTotalTokens"),
            "byProvider": by_provider,
            "cost": cost_from_pricing(self._pricing, by_provider, to_int(stats.get("rangeOutputTokens"))),
        }

    def _run_sync(self, coroutine_factory):
        """采集线程调用：已在事件循环线程就跑同步查询，否则桥接过去等结果。

        之前一律桥接，结果在循环线程里自己等自己，每个快照卡满 30s（首轮实测 394s）。
        """
        result = coroutine_factory()
        if hasattr(result, "__await__"):
            # 查询本身是协程时才桥接；同步查询直接返回，别再套一层 asyncio.run
            return self._call_async(result, timeout=COLLECT_TIMEOUT_SEC)
        return result

    def _messages_query(self) -> dict | None:
        """`platform_stats` 直查（同步）：趋势 24h 窗口 + 全表总量。"""

        from sqlalchemy import func, select

        from astrbot.core.db.po import PlatformStat

        db = self.context.get_db()
        now = int(time.time())
        window_start = datetime.now().replace(microsecond=0) - timedelta(seconds=MESSAGES_WINDOW_SEC)
        with _session_of(db) as session:
            rows = session.execute(
                select(PlatformStat.timestamp, PlatformStat.count)
                .where(PlatformStat.timestamp >= window_start)
                .order_by(PlatformStat.timestamp)
            ).all()
            total = session.execute(select(func.coalesce(func.sum(PlatformStat.count), 0))).scalar_one()
        return {"total": to_int(total), "series": series_from_rows(rows, now)}

    def _usage_query(self) -> dict | None:
        """Token 用量（同步）：先试 StatService，再回落 DB。"""
        days = 1
        stats = self._provider_stats_via_service(days)
        if stats is None:
            stats = self._provider_stats_via_db(days)
        return stats

    def _provider_stats_via_service(self, days: int) -> dict | None:
        """尝试构造 StatService 并调用；构造不起来返回 None（不抛）。"""
        lifecycle = self._find_core_lifecycle()
        if lifecycle is None:
            return None
        try:
            from astrbot.dashboard.services.stat_service import StatService

            service = StatService(self.context.get_db(), lifecycle, lifecycle.astrbot_config)
        except Exception:  # noqa: BLE001
            return None
        try:
            # 采集线程（已在事件循环线程内）不允许再 asyncio.run；这种情况只用 DB 回落
            raw = asyncio.run(service.get_provider_token_stats(days))
        except RuntimeError:
            return None
        except Exception:  # noqa: BLE001
            return None
        return self._normalize_provider_stats(raw, days)

    @staticmethod
    def _normalize_provider_stats(raw: Any, days: int) -> dict | None:
        """StatService 输出按 04 §4.7 转 camelCase；缺失项给 null。"""
        if not isinstance(raw, dict):
            return None
        by_provider = [
            {"providerId": clean_text(item.get("provider_id"), 128), "totalTokens": to_int(item.get("tokens"))}
            for item in (raw.get("range_by_provider") or [])
            if isinstance(item, dict)
        ]
        today_values = [
            to_int(item.get("tokens")) for item in (raw.get("today_by_provider") or []) if isinstance(item, dict)
        ]
        today_values = [value for value in today_values if value is not None]
        return {
            "rangeDays": to_int(raw.get("days")) or days,
            "rangeTotalTokens": to_int(raw.get("range_total_tokens")),
            "rangeOutputTokens": to_int(raw.get("range_output_tokens")),
            "todayTotalTokens": sum(today_values) if today_values else None,
            "byProvider": by_provider or None,
        }

    def _provider_stats_via_db(self, days: int) -> dict | None:
        """DB 回落（同步）：直查 `provider_stats` 表的 token 列。"""
        from sqlalchemy import func, select

        from astrbot.core.db.po import ProviderStat

        db = self.context.get_db()
        now = datetime.now()
        # created_at 落库为 UTC：本地日界/窗口先减时区偏移再比，否则「今日」会差 8 小时
        offset = datetime.now().astimezone().utcoffset() or timedelta(0)
        start = now.replace(microsecond=0) - timedelta(days=days) - offset
        today_start = now.replace(hour=0, minute=0, second=0, microsecond=0) - offset
        with _session_of(db) as session:
            rows = session.execute(
                select(
                    ProviderStat.provider_id,
                    func.coalesce(func.sum(ProviderStat.token_input_other + ProviderStat.token_input_cached + ProviderStat.token_output), 0),
                    func.coalesce(func.sum(ProviderStat.token_output), 0),
                )
                .where(ProviderStat.created_at >= start)
                .group_by(ProviderStat.provider_id)
            ).all()
            today_total = session.execute(
                select(func.coalesce(func.sum(ProviderStat.token_input_other + ProviderStat.token_input_cached + ProviderStat.token_output), 0)).where(
                    ProviderStat.created_at >= today_start
                )
            ).scalar_one_or_none()
        if not rows:
            # 窗口内一行都没有：采到了，但确实没有调用记录 → 0 / []
            return {
                "rangeDays": days,
                "rangeTotalTokens": 0,
                "rangeOutputTokens": 0,
                "todayTotalTokens": to_int(today_total) or 0,
                "byProvider": [],
            }
        ordered = sorted(rows, key=lambda row: to_int(row[1]) or 0, reverse=True)
        by_provider = [{"providerId": clean_text(row[0], 128), "totalTokens": to_int(row[1]) or 0} for row in ordered]
        return {
            "rangeDays": days,
            "rangeTotalTokens": sum(item["totalTokens"] for item in by_provider),
            "rangeOutputTokens": sum(to_int(row[2]) or 0 for row in ordered),
            "todayTotalTokens": to_int(today_total),
            "byProvider": by_provider,
        }

    # -- 采集工具 -----------------------------------------------------------

    def _find_core_lifecycle(self):
        """插件侧一般拿不到 `core_lifecycle`；能拿到才走服务层。"""
        for candidate in (
            getattr(self._raw_config, "_core_lifecycle", None),
            getattr(self.context, "core_lifecycle", None),
        ):
            if candidate is not None:
                return candidate
        return None

    @staticmethod
    def _base_url() -> str:
        """本机 WebUI 基址；端口只从环境变量读，绝不读 dashboard 凭据。"""
        port = os.environ.get("ASTRBOT_DASHBOARD_PORT") or os.environ.get("DASHBOARD_PORT") or "6185"
        return f"http://127.0.0.1:{port}"

    @staticmethod
    def _http_json(url: str) -> tuple[int | None, dict | None]:
        """免鉴权 GET；返回 (状态码或 None, 对象)。"""
        try:
            with urllib.request.urlopen(url, timeout=3) as resp:
                status = int(resp.status)
                raw = resp.read(MAX_PAYLOAD_BYTES)
        except urllib.error.HTTPError as exc:
            return int(exc.code), None
        except (urllib.error.URLError, OSError, ValueError):
            return None, None
        try:
            body = json.loads(raw.decode("utf-8"))
        except (UnicodeDecodeError, json.JSONDecodeError):
            body = None
        return status, body if isinstance(body, dict) else None

    @staticmethod
    def _package_version(name: str) -> str | None:
        """读包版本号，不 import 包本体。"""
        try:
            from importlib.metadata import version

            return clean_text(version(name), 32)
        except Exception:  # noqa: BLE001
            return None

    def _process_start_time(self) -> int | None:
        """探活成功但没给启动时刻时，用进程自身创建时间兜底。"""
        try:
            process = self._psutil_process()
            return int(process.create_time()) if process is not None else None
        except Exception:  # noqa: BLE001
            return None

    def _resolve_instance_id(self) -> str | None:
        """实例 UUID：先读环境变量，再读配置文件里的 `uuid` 键；只取这一个键。"""
        if self._instance_id:
            return self._instance_id
        env = os.environ.get("ASTRBOT_INSTANCE_ID")
        if env:
            self._instance_id = clean_text(env, 64)
            return self._instance_id
        for path in self._config_candidates():
            try:
                with path.open("r", encoding="utf-8") as handle:
                    data = json.load(handle)
            except (OSError, json.JSONDecodeError, UnicodeDecodeError):
                continue
            if isinstance(data, dict) and isinstance(data.get("uuid"), str):
                self._instance_id = clean_text(data["uuid"], 64)
                return self._instance_id
        return None

    @staticmethod
    def _config_candidates() -> list[Path]:
        """AstrBot 配置文件候选路径（只读）。"""
        candidates: list[Path] = []
        raw = os.environ.get("ASTRBOT_CONFIG_PATH")
        if raw:
            candidates.append(Path(raw))
        here = Path(__file__).resolve()
        for base in [Path.cwd()] + list(here.parents[:4]):
            for rel in ("data/config/astrbot.json", "config/astrbot.json"):
                candidates.append(base / rel)
        return candidates


# ---------------------------------------------------------------------------
# 同步桥工具
# ---------------------------------------------------------------------------


def _running_loop_is(loop: asyncio.AbstractEventLoop) -> bool:
    """当前线程是否正运行着 `loop`（没有运行中的循环 → False）。"""
    try:
        return asyncio.get_running_loop() is loop
    except RuntimeError:
        return False


def _unwrap_data(body: object) -> dict | None:
    """本机免鉴权端点统一包一层 `{"status":"ok","data":{...}}`，取 data 段。"""
    if not isinstance(body, dict):
        return None
    inner = body.get("data")
    return inner if isinstance(inner, dict) else body


def _session_of(db):
    """`db.get_db()` 可能是 async 上下文，也可能是同步上下文；两者都包成同步 with。"""
    return _SyncSession(getattr(db, "get_db", lambda: None)())


class _SyncSession:
    """把 `async with db.get_db() as session` 与 `with db.get_db() as session` 统一成同步入口。

    异步上下文交给一个私有事件循环跑 enter/exit；同步上下文直接代理。
    """

    def __init__(self, manager: Any) -> None:
        self._manager = manager
        self._inner = None
        self._aenter = getattr(manager, "__aenter__", None)
        self._aexit = getattr(manager, "__aexit__", None)
        self._loop = None

    def __enter__(self):
        if self._aenter is not None:
            self._loop = asyncio.new_event_loop()
            self._inner = self._loop.run_until_complete(self._aenter())
            return _SyncProxy(self._inner, self._loop)
        if self._manager is None:
            raise RuntimeError("database unavailable")
        if hasattr(self._manager, "__enter__"):
            self._inner = self._manager.__enter__()
            return self._inner
        return _SyncProxy(self._manager, None)

    def __exit__(self, exc_type, exc, tb) -> bool:
        try:
            if self._aexit is not None:
                return bool(self._loop.run_until_complete(self._aexit(exc_type, exc, tb)))
            if hasattr(self._manager, "__exit__"):
                return bool(self._manager.__exit__(exc_type, exc, tb))
            return False
        finally:
            if self._loop is not None:
                try:
                    self._loop.run_until_complete(self._loop.shutdown_asyncgens())
                finally:
                    self._loop.close()
                    self._loop = None


class _SyncProxy:
    """同步代理：`execute()` 在私有循环里 await，其余调用原样转发。"""

    def __init__(self, inner: Any, loop) -> None:
        self._inner = inner
        self._loop = loop

    def __getattr__(self, name: str):
        attribute = getattr(self._inner, name)
        if not callable(attribute) or not hasattr(attribute, "__call__"):
            return attribute
        if not hasattr(attribute, "__await__") and type(attribute).__name__ != "method":

            def plain(*args, **kwargs):
                return attribute(*args, **kwargs)

            return plain

        def wrapped(*args, **kwargs):
            result = attribute(*args, **kwargs)
            if hasattr(result, "__await__"):
                loop = self._loop or asyncio.new_event_loop()
                try:
                    return loop.run_until_complete(result)
                finally:
                    if self._loop is None:
                        loop.close()
            return result

        return wrapped
