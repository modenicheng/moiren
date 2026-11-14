# SPDX-License-Identifier: MIT
"""Host delivery uses real closed ZIP bytes, never a native acceptance claim."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest import mock
import zipfile

SOURCE = Path(__file__).resolve().parents[1] / 'vad-delivery.py'
MODULE = None
if SOURCE.exists():
    SPEC = importlib.util.spec_from_file_location('vad_delivery', SOURCE)
    MODULE = importlib.util.module_from_spec(SPEC)
    SPEC.loader.exec_module(MODULE)


def encoded(value):
    return json.dumps(value, separators=(',', ':'), allow_nan=False).encode('utf-8')


def pin(path, data):
    return {'Path': str(path), 'Bytes': len(data),
            'Sha256': hashlib.sha256(data).hexdigest().upper()}


def fixture(directory):
    run = r'D:\MoirenTest\fixture-delivery'
    nonce = 'a' * 32
    manifest_pin = pin(r'D:\MoirenTest\inputs.json', b'{}')
    result = encoded({'ControllerComplete': True, 'Nonce': nonce, 'Safe': False,
                      'SchemaVersion': 1, 'Manifest': manifest_pin,
                      'NativePassed': False, 'Qualified': False})
    result_pin = pin(run + r'\controller-result.json', result)
    provenance = encoded({'SchemaVersion': 1, 'Nonce': nonce, 'ResultPin': result_pin,
                          'ResultJson': result.decode(),
                          'ManifestPin': manifest_pin,
                          'InputJson': encoded({'ManifestPin': manifest_pin}).decode(), 'SourcePins': [],
                          'StagedScriptPins': [], 'TerminalPins': [result_pin],
                          'CollectorProvenance': 'InjectedCollectorSource',
                          'Transferred': False, 'NativePassed': False, 'Qualified': False})
    payloads = {'run/': b'', 'run/controller-result.json': result,
                'run/empty.log': b'', 'export/provenance.json': provenance}
    members = [{'Name': name, 'Kind': 'Directory' if name.endswith('/') else 'File',
                'Bytes': len(data), 'Sha256': None if name.endswith('/') else pin('', data)['Sha256']}
               for name, data in payloads.items()]
    inventory = encoded({'SchemaVersion': 1, 'Nonce': nonce, 'ResultPin': result_pin,
                         'Limits': {'RunRelativeDepth': 8, 'Members': 512,
                                    'FileBytes': 16777216, 'TotalBytes': 67108864},
                         'Members': members})
    with zipfile.ZipFile(directory / 'archive.zip', 'x', compression=zipfile.ZIP_STORED) as archive:
        for name, data in payloads.items():
            archive.writestr(name, data)
        archive.writestr('export/inventory.json', inventory)
    (directory / 'inventory.json').write_bytes(inventory)
    ack = encoded({'SchemaVersion': 1, 'Meaning': 'LOCAL_ARCHIVE_VERIFICATION_ONLY',
                   'Nonce': nonce, 'ResultPin': result_pin,
                   'ArchivePin': pin(run + '.zip', (directory / 'archive.zip').read_bytes()),
                   'InventoryPin': pin(run + '.zip.inventory.json', inventory),
                   'Transferred': False, 'NativePassed': False, 'Qualified': False})
    (directory / 'local-ack.json').write_bytes(ack)
    return run, pin('', ack)['Sha256']


class Delivery(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(MODULE, 'host delivery verifier is not implemented')
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name).resolve()
        self.run, self.ack_hash = fixture(self.root)

    def verify(self, run=None, expected=None):
        return MODULE.verify_delivery(self.root, run or self.run, expected or self.ack_hash)

    def test_real_zip_every_member_and_readback_host_receipt(self):
        receipt = self.verify()
        self.assertEqual(receipt['Meaning'], 'HOST_ARCHIVE_DELIVERY_VERIFIED')
        self.assertTrue(receipt['Transferred'])
        self.assertFalse(receipt['NativePassed'])
        self.assertFalse(receipt['Qualified'])
        self.assertEqual(receipt['RunDirectory'], self.run)
        self.assertEqual(receipt['Target'], 'moiren-lan')
        self.assertEqual(receipt['SourceLocalAckPin']['Sha256'], self.ack_hash)
        self.assertEqual(json.loads((self.root / 'host-delivery.json').read_bytes()), receipt)
        with self.assertRaises((ValueError, FileExistsError)):
            self.verify()

    def test_independent_ack_hash_and_original_run_are_required(self):
        for change in ({'expected': '0' * 64}, {'run': r'D:\MoirenTest\foreign'},
                       {'run': r'D:\MoirenTest\..\foreign'}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                self.verify(**change)
        self.assertFalse((self.root / 'host-delivery.json').exists())

    def test_downloaded_archive_inventory_and_ack_bytes_are_pinned(self):
        for name in ('archive.zip', 'inventory.json', 'local-ack.json'):
            with self.subTest(name=name):
                path = self.root / name
                original = path.read_bytes()
                try:
                    path.write_bytes(original + b' ')
                    with self.assertRaises(ValueError):
                        self.verify()
                finally:
                    path.write_bytes(original)

    def test_duplicate_json_fields_refuse_even_with_fresh_external_hash(self):
        path = self.root / 'local-ack.json'
        value = path.read_bytes().replace(b'"SchemaVersion":1', b'"SchemaVersion":1,"SchemaVersion":1')
        path.write_bytes(value)
        with self.assertRaises(ValueError):
            self.verify(expected=pin('', value)['Sha256'])

    def test_unknown_transfer_or_qualification_flags_cannot_be_promoted(self):
        path = self.root / 'local-ack.json'
        original = json.loads(path.read_bytes())
        for key in ('Transferred', 'NativePassed', 'Qualified'):
            value = dict(original)
            value[key] = True
            data = encoded(value)
            path.write_bytes(data)
            with self.subTest(key=key), self.assertRaises(ValueError):
                self.verify(expected=pin('', data)['Sha256'])

    def test_corrupt_extra_missing_duplicate_and_traversal_zip_members_refuse(self):
        archive_path = self.root / 'archive.zip'
        original = archive_path.read_bytes()
        ack_path = self.root / 'local-ack.json'
        original_ack = json.loads(ack_path.read_bytes())
        for name in ('unexpected.log', 'run/empty.log', '../escape.log', 'RUN/empty.log'):
            archive_path.write_bytes(original)
            with zipfile.ZipFile(archive_path, 'a') as archive:
                archive.writestr(name, b'bad')
            ack = dict(original_ack)
            ack['ArchivePin'] = pin(self.run + '.zip', archive_path.read_bytes())
            data = encoded(ack)
            ack_path.write_bytes(data)
            with self.subTest(name=name), self.assertRaises(ValueError):
                self.verify(expected=pin('', data)['Sha256'])

    def test_receive_requires_confirmation_before_transport_or_output_creation(self):
        output = self.root / 'not-created'
        calls = []
        def transport(*args, **kwargs):
            calls.append((args, kwargs))
            raise AssertionError('must not call transport')
        with self.assertRaises(ValueError):
            MODULE.receive_delivery(self.run, self.ack_hash, output,
                                    user_reboot_confirmed=False, download=transport)
        self.assertFalse(output.exists())
        self.assertFalse(calls)

    def test_receive_downloads_once_in_order_then_verifies_real_bytes(self):
        output = self.root / 'received'
        calls = []
        remote = {self.run + '.archive-local.json': (self.root / 'local-ack.json').read_bytes(),
                  self.run + '.zip.inventory.json': (self.root / 'inventory.json').read_bytes(),
                  self.run + '.zip': (self.root / 'archive.zip').read_bytes()}
        def transport(path, destination, **options):
            self.assertTrue(options['user_reboot_confirmed'])
            calls.append(path)
            with Path(destination).open('xb') as stream:
                stream.write(remote[path])
            return pin(destination, remote[path])
        receipt = MODULE.receive_delivery(self.run, self.ack_hash, output,
                                          user_reboot_confirmed=True, download=transport)
        self.assertEqual(calls, list(remote))
        self.assertTrue(receipt['Transferred'])
        self.assertEqual(receipt['HostArchivePin']['Path'], str(output / 'archive.zip'))

    def test_receive_refuses_wrong_ack_before_inventory_or_archive_transport(self):
        output = self.root / 'rejected'
        calls = []
        def transport(path, destination, **options):
            calls.append(path)
            with Path(destination).open('xb') as stream:
                stream.write((self.root / 'local-ack.json').read_bytes())
        with self.assertRaises(ValueError):
            MODULE.receive_delivery(self.run, '0' * 64, output,
                                    user_reboot_confirmed=True, download=transport)
        self.assertEqual(calls, [self.run + '.archive-local.json'])

    def test_strict_scalar_types_and_path_binding_before_archive(self):
        path = self.root / 'local-ack.json'
        original = json.loads(path.read_bytes())
        for field, replacement in (('SchemaVersion', True), ('Transferred', 0),
                                   ('Nonce', 123), ('ResultPin', {'Path': r'C:\foreign', 'Bytes': 0,
                                                               'Sha256': '0' * 64})):
            changed = dict(original)
            changed[field] = replacement
            raw = encoded(changed)
            path.write_bytes(raw)
            with self.subTest(field=field), self.assertRaises(ValueError):
                self.verify(expected=pin('', raw)['Sha256'])

    def test_actual_named_source_pins_bind_input_snapshot_without_opening_sources(self):
        archive_path = self.root / 'archive.zip'
        with zipfile.ZipFile(archive_path) as archive:
            payloads = {item.filename: archive.read(item) for item in archive.infolist()}
        source = {**pin(r'D:\absent-original\closed-export.ps1', b'# source'), 'Name': 'closed-export.ps1'}
        provenance = json.loads(payloads['export/provenance.json'])
        provenance['SourcePins'] = [source]
        inputs = json.loads(provenance['InputJson'])
        inputs['Pins'] = {'Sources': [source]}
        provenance['InputJson'] = encoded(inputs).decode()
        payloads['export/provenance.json'] = encoded(provenance)
        inventory = json.loads(payloads['export/inventory.json'])
        for row in inventory['Members']:
            if row['Name'] == 'export/provenance.json':
                row['Bytes'] = len(payloads[row['Name']])
                row['Sha256'] = pin('', payloads[row['Name']])['Sha256']
        payloads['export/inventory.json'] = encoded(inventory)
        with zipfile.ZipFile(archive_path, 'w') as archive:
            for name, raw in payloads.items():
                archive.writestr(name, raw)
        (self.root / 'inventory.json').write_bytes(payloads['export/inventory.json'])
        ack = json.loads((self.root / 'local-ack.json').read_bytes())
        ack['ArchivePin'] = pin(self.run + '.zip', archive_path.read_bytes())
        ack['InventoryPin'] = pin(self.run + '.zip.inventory.json', payloads['export/inventory.json'])
        raw = encoded(ack)
        (self.root / 'local-ack.json').write_bytes(raw)
        self.assertTrue(self.verify(expected=pin('', raw)['Sha256'])['Transferred'])

    def test_complete_delivery_returns_and_verifies_target_ack_without_promoting_acceptance(self):
        output = self.root / 'complete'
        calls = []
        uploads = []
        files = {self.run + '.archive-local.json': (self.root / 'local-ack.json').read_bytes(),
                 self.run + '.zip.inventory.json': (self.root / 'inventory.json').read_bytes(),
                 self.run + '.zip': (self.root / 'archive.zip').read_bytes()}
        def download(remote, destination, **options):
            calls.append(remote)
            with Path(destination).open('xb') as stream:
                stream.write(files[remote])
        finalizer = r'D:\MoirenTest\reviewed\delivery-finalize.ps1'
        dependencies = [pin('D:\\MoirenTest\\reviewed\\' + name, b'# source')
                        for name in ('closed-export.ps1', 'finite-tool.ps1')]
        def finalize(run, raw, ack_sha, finalizer_path, finalizer_sha, **options):
            self.assertEqual(run, self.run)
            self.assertEqual(ack_sha, self.ack_hash)
            self.assertEqual(finalizer_path, finalizer)
            self.assertEqual(finalizer_sha, 'B' * 64)
            self.assertEqual(options['dependency_pins'], dependencies)
            uploads.append(raw)
            host = json.loads(raw)
            target = dict(host, Meaning='TARGET_ARCHIVE_DELIVERY_ACKNOWLEDGED',
                          DeliveryReceiptPin=pin(run + '.delivery-host.json', raw),
                          DeliveryReceiptJson=raw.decode())
            target_raw = encoded(target)
            files[run + '.archive-delivered.json'] = target_raw
            return pin(run + '.archive-delivered.json', target_raw)
        receipt = MODULE.deliver_run(self.run, self.ack_hash, output, finalizer, 'B' * 64,
                                     dependency_pins=dependencies, user_reboot_confirmed=True,
                                     download=download, finalize=finalize)
        self.assertTrue(receipt['Transferred'])
        self.assertFalse(receipt['Qualified'])
        self.assertFalse(receipt['NativePassed'])
        self.assertEqual(calls, list(files))
        self.assertEqual(len(uploads), 1)
        self.assertEqual((output / 'host-delivery.json').read_bytes(), uploads[0])
        self.assertEqual(json.loads((output / 'delivery-complete.json').read_bytes()), receipt)

    def test_complete_delivery_requires_confirmation_before_any_transport_or_output(self):
        output = self.root / 'not-started'
        with self.assertRaises(ValueError):
            MODULE.deliver_run(self.run, self.ack_hash, output,
                               r'D:\MoirenTest\reviewed\delivery-finalize.ps1', 'B' * 64,
                               dependency_pins=[], user_reboot_confirmed=False)
        self.assertFalse(output.exists())

    def test_receive_failure_drains_original_pending_transport_without_retry(self):
        class PendingOwner:
            closed = False
            process = object()
            observations = 0
            def observe(self):
                self.observations += 1
                if self.observations == 1:
                    raise OSError('transient observation failure')
                self.closed = self.observations >= 3
        owner = PendingOwner()
        error = RuntimeError('original sticky transport failure')
        error.owner = owner
        calls = []
        def download(*args, **options):
            calls.append(args)
            raise error
        with self.assertRaises(RuntimeError) as raised:
            MODULE.receive_delivery(self.run, self.ack_hash, self.root / 'pending',
                                    user_reboot_confirmed=True, download=download)
        self.assertIs(raised.exception, error)
        self.assertTrue(owner.closed, 'caller abandoned retained transport owner')
        self.assertEqual(owner.observations, 3)
        self.assertEqual(len(calls), 1)

    def test_failed_finalize_wrong_target_path_and_foreign_nonce_never_complete(self):
        for mode in ('pending-finalize', 'wrong-path', 'foreign-nonce'):
            with self.subTest(mode=mode):
                output = self.root / mode
                calls = []
                files = {self.run + '.archive-local.json': (self.root / 'local-ack.json').read_bytes(),
                         self.run + '.zip.inventory.json': (self.root / 'inventory.json').read_bytes(),
                         self.run + '.zip': (self.root / 'archive.zip').read_bytes()}
                class Owner:
                    closed = False
                    def observe(self):
                        self.closed = True
                owner = Owner()
                def download(remote, destination, **options):
                    calls.append(remote)
                    with Path(destination).open('xb') as stream:
                        stream.write(files[remote])
                def finalize(run, raw, ack_sha, source, source_sha, **options):
                    if mode == 'pending-finalize':
                        error = RuntimeError('sticky failed finalization')
                        error.owner = owner
                        raise error
                    if mode == 'wrong-path':
                        return pin(r'D:\MoirenTest\foreign.archive-delivered.json', b'{}')
                    host = json.loads(raw)
                    target = dict(host, Meaning='TARGET_ARCHIVE_DELIVERY_ACKNOWLEDGED',
                                  Nonce='b' * 32, DeliveryReceiptPin=pin(run + '.delivery-host.json', raw),
                                  DeliveryReceiptJson=raw.decode())
                    target_raw = encoded(target)
                    files[run + '.archive-delivered.json'] = target_raw
                    return pin(run + '.archive-delivered.json', target_raw)
                finalizer = r'D:\MoirenTest\reviewed\delivery-finalize.ps1'
                dependencies = [pin('D:\\MoirenTest\\reviewed\\' + name, b'# source')
                                for name in ('closed-export.ps1', 'finite-tool.ps1')]
                with self.assertRaises((ValueError, RuntimeError)):
                    MODULE.deliver_run(self.run, self.ack_hash, output, finalizer, 'B' * 64,
                                       dependency_pins=dependencies, user_reboot_confirmed=True,
                                       download=download, finalize=finalize)
                self.assertFalse((output / 'delivery-complete.json').exists())
                self.assertEqual(len(calls), 4 if mode == 'foreign-nonce' else 3)
                if mode == 'pending-finalize':
                    self.assertTrue(owner.closed)

    @unittest.skipUnless(__import__('os').name == 'nt', 'Windows file-share exclusion')
    def test_original_file_writer_is_excluded_through_receipt_publication(self):
        original = MODULE.validate_provenance
        mutations = []
        def inspect(*args):
            path = self.root / 'archive.zip'
            try:
                with path.open('r+b') as writer:
                    mutations.append('writer-opened')
            except OSError:
                mutations.append('writer-blocked')
            return original(*args)
        with mock.patch.object(MODULE, 'validate_provenance', side_effect=inspect):
            self.verify()
        self.assertEqual(mutations, ['writer-blocked'])

    def test_original_open_handle_identity_is_checked_against_path(self):
        with __import__('contextlib').ExitStack() as stack:
            path = self.root / 'archive.zip'
            stream = MODULE.open_input(stack, path, MODULE.ARCHIVE_CAP)
            actual = MODULE.os.stat
            def wrong_identity(candidate, **kwargs):
                info = actual(candidate, **kwargs)
                if Path(candidate) == path:
                    return type('ChangedIdentity', (), {'st_dev': info.st_dev, 'st_ino': info.st_ino + 1,
                                                       'st_mode': info.st_mode,
                                                       'st_file_attributes': getattr(info, 'st_file_attributes', 0)})()
                return info
            with mock.patch.object(MODULE.os, 'stat', side_effect=wrong_identity):
                with self.assertRaises(ValueError):
                    MODULE.content_pin(path, stream, MODULE.ARCHIVE_CAP)


if __name__ == '__main__':
    unittest.main()
