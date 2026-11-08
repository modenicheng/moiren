# SPDX-License-Identifier: MIT
"""OBS setup/normal-close boundary; no recording or acquisition acceptance.

Retain this object on the case BEFORE calling open(), including on exceptions.
connect(password, timeout=seconds) must authenticate and return a client whose
req(kind, data=None, timeout=seconds) honors a whole-request deadline. Failed
connections dispose unreturned sockets, or raise with cleanup_owner retaining
the exact close-capable owner for the case's failure cleanup. Client.close() and
close_window(exact_process) must be bounded by the enclosing case watchdog;
close_window only requests normal close, while case.join performs the join.

The caller owns Start/StopRecord and supplies recording_inactive=True only after
observing inactive (or proving no recording was dispatched). Unknown launch or
live-owner failures deliberately keep the lease: no automatic release, retry,
process discovery, termination or stale-lock recovery is provided here.
"""
import importlib.util
from pathlib import Path
import time

SPEC = importlib.util.spec_from_file_location('obs_acquisition_reuse',
                                             Path(__file__).with_name('obs-reuse.py'))
REUSE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(REUSE)

DAWN_ENDPOINT = '{0.0.0.00000000}.{65148f19-e008-4b5b-b289-9dc1f426d9a1}'
READINESS_SECONDS = 35
READINESS_ATTEMPTS = 140
CONFIGURATION_SECONDS = 10
RPC_SECONDS = 5
JOIN_SECONDS = 12


class ObsAcquisition:
    def __init__(self, case, connect, close_window, *, reuse=None,
                 clock=time.monotonic, sleep=time.sleep):
        # Injectable reuse/clock boundaries are for file-only tests. A deployed
        # caller uses the default fixed-path target/SID/hash/owner guards.
        self.case, self.connect, self.close_window = case, connect, close_window
        self.reuse = REUSE.ReusedObs() if reuse is None else reuse
        self.clock, self.sleep = clock, sleep
        self.client = None
        self.open_attempted = self.close_attempted = self.finished = False
        self.configured = self.normal_joined = False
        self.readiness_attempts = 0
        self.errors = []

    @property
    def process(self):
        # ReusedObs retains the returned owner before any readiness/config RPC.
        return self.reuse.process

    def _error(self, phase, error):
        # Keep failures sticky and bounded without copying websocket credentials
        # or arbitrarily large exception text into the acquisition receipt.
        item = dict(phase=phase, error_type=type(error).__name__[:128])
        if item not in self.errors and len(self.errors) < 16:
            self.errors.append(item)

    def _budget(self, end, phase):
        self.case.check()
        remaining = end - self.clock()
        if remaining <= 0:
            raise RuntimeError(f'OBS {phase} deadline')
        return min(RPC_SECONDS, remaining)

    def _alive(self):
        if self.process.poll() is not None:
            raise RuntimeError('owned OBS exited before setup completed')

    def _close_socket(self):
        if self.client is not None:
            self.client.close()
            self.client = None

    def _ready(self):
        end = self.clock() + READINESS_SECONDS
        for _ in range(READINESS_ATTEMPTS):
            self._alive()
            timeout = self._budget(end, 'readiness')
            self.readiness_attempts += 1
            try:
                self.client = self.connect(self.reuse.password, timeout=timeout)
                self.client.req('GetVersion', timeout=self._budget(end, 'readiness'))
                self._budget(end, 'readiness')
                self._alive()
                return
            except Exception as error:
                pending = getattr(error, 'cleanup_owner', None)
                if pending is not None:
                    # A handshake whose cleanup failed is not a retryable
                    # connection refusal. Keep its exact socket before escape.
                    self.client = pending
                    raise
                # A failed connection/GetVersion may be retried; input mutation
                # never is. A socket-close failure escapes and retains its owner.
                self._close_socket()
                self._alive()
                remaining = self._budget(end, 'readiness')
                self.sleep(min(0.25, remaining))
        raise RuntimeError('OBS readiness attempt bound')

    def _configure(self):
        steps = [
            ('CreateInput', dict(sceneName='Isolated', inputName='DawnOutput',
                                 inputKind='wasapi_output_capture', inputSettings=dict(
                                     device_id=DAWN_ENDPOINT, use_device_timing=True),
                                 sceneItemEnabled=True)),
            ('SetInputAudioTracks', dict(inputName='DawnOutput', inputAudioTracks={
                str(i): i == 1 for i in range(1, 7)})),
            ('SetInputAudioMonitorType', dict(inputName='DawnOutput',
                                            monitorType='OBS_MONITORING_TYPE_NONE')),
            ('SetInputAudioSyncOffset', dict(inputName='DawnOutput', inputAudioSyncOffset=0)),
            ('SetInputVolume', dict(inputName='DawnOutput', inputVolumeMul=1.0))]
        end = self.clock() + CONFIGURATION_SECONDS
        for kind, data in steps:
            self._alive()
            self.client.req(kind, data, timeout=self._budget(end, 'configuration'))
            self._budget(end, 'configuration')
            self._alive()
        self.configured = True

    def open(self):
        if self.open_attempted:
            raise RuntimeError('one OBS setup attempt required')
        self.open_attempted = True
        try:
            self.case.check()
            run = REUSE.no_links(self.case.root)
            receipt = run / 'obs-acquisition-result.json'
            if receipt.exists() or receipt.is_symlink():
                raise ValueError('prior OBS acquisition evidence must not be overwritten')
            self.reuse.prepare(self.case.root)
            self.reuse.launch(self.case.launch)
            self._ready()
            self._configure()
            return self.client
        except BaseException as error:
            self._error('setup', error)
            raise

    def finish(self, *, recording_inactive):
        """Archive/release only a terminal owner; never accept an acquisition.

        After a failed close/join this can archive a subsequently terminal owner,
        but it never repeats the close and its sticky failure remains. A live or
        uncertain owner keeps the lease for the case's separate failure cleanup.
        """
        if self.finished:
            raise RuntimeError('OBS boundary already finalized')
        try:
            if recording_inactive is not True:
                raise RuntimeError('explicit recording inactive witness required')
            if self.reuse.launch_attempted and self.process is None:
                raise RuntimeError('uncertain OBS launch retains lease')
            if self.reuse.lease is None or self.reuse.run is None:
                raise RuntimeError('OBS session is not prepared')
            live = self.process is not None and self.process.poll() is None
            if live and self.close_attempted:
                raise RuntimeError('OBS normal close already attempted; owner still live')
            if not self.close_attempted:
                self._close_socket()
                # A failed socket cleanup has not sent WM_CLOSE. Retrying only
                # that retained socket may precede the one normal-close attempt.
                self.close_attempted = True
                if live:
                    self.close_window(self.process)
                    code = self.case.join(self.process, JOIN_SECONDS)
                    if type(code) is not int or self.process.poll() != code:
                        raise RuntimeError('OBS join did not confirm the retained owner exit')
                    self.normal_joined = True
                else:
                    self._error('exit', RuntimeError('OBS exited before normal close'))
            # A failed socket close remains owned even if separate held-process
            # cleanup later observes exit. Never drop it merely to release lease.
            self._close_socket()
            if self.process is not None and self.process.poll() is None:
                raise RuntimeError('owned OBS still live; preserve lease')
            obs = self.reuse.finish()
            result = dict(
                boundary_complete=bool(self.configured and self.normal_joined
                                       and obs['obs_exit_success'] and not self.errors),
                acquisition_accepted=False, qualified=False, application_copies=0,
                readiness_attempts=self.readiness_attempts, errors=list(self.errors), obs=obs)
            REUSE.save(self.case.root / 'obs-acquisition-result.json', result)
            self.finished = True
            return result
        except BaseException as error:
            self._error('finish', error)
            raise
