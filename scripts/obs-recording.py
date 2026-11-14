# SPDX-License-Identifier: MIT
"""Bounded OBS recording transitions, not media or acquisition acceptance.

Retain the object before start(). A lost StartRecord reply can still leave an
in-flight start: only an observed active -> dispatched stop -> observed inactive
sequence releases recording ownership. Cleanup may resume observation, but never
redispatches a start or stop. Failures remain sticky even if cleanup succeeds.
The client must honor whole-request deadlines, as obs-websocket.py does.
"""
import time

TRANSITION_SECONDS = 10
POLL_ATTEMPTS = 200
RPC_SECONDS = 5


class ObsRecording:
    def __init__(self, case, client, process, *, clock=time.monotonic, sleep=time.sleep):
        self.case, self.client, self.process = case, client, process
        self.clock, self.sleep = clock, sleep
        self.start_attempted = self.start_dispatched = self.stop_dispatched = False
        self.responsible = self.active_observed = self.inactive_observed = False
        self.stop_response = None
        self.errors = []

    @property
    def recording_inactive(self):
        return not self.start_dispatched or (self.inactive_observed and not self.responsible)

    def _error(self, phase, error):
        item = dict(phase=phase, error_type=type(error).__name__[:128])
        if item not in self.errors and len(self.errors) < 16:
            self.errors.append(item)

    def _budget(self, end):
        self.case.check()
        if self.process.poll() is not None:
            raise RuntimeError('owned OBS exited during recording transition')
        remaining = end - self.clock()
        if remaining <= 0:
            raise RuntimeError('OBS recording transition deadline')
        return min(RPC_SECONDS, remaining)

    def _wait(self, active, end):
        for _ in range(POLL_ATTEMPTS):
            status = self.client.req('GetRecordStatus', timeout=self._budget(end))
            self._budget(end)
            if not isinstance(status, dict) or type(status.get('outputActive')) is not bool:
                raise RuntimeError('invalid OBS recording status')
            observed = status['outputActive']
            self.active_observed |= observed
            if observed is active:
                if not active:
                    self.inactive_observed, self.responsible = True, False
                return
            self.sleep(min(0.05, self._budget(end)))
        raise RuntimeError('OBS recording transition attempt bound')

    def start(self):
        if self.start_attempted:
            raise RuntimeError('one OBS recording start attempt required')
        self.start_attempted = True
        end = self.clock() + TRANSITION_SECONDS
        try:
            timeout = self._budget(end)
            # Record ownership before dispatch, not after an unreliable reply.
            self.start_dispatched = self.responsible = True
            self.client.req('StartRecord', timeout=timeout)
            self._budget(end)
            self._wait(True, end)
        except BaseException as error:
            self._error('start', error)
            raise

    def stop(self):
        if not self.responsible:
            return self.stop_response
        end = self.clock() + TRANSITION_SECONDS
        if not self.active_observed:
            try:
                # An initial inactive read cannot cancel an in-flight start.
                self._wait(True, end)
            except BaseException as error:
                self._error('active', error)
                raise
        failure = None
        try:
            timeout = self._budget(end)
            if not self.stop_dispatched:
                self.stop_dispatched = True
                self.stop_response = self.client.req('StopRecord', timeout=timeout)
                self._budget(end)
        except BaseException as error:
            self._error('stop', error)
            failure = error
        try:
            self._wait(False, end)
        except BaseException as error:
            self._error('inactive', error)
            if failure is None:
                failure = error
        if failure is not None:
            raise failure
        return self.stop_response

    def receipt(self):
        return dict(
            start_attempted=self.start_attempted, start_dispatched=self.start_dispatched,
            active_observed=self.active_observed, stop_dispatched=self.stop_dispatched,
            inactive_observed=self.inactive_observed, responsible=self.responsible,
            recording_inactive=self.recording_inactive,
            recording_complete=bool(self.active_observed and self.stop_dispatched
                                    and self.inactive_observed and not self.errors),
            errors=list(self.errors), acquisition_accepted=False, qualified=False)
