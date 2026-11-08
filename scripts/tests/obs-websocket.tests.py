# SPDX-License-Identifier: MIT
"""OBS v5 protocol tests: fake sockets only, never network or OBS."""
import base64
from collections import deque
import hashlib
import importlib.util
import json
from pathlib import Path
import sys
import traceback
import types
import unittest
from unittest import mock

sys.dont_write_bytecode = True
SOURCE = Path(__file__).resolve().parents[1] / 'obs-websocket.py'
SPEC = importlib.util.spec_from_file_location('obs_websocket_tests', SOURCE) if SOURCE.exists() else None
M = importlib.util.module_from_spec(SPEC) if SPEC else None
if SPEC:
    SPEC.loader.exec_module(M)

PASSWORD = 'private-password-not-for-errors'
SALT = 'private-salt'
CHALLENGE = 'private-challenge'


def wire(op, data):
    return json.dumps({'op': op, 'd': data})


def hello():
    return wire(0, {'obsStudioVersion': '31.0.0', 'obsWebSocketVersion': '5.5.0',
                    'rpcVersion': 1, 'authentication': {'salt': SALT, 'challenge': CHALLENGE}})


def identified():
    return wire(2, {'negotiatedRpcVersion': 1})


def response(kind='GetVersion', rid='1', **overrides):
    data = dict(requestType=kind, requestId=rid, requestStatus={'result': True, 'code': 100})
    data.update(overrides)
    return wire(7, data)


def event(**overrides):
    data = dict(eventType='RecordStateChanged', eventIntent=64, eventData={'outputActive': False})
    data.update(overrides)
    return wire(5, data)


class Clock:
    def __init__(self):
        self.now = 0.0

    def __call__(self):
        return self.now


class Socket:
    def __init__(self, clock, messages=()):
        self.clock, self.messages = clock, deque(messages)
        self.sent, self.timeouts, self.operations = [], [], []
        self.timeout = None
        self.send_delay = self.recv_delay = 0
        self.close_calls = 0
        self.send_error = self.recv_error = self.close_error = None

    def settimeout(self, timeout):
        self.timeout = timeout
        self.timeouts.append(timeout)
        self.operations.append('settimeout')

    def send(self, data):
        self.operations.append('send')
        self.sent.append(data)
        self.clock.now += self.send_delay
        if self.send_error:
            raise self.send_error
        return len(data)

    def recv(self):
        self.operations.append('recv')
        self.clock.now += self.recv_delay
        if self.recv_error:
            raise self.recv_error
        if not self.messages:
            raise TimeoutError('private underlying recv error')
        return self.messages.popleft()

    def close(self):
        self.close_calls += 1
        if self.close_error:
            raise self.close_error


class ProtocolTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(M, 'bounded OBS websocket connector is missing')
        self.clock = Clock()
        self.sock = Socket(self.clock, [hello(), identified()])
        self.dials = []

    def dial(self, url, *, timeout):
        self.dials.append((url, timeout))
        return self.sock

    def connect(self, password=PASSWORD, timeout=5):
        return M.connect(password, timeout=timeout, dial=self.dial, clock=self.clock)

    def assert_sanitized(self, error):
        text = ''.join(traceback.format_exception(error))
        for secret in (PASSWORD, SALT, CHALLENGE, 'private-remote-comment', 'private underlying'):
            self.assertNotIn(secret, text)

    def test_authenticated_handshake_matches_official_formula_and_fixed_url(self):
        client = self.connect()
        self.assertEqual(self.dials, [('ws://127.0.0.1:4471', 5)])
        secret = base64.b64encode(hashlib.sha256((PASSWORD + SALT).encode()).digest()).decode()
        auth = base64.b64encode(hashlib.sha256((secret + CHALLENGE).encode()).digest()).decode()
        self.assertEqual(json.loads(self.sock.sent[0]), {
            'op': 1, 'd': {'rpcVersion': 1, 'eventSubscriptions': 0, 'authentication': auth}})
        self.assertEqual(self.sock.operations, ['settimeout', 'recv', 'settimeout', 'send',
                                               'settimeout', 'recv'])
        self.assertIsNotNone(client)

    def test_class_connector_is_same_directly_callable_adapter_api(self):
        connector = getattr(M, 'WebSocketClient', None)
        self.assertIsNotNone(connector, 'WebSocketClient connector API is missing')
        client = connector.connect(PASSWORD, timeout=5, dial=self.dial, clock=self.clock)
        self.assertIsInstance(client, connector)
        self.assertEqual(len(self.dials), 1)
        self.sock.messages.append(response())
        self.assertEqual(client.req('GetVersion', timeout=3), {})
        client.close()
        self.assertEqual(self.sock.close_calls, 1)

    def test_import_does_not_import_websocket_client(self):
        original = __import__
        def guarded(name, *args, **kwargs):
            if name == 'websocket' or name.startswith('websocket.'):
                self.fail('websocket-client loaded before connect')
            return original(name, *args, **kwargs)
        with mock.patch('builtins.__import__', side_effect=guarded):
            spec = importlib.util.spec_from_file_location('obs_ws_import_test', SOURCE)
            spec.loader.exec_module(importlib.util.module_from_spec(spec))

    def test_bad_timeouts_refuse_before_dial(self):
        for value in (True, False, None, '5', 0, -1, float('nan'), float('inf'), 61, 10 ** 400):
            with self.subTest(value=value), self.assertRaises(M.ObsWebSocketError):
                self.connect(timeout=value)
        self.assertEqual(self.dials, [])

    def test_bad_password_refuses_before_dial(self):
        for value in (None, True, '', 'x' * 4097, '\ud800'):
            with self.subTest(value=repr(value)), self.assertRaises(M.ObsWebSocketError):
                self.connect(password=value)
        self.assertEqual(self.dials, [])

    def test_shared_connect_deadline_counts_dial_hello_and_identify(self):
        def dial(url, *, timeout):
            self.clock.now += 1
            return self.dial(url, timeout=timeout)
        self.sock.recv_delay = 1
        self.sock.send_delay = 1
        M.connect(PASSWORD, timeout=5, dial=dial, clock=self.clock)
        self.assertEqual(self.sock.timeouts, [4, 3, 2])

    def test_every_late_handshake_stage_refuses_and_closes(self):
        for stage in ('dial', 'hello', 'identify', 'identified'):
            with self.subTest(stage=stage):
                self.setUp()
                def dial(url, *, timeout):
                    if stage == 'dial':
                        self.clock.now += 6
                    return self.dial(url, timeout=timeout)
                original_recv = self.sock.recv
                def recv():
                    result = original_recv()
                    if (stage == 'hello' and len(self.sock.messages) == 1 or
                            stage == 'identified' and not self.sock.messages):
                        self.clock.now += 6
                    return result
                self.sock.recv = recv
                self.sock.send_delay = 6 if stage == 'identify' else 0
                with self.assertRaises(M.ObsWebSocketError):
                    M.connect(PASSWORD, timeout=5, dial=dial, clock=self.clock)
                self.assertEqual(self.sock.close_calls, 1)

    def test_invalid_hello_types_rpc_and_authentication_all_close(self):
        valid = json.loads(hello())['d']
        candidates = [dict(valid, rpcVersion=v) for v in (True, '1', 0, 2, 1.0)]
        candidates += [dict(valid, authentication=v) for v in (None, [], {},
            {'salt': '', 'challenge': CHALLENGE}, {'salt': SALT, 'challenge': 7})]
        candidates += [dict(valid, obsStudioVersion=v) for v in (None, '', 1)]
        candidates += [dict(valid, obsWebSocketVersion='4.9.0')]
        missing_auth = dict(valid)
        del missing_auth['authentication']
        candidates.append(missing_auth)
        for data in candidates:
            with self.subTest(data=data):
                self.setUp()
                self.sock.messages = deque([wire(0, data), identified()])
                with self.assertRaises(M.ObsWebSocketError):
                    self.connect()
                self.assertEqual(self.sock.close_calls, 1)

    def test_wrong_handshake_ops_and_negotiated_rpc_refuse(self):
        for messages in ([event(), identified()], [hello(), event()],
                         [hello(), wire(2, {'negotiatedRpcVersion': True})],
                         [hello(), wire(2, {'negotiatedRpcVersion': 2})],
                         [hello(), wire(2, {})]):
            with self.subTest(messages=messages):
                self.setUp()
                self.sock.messages = deque(messages)
                with self.assertRaises(M.ObsWebSocketError):
                    self.connect()
                self.assertEqual(self.sock.close_calls, 1)

    def test_invalid_json_envelope_binary_duplicates_and_nonfinite_refuse(self):
        candidates = ['{', '[]', 'null', '{}', wire(True, {}), wire(0, []), b'binary', '',
                      '{"op":0,"op":2,"d":{}}',
                      '{"op":0,"d":{"x":NaN}}', '{"op":0,"d":{"x":1e400}}',
                      '{"op":0,"d":{"x":Infinity}}', '[' * 1500 + ']' * 1500]
        for raw in candidates:
            with self.subTest(raw=str(raw)[:60]):
                self.setUp()
                self.sock.messages = deque([raw])
                with self.assertRaises(M.ObsWebSocketError) as caught:
                    self.connect()
                self.assert_sanitized(caught.exception)
                self.assertEqual(self.sock.close_calls, 1)

    def test_handshake_socket_errors_do_not_leak_secrets(self):
        self.sock.send_error = RuntimeError(PASSWORD + SALT + CHALLENGE)
        with self.assertRaises(M.ObsWebSocketError) as caught:
            self.connect()
        self.assert_sanitized(caught.exception)
        self.assertEqual(self.sock.close_calls, 1)

    def test_handshake_cleanup_failure_is_sanitized(self):
        self.sock.messages = deque(['malformed'])
        self.sock.close_error = RuntimeError(PASSWORD)
        with self.assertRaises(M.ObsWebSocketError) as caught:
            self.connect()
        self.assert_sanitized(caught.exception)
        self.assertIn('cleanup', str(caught.exception))

    def test_request_schema_response_and_unique_ids(self):
        client = self.connect()
        self.sock.messages.extend([response(responseData={'version': '5'}),
                                   response('SetInputVolume', '2')])
        self.assertEqual(client.req('GetVersion', timeout=3), {'version': '5'})
        self.assertEqual(client.req('SetInputVolume', {'inputVolumeMul': 1.0}, timeout=3), {})
        self.assertEqual(json.loads(self.sock.sent[1]), {'op': 6, 'd': {
            'requestType': 'GetVersion', 'requestId': '1', 'requestData': {}}})
        self.assertEqual(json.loads(self.sock.sent[2])['d']['requestId'], '2')

    def test_request_uses_one_deadline_for_send_events_and_response(self):
        client = self.connect()
        self.sock.send_delay = 1
        self.sock.recv_delay = 1
        self.sock.messages.extend([event(), response()])
        self.sock.timeouts.clear()
        self.assertEqual(client.req('GetVersion', timeout=4), {})
        self.assertEqual(self.sock.timeouts, [4, 3, 2])

    def test_late_request_send_never_receives_or_retries(self):
        client = self.connect()
        self.sock.send_delay = 4
        self.sock.messages.append(response('StartRecord'))
        before = self.sock.operations.count('recv')
        with self.assertRaises(M.ObsWebSocketError):
            client.req('StartRecord', timeout=3)
        self.assertEqual(self.sock.operations.count('recv'), before)
        with self.assertRaises(M.ObsWebSocketError):
            client.req('StartRecord', timeout=3)
        self.assertEqual(len(self.sock.sent), 2)

    def test_late_request_response_never_passes_or_retries(self):
        client = self.connect()
        self.sock.recv_delay = 4
        self.sock.messages.append(response())
        with self.assertRaises(M.ObsWebSocketError):
            client.req('GetVersion', timeout=3)
        with self.assertRaises(M.ObsWebSocketError):
            client.req('GetVersion', timeout=3)
        self.assertEqual(len(self.sock.sent), 2)

    def test_send_failure_consumes_one_mutation_attempt_and_poisons_connection(self):
        client = self.connect()
        self.sock.send_error = RuntimeError(PASSWORD)
        for _ in range(2):
            with self.assertRaises(M.ObsWebSocketError) as caught:
                client.req('StartRecord', timeout=3)
            self.assert_sanitized(caught.exception)
        self.assertEqual(len(self.sock.sent), 2)

    def test_mismatched_request_id_type_or_op_cannot_become_success(self):
        candidates = [response(rid='old'), response(rid=1), response('OtherRequest'),
                      wire(2, {'negotiatedRpcVersion': 1}), wire(8, {})]
        for raw in candidates:
            with self.subTest(raw=raw):
                self.setUp()
                client = self.connect()
                self.sock.messages.append(raw)
                with self.assertRaises(M.ObsWebSocketError):
                    client.req('GetVersion', timeout=3)

    def test_request_status_and_response_data_require_exact_types(self):
        statuses = [None, [], {}, {'result': 1, 'code': 100}, {'result': True, 'code': True},
                    {'result': True, 'code': '100'}, {'result': True, 'code': 608},
                    {'result': False, 'code': 100}, {'result': True, 'code': 100, 'comment': 7}]
        candidates = [response(requestStatus=v) for v in statuses]
        candidates += [response(responseData=v) for v in (None, [], True, 'text')]
        for raw in candidates:
            with self.subTest(raw=raw):
                self.setUp()
                client = self.connect()
                self.sock.messages.append(raw)
                with self.assertRaises(M.ObsWebSocketError):
                    client.req('GetVersion', timeout=3)

    def test_remote_failure_keeps_numeric_status_without_echoing_comment(self):
        client = self.connect()
        self.sock.messages.append(response(requestStatus={
            'result': False, 'code': 608, 'comment': 'private-remote-comment ' + PASSWORD}))
        with self.assertRaises(M.ObsWebSocketError) as caught:
            client.req('GetVersion', timeout=3)
        self.assert_sanitized(caught.exception)
        self.assertEqual(caught.exception.status_code, 608)

    def test_bad_events_are_not_silently_discarded(self):
        for data in ({'eventType': '', 'eventIntent': 1},
                     {'eventType': 'Changed', 'eventIntent': True},
                     {'eventType': 'Changed', 'eventIntent': 1, 'eventData': None}):
            with self.subTest(data=data):
                self.setUp()
                client = self.connect()
                self.sock.messages.extend([wire(5, data), response()])
                with self.assertRaises(M.ObsWebSocketError):
                    client.req('GetVersion', timeout=3)

    def test_event_flood_has_message_bound_even_with_frozen_clock(self):
        client = self.connect()
        self.sock.messages.extend([event()] * 1000 + [response()])
        with self.assertRaises(M.ObsWebSocketError):
            client.req('GetVersion', timeout=3)
        self.assertEqual(len(self.sock.messages), 1001 - M.MAX_MESSAGES)

    def test_message_and_aggregate_byte_caps_refuse(self):
        for aggregate in (False, True):
            with self.subTest(aggregate=aggregate):
                self.setUp()
                client = self.connect()
                if aggregate:
                    big_event = event(eventData={'x': 'a' * (M.MAX_MESSAGE_BYTES - 200)})
                    self.sock.messages.extend([big_event] * M.MAX_MESSAGES)
                else:
                    self.sock.messages.append('x' * (M.MAX_MESSAGE_BYTES + 1))
                with self.assertRaises(M.ObsWebSocketError):
                    client.req('GetVersion', timeout=3)

    def test_bad_outbound_request_never_sends(self):
        client = self.connect()
        invalid = [('GetVersion', []), ('GetVersion', {'x': float('nan')}),
                   ('GetVersion', {'x': float('inf')}), ('GetVersion', {1: 'not string key'}),
                   ('GetVersion', {'x': '\ud800'}), ('', {}), (None, {}),
                   ('GetVersion', {'x': 'x' * 70000})]
        for kind, data in invalid:
            with self.subTest(kind=kind), self.assertRaises(M.ObsWebSocketError):
                client.req(kind, data, timeout=3)
        self.assertEqual(len(self.sock.sent), 1)

    def test_bad_request_timeout_does_not_send(self):
        client = self.connect()
        for timeout in (True, None, 0, -1, float('nan'), float('inf'), 61):
            with self.subTest(timeout=timeout), self.assertRaises(M.ObsWebSocketError):
                client.req('GetVersion', timeout=timeout)
        self.assertEqual(len(self.sock.sent), 1)

    def test_close_is_idempotent_and_prevents_requests(self):
        client = self.connect()
        client.close()
        client.close()
        self.assertEqual(self.sock.close_calls, 1)
        with self.assertRaises(M.ObsWebSocketError):
            client.req('GetVersion', timeout=3)
        self.assertEqual(len(self.sock.sent), 1)

    def test_close_failure_retains_socket_for_cleanup_retry(self):
        client = self.connect()
        self.sock.close_error = RuntimeError(PASSWORD)
        with self.assertRaises(M.ObsWebSocketError) as caught:
            client.close()
        self.assert_sanitized(caught.exception)
        self.sock.close_error = None
        client.close()
        self.assertEqual(self.sock.close_calls, 2)

    def test_consumer_can_close_after_receive_failure_without_losing_owner(self):
        client = self.connect()
        self.sock.recv_error = RuntimeError(PASSWORD)
        with self.assertRaises(M.ObsWebSocketError) as caught:
            client.req('GetVersion', timeout=3)
        self.assert_sanitized(caught.exception)
        self.assertEqual(self.sock.close_calls, 0)
        client.close()
        self.assertEqual(self.sock.close_calls, 1)


class RawSocket:
    def __init__(self):
        self.timeout = None
        self.timeouts, self.sent = [], []
        self.closed = False
        self.connect_error = None

    def settimeout(self, seconds):
        self.timeout = seconds
        self.timeouts.append(seconds)

    def gettimeout(self):
        return self.timeout

    def connect(self, address):
        self.address = address
        if self.connect_error:
            raise self.connect_error

    def recv(self, size):
        return b'x' * min(size, 16)

    def send(self, data):
        self.sent.append(data)
        return min(len(data), 16)

    def close(self):
        self.closed = True


class NativeBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(M, 'bounded OBS websocket connector is missing')
        self.clock = Clock()
        self.raw = RawSocket()

    def test_raw_wire_resets_deadline_for_every_partial_io(self):
        budget = M.WireSocket(self.raw, clock=self.clock)
        budget.begin(5)
        self.assertEqual(budget.recv(100), b'x' * 16)
        self.clock.now = 2
        self.assertEqual(budget.send(b'small'), 5)
        self.assertEqual(self.raw.timeouts, [5, 3])
        self.clock.now = 5
        with self.assertRaises(M.ObsWebSocketError):
            budget.recv(1)

    def test_raw_wire_caps_control_frame_flood_and_bytes_before_large_read(self):
        budget = M.WireSocket(self.raw, clock=self.clock)
        budget.begin(5)
        for _ in range(M.MAX_IO_OPERATIONS):
            budget.recv(1)
        with self.assertRaises(M.ObsWebSocketError):
            budget.recv(1)
        budget.begin(5)
        seen = []
        def huge(size):
            seen.append(size)
            return b'x' * size
        self.raw.recv = huge
        budget.recv(M.MAX_WIRE_BYTES * 100)
        self.assertLessEqual(seen[0], M.MAX_WIRE_BYTES)
        with self.assertRaises(M.ObsWebSocketError):
            budget.recv(1)

    def test_default_dial_closes_raw_socket_on_tcp_failure(self):
        self.raw.connect_error = RuntimeError(PASSWORD)
        fake = types.SimpleNamespace(WebSocket=mock.Mock())
        with mock.patch.dict(sys.modules, {'websocket': fake}), \
                mock.patch.object(M.socket, 'socket', return_value=self.raw):
            with self.assertRaises(M.ObsWebSocketError) as caught:
                M.connect(PASSWORD, timeout=5, clock=self.clock)
        self.assertNotIn(PASSWORD, str(caught.exception))
        self.assertTrue(self.raw.closed)

    def test_default_dial_uses_owned_tcp_socket_and_no_redirect_proxy(self):
        owner = self
        class WebSocket(Socket):
            def __init__(self, **kwargs):
                super().__init__(owner.clock, [hello(), identified()])
                owner.ws = self

            def connect(self, url, **kwargs):
                owner.assertEqual(url, 'ws://127.0.0.1:4471')
                owner.assertEqual(kwargs['subprotocols'], ['obswebsocket.json'])
                owner.assertEqual(kwargs['redirect_limit'], 0)
                owner.assertIsInstance(kwargs['socket'], M.WireSocket)

            def shutdown(self):
                self.close()

            def getstatus(self):
                return 101

            def getsubprotocol(self):
                return 'obswebsocket.json'
        fake = types.SimpleNamespace(WebSocket=WebSocket)
        with mock.patch.dict(sys.modules, {'websocket': fake}), \
                mock.patch.object(M.socket, 'socket', return_value=self.raw):
            client = M.connect(PASSWORD, timeout=5, clock=self.clock)
            self.assertEqual(self.raw.address, ('127.0.0.1', 4471))
            client.close()
        self.assertTrue(self.raw.closed)

    def test_http_redirect_or_wrong_subprotocol_never_reaches_authentication(self):
        for status, protocol in ((302, 'obswebsocket.json'), (True, 'obswebsocket.json'),
                                 (101, None), (101, 'obswebsocket.msgpack')):
            with self.subTest(status=status, protocol=protocol):
                self.setUp()
                ws = mock.Mock()
                ws.getstatus.return_value = status
                ws.getsubprotocol.return_value = protocol
                fake = types.SimpleNamespace(WebSocket=mock.Mock(return_value=ws))
                with mock.patch.dict(sys.modules, {'websocket': fake}), \
                        mock.patch.object(M.socket, 'socket', return_value=self.raw):
                    with self.assertRaises(M.ObsWebSocketError):
                        M.connect(PASSWORD, timeout=5, clock=self.clock)
                self.assertTrue(self.raw.closed)
                ws.recv.assert_not_called()
                ws.send.assert_not_called()

    def test_failed_default_library_import_never_opens_socket(self):
        with mock.patch.dict(sys.modules, {'websocket': None}), \
                mock.patch.object(M.socket, 'socket') as factory:
            with self.assertRaises(M.ObsWebSocketError):
                M.connect(PASSWORD, timeout=5, clock=self.clock)
        factory.assert_not_called()

    def test_zero_progress_raw_writes_are_bounded(self):
        self.raw.send = lambda _data: 0
        budget = M.WireSocket(self.raw, clock=self.clock)
        budget.begin(5)
        for _ in range(M.MAX_IO_OPERATIONS):
            self.assertEqual(budget.send(b'x'), 0)
        with self.assertRaises(M.ObsWebSocketError):
            budget.send(b'x')

    def test_default_dial_closes_raw_and_websocket_after_http_upgrade_failure(self):
        ws = mock.Mock()
        ws.connect.side_effect = RuntimeError(PASSWORD)
        fake = types.SimpleNamespace(WebSocket=mock.Mock(return_value=ws))
        with mock.patch.dict(sys.modules, {'websocket': fake}), \
                mock.patch.object(M.socket, 'socket', return_value=self.raw):
            with self.assertRaises(M.ObsWebSocketError):
                M.connect(PASSWORD, timeout=5, clock=self.clock)
        self.assertTrue(self.raw.closed)
        ws.shutdown.assert_called_once()


if __name__ == '__main__':
    unittest.main()
