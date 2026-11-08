# SPDX-License-Identifier: MIT
"""Bounded authenticated OBS v5 JSON client for the dedicated loopback endpoint.

No import-time I/O. websocket-client is imported only by the default dialer.
Injected dialers must return an owned send/recv/settimeout/close socket, enforce
the supplied dial deadline and clean up anything they raise before returning.
The default dialer owns its raw TCP socket even when HTTP upgrade fails. Its
wire wrapper bounds partial I/O/control-frame loops, not just completed JSON.
No request retry/reconnect, recording lifecycle or acquisition acceptance here.
"""
import base64
import hashlib
import json
import math
import re
import socket
import time

URL = 'ws://127.0.0.1:4471'
MAX_TIMEOUT = 60
MAX_MESSAGE_BYTES = 64 * 1024
MAX_TOTAL_BYTES = 256 * 1024
MAX_MESSAGES = 32
MAX_WIRE_BYTES = 1024 * 1024
MAX_IO_OPERATIONS = 4096


class ObsWebSocketError(RuntimeError):
    def __init__(self, message, *, status_code=None):
        super().__init__(message)
        self.status_code = status_code


def _timeout(value):
    if type(value) not in (int, float) or not 0 < value <= MAX_TIMEOUT or not math.isfinite(value):
        raise ObsWebSocketError('invalid OBS timeout')
    return value


def _remaining(end, clock):
    value = end - clock()
    if not math.isfinite(value) or value <= 0:
        raise ObsWebSocketError('OBS operation deadline exceeded')
    return value


class WireSocket:
    """Owned raw socket with an absolute deadline across fragmented WS I/O."""
    def __init__(self, raw, *, clock=time.monotonic):
        self.raw, self.clock = raw, clock
        self.end = self.timeout = None
        self.bytes = self.operations = 0

    def begin(self, seconds):
        self.timeout = _timeout(seconds)
        self.end = self.clock() + seconds
        self.bytes = self.operations = 0

    def settimeout(self, seconds):
        self.timeout = _timeout(seconds)

    def gettimeout(self):
        return min(self.timeout, _remaining(self.end, self.clock))

    def _before(self):
        if self.operations >= MAX_IO_OPERATIONS or self.bytes >= MAX_WIRE_BYTES:
            raise ObsWebSocketError('OBS wire operation/byte bound')
        self.operations += 1
        self.raw.settimeout(self.gettimeout())

    def connect(self, address):
        self._before()
        self.raw.connect(address)
        _remaining(self.end, self.clock)

    def recv(self, size):
        self._before()
        data = self.raw.recv(min(size, MAX_WIRE_BYTES - self.bytes))
        self.bytes += len(data)
        _remaining(self.end, self.clock)
        return data

    def send(self, data):
        self._before()
        if len(data) > MAX_WIRE_BYTES - self.bytes:
            raise ObsWebSocketError('OBS wire byte bound')
        sent = self.raw.send(data)
        self.bytes += sent
        _remaining(self.end, self.clock)
        return sent

    def fileno(self):
        return self.raw.fileno()

    def close(self):
        self.raw.close()


class _Transport:
    def __init__(self, websocket, wire):
        self.websocket, self.wire = websocket, wire

    def settimeout(self, seconds):
        self.wire.begin(seconds)
        self.websocket.settimeout(seconds)

    def send(self, value):
        return self.websocket.send(value)

    def recv(self):
        return self.websocket.recv()

    def close(self):
        try:
            # shutdown closes immediately; close() may wait for a peer handshake.
            self.websocket.shutdown()
        finally:
            self.wire.close()


def _dial(seconds, clock):
    raw = websocket = None
    end = clock() + seconds
    try:
        import websocket as library
        raw = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        wire = WireSocket(raw, clock=clock)
        wire.begin(_remaining(end, clock))
        wire.connect(('127.0.0.1', 4471))
        websocket = library.WebSocket(enable_multithread=False, skip_utf8_validation=False)
        websocket.connect(URL, socket=wire, timeout=_remaining(end, clock),
                          subprotocols=['obswebsocket.json'], redirect_limit=0)
        status = websocket.getstatus()
        if (type(status) is not int or status != 101 or
                websocket.getsubprotocol() != 'obswebsocket.json'):
            raise ObsWebSocketError('invalid OBS websocket upgrade/subprotocol')
        _remaining(end, clock)
        return _Transport(websocket, wire)
    except BaseException:
        failed = False
        if websocket is not None:
            try:
                websocket.shutdown()
            except BaseException:
                failed = True
        if raw is not None:
            try:
                raw.close()
            except BaseException:
                failed = True
        raise ObsWebSocketError('OBS dial cleanup failed' if failed else 'OBS dial failed') from None


def _unique(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('duplicate key')
        result[key] = value
    return result


def _constant(_value):
    raise ValueError('nonfinite JSON')


def _tree(value, depth=0, count=None):
    count = [0] if count is None else count
    count[0] += 1
    if depth > 32 or count[0] > 4096:
        raise ValueError('JSON structural bound')
    if type(value) is dict:
        for key, item in value.items():
            if type(key) is not str:
                raise ValueError('JSON object key type')
            _tree(key, depth + 1, count)
            _tree(item, depth + 1, count)
    elif type(value) is list:
        for item in value:
            _tree(item, depth + 1, count)
    elif type(value) is str:
        value.encode('utf-8')
    elif type(value) is float:
        if not math.isfinite(value):
            raise ValueError('nonfinite JSON')
    elif value is not None and type(value) not in (int, bool):
        raise ValueError('JSON value type')


def _encode(value):
    try:
        _tree(value)
        text = json.dumps(value, ensure_ascii=False, allow_nan=False, separators=(',', ':'))
        if len(text.encode('utf-8')) > MAX_MESSAGE_BYTES:
            raise ValueError('JSON size')
        return text
    except (ValueError, TypeError, RecursionError, UnicodeError):
        raise ObsWebSocketError('invalid outbound OBS JSON') from None


def _string(value, maximum=4096):
    return type(value) is str and 0 < len(value) <= maximum


class WebSocketClient:
    def __init__(self, owner, clock):
        self.owner, self.clock = owner, clock
        self.sequence = 0
        self.usable = True
        self.closed = False
        self.messages = self.bytes = 0

    @staticmethod
    def connect(password, *, timeout, dial=None, clock=time.monotonic):
        return connect(password, timeout=timeout, dial=dial, clock=clock)

    def _settimeout(self, end):
        remaining = _remaining(end, self.clock)
        try:
            self.owner.settimeout(remaining)
        except Exception:
            raise ObsWebSocketError('OBS socket timeout setup failed') from None

    def _send(self, value, end):
        text = _encode(value)
        self._settimeout(end)
        try:
            self.owner.send(text)
        except Exception:
            raise ObsWebSocketError('OBS send failed; request outcome unknown') from None
        _remaining(end, self.clock)

    def _recv(self, end):
        if self.messages >= MAX_MESSAGES:
            raise ObsWebSocketError('OBS message bound')
        self._settimeout(end)
        try:
            text = self.owner.recv()
        except Exception:
            raise ObsWebSocketError('OBS receive failed; request outcome unknown') from None
        _remaining(end, self.clock)
        self.messages += 1
        try:
            if type(text) is not str or not text or len(text) > MAX_MESSAGE_BYTES:
                raise ValueError('text frame required')
            size = len(text.encode('utf-8'))
            self.bytes += size
            if size > MAX_MESSAGE_BYTES or self.bytes > MAX_TOTAL_BYTES:
                raise ValueError('JSON size')
            value = json.loads(text, object_pairs_hook=_unique, parse_constant=_constant)
            _tree(value)
            if (type(value) is not dict or type(value.get('op')) is not int or
                    type(value.get('d')) is not dict):
                raise ValueError('OBS envelope')
            return value['op'], value['d']
        except (ValueError, TypeError, RecursionError, UnicodeError):
            raise ObsWebSocketError('invalid inbound OBS JSON/envelope') from None

    def req(self, kind, data=None, *, timeout):
        seconds = _timeout(timeout)
        end = self.clock() + seconds
        if not self.usable or self.closed:
            raise ObsWebSocketError('OBS connection is closed or unusable')
        if type(kind) is not str or not re.fullmatch(r'[A-Za-z][A-Za-z0-9]{0,127}', kind):
            raise ObsWebSocketError('invalid OBS request type')
        if data is not None and type(data) is not dict:
            raise ObsWebSocketError('invalid OBS request data')
        rid = str(self.sequence + 1)
        payload = dict(op=6, d=dict(requestType=kind, requestId=rid,
                                   requestData={} if data is None else data))
        _encode(payload)
        self.sequence += 1
        self.messages = self.bytes = 0
        # Any failure after dispatch poisons the connection. Never retry an
        # unconfirmed mutation or accept a later response as a different request.
        self.usable = False
        self._send(payload, end)
        while True:
            op, result = self._recv(end)
            if op == 5:
                if (not _string(result.get('eventType'), 128) or
                        type(result.get('eventIntent')) is not int or
                        not 0 <= result['eventIntent'] <= 0xffffffff or
                        'eventData' in result and type(result['eventData']) is not dict):
                    raise ObsWebSocketError('invalid OBS event')
                continue
            if (op != 7 or result.get('requestId') != rid or
                    type(result.get('requestId')) is not str or result.get('requestType') != kind):
                raise ObsWebSocketError('unexpected OBS response identity/op')
            status = result.get('requestStatus')
            if (type(status) is not dict or type(status.get('result')) is not bool or
                    type(status.get('code')) is not int or not 0 <= status['code'] <= 0xffffffff or
                    'comment' in status and type(status['comment']) is not str or
                    status['result'] != (status['code'] == 100)):
                raise ObsWebSocketError('invalid OBS response status')
            if status['result'] is False:
                raise ObsWebSocketError('OBS request refused', status_code=status['code'])
            body = result.get('responseData', {})
            if type(body) is not dict:
                raise ObsWebSocketError('invalid OBS response data')
            _remaining(end, self.clock)
            self.usable = True
            return body

    def close(self):
        self.usable = False
        if not self.closed:
            try:
                self.owner.close()
            except Exception:
                raise ObsWebSocketError('OBS socket cleanup failed') from None
            self.closed = True


def connect(password, *, timeout, dial=None, clock=time.monotonic):
    seconds = _timeout(timeout)
    end = clock() + seconds
    if not _string(password):
        raise ObsWebSocketError('invalid OBS password')
    try:
        password.encode('utf-8')
    except UnicodeError:
        raise ObsWebSocketError('invalid OBS password') from None
    owner = client = None
    try:
        if dial is None:
            owner = _dial(_remaining(end, clock), clock)
        else:
            try:
                owner = dial(URL, timeout=_remaining(end, clock))
            except Exception:
                raise ObsWebSocketError('OBS dial failed') from None
        client = WebSocketClient(owner, clock)
        op, hello = client._recv(end)
        auth = hello.get('authentication')
        if (op != 0 or type(hello.get('rpcVersion')) is not int or hello['rpcVersion'] != 1 or
                not _string(hello.get('obsStudioVersion'), 128) or
                not _string(hello.get('obsWebSocketVersion'), 128) or
                not re.fullmatch(r'5\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?',
                                 hello['obsWebSocketVersion']) or
                type(auth) is not dict or not _string(auth.get('salt')) or
                not _string(auth.get('challenge'))):
            raise ObsWebSocketError('invalid or unauthenticated OBS Hello')
        secret = base64.b64encode(hashlib.sha256((password + auth['salt']).encode()).digest()).decode()
        proof = base64.b64encode(hashlib.sha256((secret + auth['challenge']).encode()).digest()).decode()
        client._send(dict(op=1, d=dict(rpcVersion=1, eventSubscriptions=0, authentication=proof)), end)
        op, result = client._recv(end)
        if (op != 2 or type(result.get('negotiatedRpcVersion')) is not int or
                result['negotiatedRpcVersion'] != 1):
            raise ObsWebSocketError('invalid OBS Identified')
        _remaining(end, clock)
        return client
    except BaseException as error:
        failed = False
        if owner is not None:
            try:
                owner.close()
            except BaseException:
                failed = True
        message = str(error) if isinstance(error, ObsWebSocketError) else 'OBS handshake failed'
        if failed:
            message = 'OBS handshake cleanup failed'
        raise ObsWebSocketError(message) from None
