# SPDX-License-Identifier: MIT
"""Compose fixed OBS reuse, authenticated RPC and truthful recording cleanup.

Retain this object on the native case before start(). The case owns its exact
process handle and supplies bounded check/launch/join and normal-window-close
callbacks. This module never discovers/kills processes, copies OBS or accepts a
route/recording waveform. An unresolved recording keeps the socket and lease.
"""
import importlib.util
from pathlib import Path
import time


def _load(name, filename):
    spec = importlib.util.spec_from_file_location(name, Path(__file__).with_name(filename))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


ACQUISITION = _load('obs_capture_acquisition', 'obs-acquisition.py')
RECORDING = _load('obs_capture_recording', 'obs-recording.py')


class ObsCaptureSession:
    def __init__(self, case, close_window, *, acquisition=None, connect=None,
                 clock=time.monotonic, sleep=time.sleep):
        if acquisition is None:
            if connect is None:
                connect = _load('obs_capture_websocket', 'obs-websocket.py').connect
            acquisition = ACQUISITION.ObsAcquisition(
                case, connect, close_window, clock=clock, sleep=sleep)
        self.case, self.acquisition = case, acquisition
        self.clock, self.sleep = clock, sleep
        self.recording = None
        self.start_attempted = self.finished = False
        self.errors = []

    @property
    def client(self):
        return self.acquisition.client

    def _error(self, phase, error):
        item = dict(phase=phase, error_type=type(error).__name__[:128])
        if item not in self.errors and len(self.errors) < 16:
            self.errors.append(item)

    def start(self):
        if self.start_attempted:
            raise RuntimeError('one OBS capture session start required')
        self.start_attempted = True
        try:
            receipt = ACQUISITION.REUSE.no_links(self.case.root / 'obs-capture-session.json')
            if receipt.exists() or receipt.is_symlink():
                raise ValueError('prior OBS capture session evidence must not be overwritten')
            client = self.acquisition.open()
            # Retain transition responsibility before dispatching StartRecord.
            self.recording = RECORDING.ObsRecording(
                self.case, client, self.acquisition.process, clock=self.clock, sleep=self.sleep)
            self.recording.start()
            return client
        except BaseException as error:
            self._error('start', error)
            raise

    def finish(self):
        if self.finished or not self.start_attempted:
            raise RuntimeError('OBS capture session is not pending finalization')
        try:
            if self.recording is not None:
                try:
                    self.recording.stop()
                except Exception as error:
                    self._error('recording', error)
                if not self.recording.recording_inactive:
                    raise RuntimeError('OBS recording transition unresolved; retain socket and lease')
            boundary = self.acquisition.finish(
                recording_inactive=self.recording is None or self.recording.recording_inactive)
            recording = self.recording.receipt() if self.recording is not None else None
            result = dict(
                capture_lifecycle_complete=bool(boundary['boundary_complete'] and recording
                                                and recording['recording_complete'] and not self.errors),
                boundary=boundary, recording=recording, errors=list(self.errors),
                application_copies=0, acquisition_accepted=False, qualified=False)
            ACQUISITION.REUSE.save(self.case.root / 'obs-capture-session.json', result)
            self.finished = True
            return result
        except BaseException as error:
            self._error('finish', error)
            raise
