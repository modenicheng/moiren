# SPDX-License-Identifier: MIT
"""Receive one closed target archive without extraction or qualification promotion."""
import argparse
from contextlib import ExitStack
import hashlib
import importlib.util
import json
import ntpath
import os
from pathlib import Path
import re
import stat
import time
import zipfile

TARGET = 'moiren-lan'
FILE_CAP = 16 * 1024 * 1024
TOTAL_CAP = 64 * 1024 * 1024
ARCHIVE_CAP = TOTAL_CAP + 1024 * 1024
ACK_CAP = 1024 * 1024
LIMITS = {'RunRelativeDepth': 8, 'Members': 512,
          'FileBytes': FILE_CAP, 'TotalBytes': TOTAL_CAP}
EXTENSIONS = {'.json', '.jsonl', '.log', '.txt', '.sum', '.etl', '.xml', '.csv',
              '.wav', '.raw', '.pcm', '.f32', '.f32le', '.u8', '.u64le',
              '.i64le', '.bin', '.out', '.err', '.stop'}
STAGED = {'finite-tool.ps1', 'task-boundary.ps1', 'probe-boundary.ps1',
          'worker-boundary.ps1', 'worker.ps1'}
MAGIC = {b'\x7fELF', b'\xfe\xed\xfa\xce', b'\xfe\xed\xfa\xcf',
         b'\xce\xfa\xed\xfe', b'\xcf\xfa\xed\xfe', b'\xca\xfe\xba\xbe'}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(data):
    return hashlib.sha256(data).hexdigest().upper()


def hash_text(value):
    require(isinstance(value, str) and re.fullmatch(r'[a-fA-F0-9]{64}', value),
            'independent SHA256 must be exactly 64 hex characters')
    return value.upper()


def canonical_windows_path(value):
    require(isinstance(value, str) and re.match(r'^[A-Z]:\\', value),
            'canonical drive-qualified Windows path required')
    require(ntpath.normpath(value) == value and '/' not in value,
            'noncanonical Windows path')
    parts = value[3:].split('\\')
    require(all(part and part not in ('.', '..') and part.rstrip(' .') == part
                and not re.search(r'[<>:"|?*\x00-\x1f]', part) for part in parts),
            'noncanonical Windows path component')
    return value


def canonical_run(value):
    canonical_windows_path(value)
    require(value.startswith('D:\\MoirenTest\\') and len(value[14:].split('\\')) <= 8,
            'run must be beneath the original D:\\MoirenTest workspace')
    return value


def strict_json(data):
    def fields(pairs):
        value = {}
        names = set()
        for name, item in pairs:
            require(name.casefold() not in names, 'duplicate JSON field')
            names.add(name.casefold())
            value[name] = item
        return value
    try:
        value = json.loads(data.decode('utf-8', errors='strict'), object_pairs_hook=fields,
                           parse_constant=lambda _: require(False, 'nonfinite JSON scalar'))
        pending = [(value, 0)]
        while pending:
            item, depth = pending.pop()
            require(depth <= 32, 'JSON nesting exceeds bounded schema')
            if isinstance(item, dict):
                pending.extend((entry, depth + 1) for entry in item.values())
            elif isinstance(item, list):
                pending.extend((entry, depth + 1) for entry in item)
        require(isinstance(value, dict), 'JSON object required')
        return value
    except (UnicodeError, RecursionError, json.JSONDecodeError) as error:
        raise ValueError('invalid bounded UTF8 JSON') from error


def pin(value, cap=ARCHIVE_CAP, path=None, named=False):
    fields = {'Path', 'Bytes', 'Sha256'} | ({'Name'} if named else set())
    require(isinstance(value, dict) and set(value) == fields,
            'exact path/length/hash pin required')
    canonical_windows_path(value['Path'])
    require(type(value['Bytes']) is int and 0 <= value['Bytes'] <= cap,
            'pin length must be a bounded integer')
    hash_text(value['Sha256'])
    require(path is None or value['Path'] == path, 'original target pin path differs')
    if named:
        require(isinstance(value['Name'], str) and value['Name'] == ntpath.basename(value['Path'])
                and re.fullmatch(r'[A-Za-z0-9][A-Za-z0-9_.-]*\.ps1', value['Name'])
                and not value['Name'].startswith('test-'), 'original named source path differs')
    return value


def same_pin(left, right):
    pin(left)
    pin(right)
    require(left['Path'] == right['Path'] and left['Bytes'] == right['Bytes']
            and hash_text(left['Sha256']) == hash_text(right['Sha256']),
            'original provenance pin differs')


def schema(value):
    require(type(value.get('SchemaVersion')) is int and value['SchemaVersion'] == 1,
            'typed schema version 1 required')


def false_flags(value, transferred=False):
    require(value.get('Transferred') is transferred and value.get('NativePassed') is False
            and value.get('Qualified') is False, 'no transfer/native/qualification promotion')


def safe_local(path, directory=False):
    path = Path(path)
    require(path.is_absolute() and path == Path(os.path.abspath(path)),
            'absolute canonical local evidence path required')
    for item in reversed((path, *path.parents)):
        try:
            attributes = item.lstat()
        except FileNotFoundError:
            continue
        require(not stat.S_ISLNK(attributes.st_mode)
                and not (getattr(attributes, 'st_file_attributes', 0) & 0x400),
                'local reparse/symlink path refused')
    require(not directory or path.is_dir(), 'local evidence directory missing')
    return path


def open_input(stack, path, cap):
    safe_local(path)
    if os.name == 'nt':
        import ctypes
        import msvcrt
        from ctypes import wintypes
        kernel = ctypes.WinDLL('kernel32', use_last_error=True)
        create = kernel.CreateFileW
        create.argtypes = [wintypes.LPCWSTR, wintypes.DWORD, wintypes.DWORD,
                           wintypes.LPVOID, wintypes.DWORD, wintypes.DWORD, wintypes.HANDLE]
        create.restype = wintypes.HANDLE
        close = kernel.CloseHandle
        close.argtypes = [wintypes.HANDLE]
        close.restype = wintypes.BOOL
        # Read sharing only excludes both writers and deletion until the
        # receipt has been persisted/read back. Transfer HANDLE ownership once.
        handle = create(str(path), 0x80000000, 1, None, 3, 0x00200000, None)
        if handle == ctypes.c_void_p(-1).value:
            raise ctypes.WinError(ctypes.get_last_error())
        try:
            fd = msvcrt.open_osfhandle(handle, os.O_RDONLY | os.O_BINARY | os.O_NOINHERIT)
        except BaseException:
            close(handle)
            raise
        try:
            stream = os.fdopen(fd, 'rb')
        except BaseException:
            os.close(fd)
            raise
        stream = stack.enter_context(stream)
    else:
        stream = stack.enter_context(Path(path).open('rb'))
    info = os.fstat(stream.fileno())
    require(stat.S_ISREG(info.st_mode) and 0 <= info.st_size <= cap,
            'bounded regular evidence file required')
    assert_local_identity(path, stream)
    return stream


def assert_local_identity(path, stream):
    safe_local(path)
    actual = os.stat(path, follow_symlinks=False)
    retained = os.fstat(stream.fileno())
    require((actual.st_dev, actual.st_ino) == (retained.st_dev, retained.st_ino),
            'evidence pathname no longer identifies original open file')


def read_stream(stream, cap, capture=False):
    stream.seek(0)
    count = 0
    sha = hashlib.sha256()
    data = bytearray() if capture else None
    while chunk := stream.read(65536):
        count += len(chunk)
        require(count <= cap, 'file exceeds bounded byte cap')
        sha.update(chunk)
        if capture:
            data.extend(chunk)
    return count, sha.hexdigest().upper(), bytes(data) if capture else None


def content_pin(path, stream, cap):
    length, sha, _ = read_stream(stream, cap)
    assert_local_identity(path, stream)
    return {'Path': str(path), 'Bytes': length, 'Sha256': sha}


def matches_bytes(expected, data):
    require(expected['Bytes'] == len(data) and hash_text(expected['Sha256']) == digest(data),
            'exact pinned byte content differs')


def read_local_ack(stream, run, expected_hash):
    _, sha, raw = read_stream(stream, ACK_CAP, capture=True)
    require(sha == hash_text(expected_hash), 'independently pinned original local acknowledgment differs')
    ack = strict_json(raw)
    schema(ack)
    require(ack.get('Meaning') == 'LOCAL_ARCHIVE_VERIFICATION_ONLY', 'not original local acknowledgment')
    false_flags(ack)
    require(isinstance(ack.get('Nonce'), str) and re.fullmatch(r'[a-fA-F0-9]{32}', ack['Nonce']),
            'original controller nonce required')
    pin(ack.get('ResultPin'), ACK_CAP, run + r'\controller-result.json')
    pin(ack.get('ArchivePin'), ARCHIVE_CAP, run + '.zip')
    pin(ack.get('InventoryPin'), FILE_CAP, run + '.zip.inventory.json')
    return ack, raw


def member(value):
    require(isinstance(value, dict) and set(value) == {'Name', 'Kind', 'Bytes', 'Sha256'},
            'exact inventory member required')
    name = value['Name']
    require(isinstance(name, str) and '\\' not in name, 'canonical ZIP name required')
    directory = value['Kind'] == 'Directory'
    require(value['Kind'] in ('File', 'Directory') and name.endswith('/') == directory,
            'member name/kind differs')
    parts = name.rstrip('/').split('/')
    require(all(part and part not in ('.', '..') and part.rstrip(' .') == part
                and not re.search(r'[<>:"|?*\x00-\x1f]', part) for part in parts),
            'ZIP traversal/invalid member component')
    require((parts[0] == 'run' and len(parts) <= 9)
            or name in ('export/provenance.json', 'export/inventory.json'),
            'foreign ZIP member or run depth exceeded')
    require(type(value['Bytes']) is int and 0 <= value['Bytes'] <= FILE_CAP,
            'member length must be bounded integer')
    if directory:
        require(value['Bytes'] == 0 and value['Sha256'] is None, 'directory has file payload')
    else:
        hash_text(value['Sha256'])
        require(ntpath.splitext(name)[1].lower() in EXTENSIONS
                or name in {'run/managed/' + item for item in STAGED},
                'unknown/executable archive member class')
    return name


def validate_provenance(raw, result_bytes, ack, expected):
    provenance = strict_json(raw)
    schema(provenance)
    false_flags(provenance)
    require(provenance.get('Nonce') == ack['Nonce'], 'provenance nonce differs')
    same_pin(provenance.get('ResultPin'), ack['ResultPin'])
    require(isinstance(provenance.get('ResultJson'), str), 'original result JSON required')
    result_raw = provenance['ResultJson'].encode('utf-8')
    matches_bytes(ack['ResultPin'], result_raw)
    require(result_raw == result_bytes, 'archive result differs from original result JSON')
    result = strict_json(result_raw)
    schema(result)
    require(result.get('ControllerComplete') is True and type(result.get('Safe')) is bool
            and result.get('Nonce') == ack['Nonce'] and result.get('NativePassed') is False
            and result.get('Qualified') is False, 'original result is not terminal unqualified evidence')
    manifest = pin(provenance.get('ManifestPin'), ACK_CAP)
    same_pin(result.get('Manifest'), manifest)
    require(isinstance(provenance.get('InputJson'), str), 'original input snapshot required')
    inputs = strict_json(provenance['InputJson'].encode('utf-8'))
    same_pin(inputs.get('ManifestPin'), manifest)
    for key in ('SourcePins', 'StagedScriptPins', 'TerminalPins'):
        require(isinstance(provenance.get(key), list), 'original provenance pin array required')
        for item in provenance[key]:
            pin(item, FILE_CAP, named=(key == 'SourcePins'))
    sources = inputs.get('Pins', {}).get('Sources', [])
    require(provenance['SourcePins'] == sources, 'original source pins/snapshot differ')
    require(isinstance(provenance.get('CollectorProvenance'), str)
            and provenance['CollectorProvenance'], 'collector provenance required')
    staged = provenance['StagedScriptPins']
    require(len(staged) in (0, 5), 'complete original staged script set required')
    stage_names = set()
    for item in staged:
        name = ntpath.basename(item['Path'])
        require(name in STAGED and name not in stage_names and item['Path'] ==
                ack['ResultPin']['Path'][:-len('controller-result.json')] + 'managed\\' + name,
                'original staged script path differs')
        stage_names.add(name)
    require({name for name in expected if name.endswith('.ps1')} ==
            {'run/managed/' + name for name in stage_names}, 'unpinned staged script membership')
    prefix = ack['ResultPin']['Path'][:-len('controller-result.json')]
    for item in provenance['TerminalPins'] + staged + [manifest, ack['ResultPin']]:
        if item['Path'].casefold().startswith(prefix.casefold()):
            name = 'run/' + item['Path'][len(prefix):].replace('\\', '/')
            row = expected.get(name)
            require(row is not None and row['Kind'] == 'File' and row['Bytes'] == item['Bytes']
                    and hash_text(row['Sha256']) == hash_text(item['Sha256']),
                    'original in-run terminal/staged pin missing or changed')
    return manifest


def verify_delivery(evidence_directory, remote_run, expected_local_ack_sha256):
    run = canonical_run(remote_run)
    expected_hash = hash_text(expected_local_ack_sha256)
    root = safe_local(evidence_directory, directory=True)
    output = root / 'host-delivery.json'
    require(not output.exists(), 'host receipt already exists; never overwrite or retry')
    with ExitStack() as stack:
        ack_file = open_input(stack, root / 'local-ack.json', ACK_CAP)
        ack, ack_raw = read_local_ack(ack_file, run, expected_hash)
        inventory_file = open_input(stack, root / 'inventory.json', FILE_CAP)
        _, _, inventory_raw = read_stream(inventory_file, FILE_CAP, capture=True)
        matches_bytes(ack['InventoryPin'], inventory_raw)
        inventory = strict_json(inventory_raw)
        schema(inventory)
        require(inventory.get('Nonce') == ack['Nonce'], 'inventory nonce differs')
        same_pin(inventory.get('ResultPin'), ack['ResultPin'])
        require(isinstance(inventory.get('Limits'), dict)
                and set(inventory['Limits']) == set(LIMITS)
                and all(type(inventory['Limits'][key]) is int and inventory['Limits'][key] == value
                        for key, value in LIMITS.items()), 'original export caps differ')
        rows = inventory.get('Members')
        require(isinstance(rows, list) and 2 <= len(rows) <= 511, 'bounded original inventory required')
        expected, seen = {}, set()
        for row in rows:
            name = member(row)
            require(name != 'export/inventory.json' and name.casefold() not in seen,
                    'duplicate/case-colliding inventory member or self entry')
            seen.add(name.casefold())
            expected[name] = row
        require({'run/', 'run/controller-result.json', 'export/provenance.json'} <= expected.keys(),
                'required original evidence missing')
        expected['export/inventory.json'] = {'Name': 'export/inventory.json', 'Kind': 'File',
                                            'Bytes': len(inventory_raw), 'Sha256': digest(inventory_raw)}
        require(sum(row['Bytes'] for row in expected.values()) <= TOTAL_CAP, 'total inflated cap exceeded')
        archive_file = open_input(stack, root / 'archive.zip', ARCHIVE_CAP)
        archive_pin = content_pin(root / 'archive.zip', archive_file, ARCHIVE_CAP)
        require(archive_pin['Bytes'] == ack['ArchivePin']['Bytes']
                and archive_pin['Sha256'] == hash_text(ack['ArchivePin']['Sha256']), 'original archive pin differs')
        archive_file.seek(0)
        captures = {}
        try:
            with zipfile.ZipFile(archive_file) as archive:
                infos = archive.infolist()
                require(len(infos) == len(expected) <= 512, 'exact ZIP membership differs')
                seen = set()
                for info in infos:
                    name = info.filename
                    require(info.orig_filename == name and name in expected
                            and name.casefold() not in seen, 'foreign/duplicate/noncanonical ZIP member')
                    seen.add(name.casefold())
                    row = expected[name]
                    mode = stat.S_IFMT(info.external_attr >> 16)
                    require(mode in (0, stat.S_IFREG, stat.S_IFDIR)
                            and not (info.external_attr & 0x400) and not (info.flag_bits & 1)
                            and info.compress_type in (zipfile.ZIP_STORED, zipfile.ZIP_DEFLATED),
                            'link/reparse/encrypted/unsupported ZIP member')
                    require(info.file_size == row['Bytes'], 'ZIP declared length differs')
                    sha, count, header = hashlib.sha256(), 0, b''
                    capture = bytearray() if name in ('export/inventory.json', 'export/provenance.json',
                                                      'run/controller-result.json') else None
                    with archive.open(info) as source:
                        while chunk := source.read(65536):
                            count += len(chunk)
                            require(count <= row['Bytes'], 'ZIP member inflated beyond exact cap')
                            sha.update(chunk)
                            header = (header + chunk)[:4]
                            if capture is not None:
                                capture.extend(chunk)
                    require(count == row['Bytes'] and (row['Kind'] == 'Directory'
                            or sha.hexdigest().upper() == hash_text(row['Sha256'])), 'ZIP member hash/length differs')
                    require(not header.startswith(b'MZ') and header not in MAGIC, 'executable image in evidence refused')
                    if capture is not None:
                        captures[name] = bytes(capture)
        except (zipfile.BadZipFile, RuntimeError, EOFError, NotImplementedError) as error:
            raise ValueError('invalid bounded closed archive') from error
        require(captures['export/inventory.json'] == inventory_raw, 'embedded inventory differs from pinned sidecar')
        manifest = validate_provenance(captures['export/provenance.json'],
                                       captures['run/controller-result.json'], ack, expected)
        # Re-read the original open handles. No external target pin path is opened.
        matches_bytes({'Bytes': len(ack_raw), 'Sha256': expected_hash}, read_stream(ack_file, ACK_CAP, True)[2])
        matches_bytes(ack['InventoryPin'], read_stream(inventory_file, FILE_CAP, True)[2])
        require(content_pin(root / 'archive.zip', archive_file, ARCHIVE_CAP) == archive_pin,
                'archive changed during verification')
        assert_local_identity(root / 'inventory.json', inventory_file)
        assert_local_identity(root / 'local-ack.json', ack_file)
        receipt = {'SchemaVersion': 1, 'Meaning': 'HOST_ARCHIVE_DELIVERY_VERIFIED',
                   'Target': TARGET, 'RunDirectory': run, 'Nonce': ack['Nonce'],
                   'ResultPin': ack['ResultPin'], 'ManifestPin': manifest,
                   'SourceLocalAckPin': {'Path': run + '.archive-local.json', 'Bytes': len(ack_raw),
                                         'Sha256': expected_hash},
                   'ArchivePin': ack['ArchivePin'], 'InventoryPin': ack['InventoryPin'],
                   'HostArchivePin': archive_pin,
                   'HostInventoryPin': content_pin(root / 'inventory.json', inventory_file, FILE_CAP),
                   'HostLocalAckPin': content_pin(root / 'local-ack.json', ack_file, ACK_CAP),
                   'HostEvidenceDirectory': str(root), 'Transferred': True,
                   'NativePassed': False, 'Qualified': False}
        raw = json.dumps(receipt, separators=(',', ':'), allow_nan=False).encode('utf-8')
        safe_local(output)
        with output.open('xb') as saved:
            saved.write(raw)
            saved.flush()
            os.fsync(saved.fileno())
        require(output.read_bytes() == raw, 'durable host receipt readback differs')
        return receipt


def transport_module():
    path = Path(__file__).with_name('vad-delivery-transport.py')
    spec = importlib.util.spec_from_file_location('vad_delivery_transport', path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def retained_call(operation, *args, **kwargs):
    try:
        return operation(*args, **kwargs)
    except BaseException as error:
        owner = getattr(error, 'owner', None)
        # The executable must not discard daemon pumps or a still-live SSH
        # child just because one observation failed. Never invoke work again.
        while owner is not None and not owner.closed:
            try:
                owner.observe()
            except Exception:
                pass
            if not owner.closed:
                time.sleep(0.01)
        raise


def receive_delivery(remote_run, expected_local_ack_sha256, output_directory, *,
                     user_reboot_confirmed, download=None):
    require(user_reboot_confirmed is True, 'user reboot confirmation required; no target access')
    run = canonical_run(remote_run)
    expected_hash = hash_text(expected_local_ack_sha256)
    output = safe_local(output_directory)
    if download is None:
        download = transport_module().download_file
    output.mkdir(exist_ok=False)
    retained_call(download, run + '.archive-local.json', output / 'local-ack.json', user_reboot_confirmed=True,
                  cap=ACK_CAP, log_prefix=output / 'download-local-ack')
    with ExitStack() as stack:
        ack, _ = read_local_ack(open_input(stack, output / 'local-ack.json', ACK_CAP), run, expected_hash)
    retained_call(download, run + '.zip.inventory.json', output / 'inventory.json', user_reboot_confirmed=True,
                  cap=FILE_CAP, log_prefix=output / 'download-inventory')
    with ExitStack() as stack:
        _, _, raw = read_stream(open_input(stack, output / 'inventory.json', FILE_CAP), FILE_CAP, True)
        matches_bytes(ack['InventoryPin'], raw)
    retained_call(download, run + '.zip', output / 'archive.zip', user_reboot_confirmed=True,
                  cap=ARCHIVE_CAP, log_prefix=output / 'download-archive')
    return verify_delivery(output, run, expected_hash)


def validate_finalizer(finalizer_path, finalizer_sha256, dependency_pins):
    canonical_run(finalizer_path)
    require(ntpath.basename(finalizer_path) == 'delivery-finalize.ps1', 'original target finalizer name required')
    finalizer_hash = hash_text(finalizer_sha256)
    require(isinstance(dependency_pins, list) and len(dependency_pins) == 2,
            'both independently pinned finalizer dependencies required')
    expected = {ntpath.join(ntpath.dirname(finalizer_path), name)
                for name in ('closed-export.ps1', 'finite-tool.ps1')}
    actual = set()
    for item in dependency_pins:
        pin(item, ACK_CAP)
        require(item['Path'] in expected and item['Path'] not in actual, 'finalizer dependency path differs')
        actual.add(item['Path'])
    require(actual == expected, 'finalizer dependency membership differs')
    return finalizer_hash


def deliver_run(remote_run, expected_local_ack_sha256, output_directory,
                finalizer_path, finalizer_sha256, *, dependency_pins,
                user_reboot_confirmed, download=None, finalize=None):
    require(user_reboot_confirmed is True, 'user reboot confirmation required; no target access')
    run = canonical_run(remote_run)
    expected_hash = hash_text(expected_local_ack_sha256)
    finalizer_hash = validate_finalizer(finalizer_path, finalizer_sha256, dependency_pins)
    if download is None or finalize is None:
        module = transport_module()
        download = module.download_file if download is None else download
        finalize = module.finalize_delivery if finalize is None else finalize
    host = receive_delivery(run, expected_hash, output_directory, user_reboot_confirmed=True,
                            download=download)
    root = Path(output_directory)
    host_raw = (root / 'host-delivery.json').read_bytes()
    require(strict_json(host_raw) == host, 'persisted host receipt changed before delivery')
    delivered_pin = retained_call(finalize, run, host_raw, expected_hash, finalizer_path,
                                  finalizer_hash, dependency_pins=dependency_pins,
                                  user_reboot_confirmed=True, log_prefix=root / 'finalize-delivery')
    pin(delivered_pin, 2 * ACK_CAP, run + '.archive-delivered.json')
    target_path = root / 'target-delivered.json'
    retained_call(download, delivered_pin['Path'], target_path, cap=2 * ACK_CAP,
                  user_reboot_confirmed=True, log_prefix=root / 'download-delivered')
    with ExitStack() as stack:
        _, _, target_raw = read_stream(open_input(stack, target_path, 2 * ACK_CAP), 2 * ACK_CAP, True)
        matches_bytes(delivered_pin, target_raw)
    target = strict_json(target_raw)
    schema(target)
    false_flags(target, transferred=True)
    require(target.get('Meaning') == 'TARGET_ARCHIVE_DELIVERY_ACKNOWLEDGED', 'target delivery acknowledgment required')
    for key in ('Target', 'RunDirectory', 'Nonce', 'ResultPin', 'ManifestPin', 'SourceLocalAckPin',
                'ArchivePin', 'InventoryPin', 'HostEvidenceDirectory', 'HostArchivePin',
                'HostInventoryPin', 'HostLocalAckPin'):
        require(target.get(key) == host[key], 'target acknowledgment changed original ' + key)
    receipt_pin = pin(target.get('DeliveryReceiptPin'), ACK_CAP, run + '.delivery-host.json')
    matches_bytes(receipt_pin, host_raw)
    require(target.get('DeliveryReceiptJson') == host_raw.decode('utf-8'), 'target did not acknowledge exact host receipt')
    complete = {'SchemaVersion': 1, 'Meaning': 'CLOSED_RUN_DELIVERY_COMPLETE', 'Target': TARGET,
                'RunDirectory': run, 'Nonce': host['Nonce'], 'ResultPin': host['ResultPin'],
                'SourceLocalAckPin': host['SourceLocalAckPin'], 'TargetAcknowledgmentPin': delivered_pin,
                'HostReceiptPin': {'Path': str(root / 'host-delivery.json'), 'Bytes': len(host_raw),
                                   'Sha256': digest(host_raw)},
                'HostTargetAcknowledgmentPin': {'Path': str(target_path), 'Bytes': len(target_raw),
                                               'Sha256': digest(target_raw)},
                'Transferred': True, 'NativePassed': False, 'Qualified': False,
                'CleanupAuthorized': False}
    with ExitStack() as stack:
        frozen = [(root / 'archive.zip', host['HostArchivePin'], ARCHIVE_CAP),
                  (root / 'inventory.json', host['HostInventoryPin'], FILE_CAP),
                  (root / 'local-ack.json', host['HostLocalAckPin'], ACK_CAP),
                  (root / 'host-delivery.json', complete['HostReceiptPin'], ACK_CAP),
                  (target_path, complete['HostTargetAcknowledgmentPin'], 2 * ACK_CAP)]
        for path, expected, cap in frozen:
            source = open_input(stack, path, cap)
            require(content_pin(path, source, cap) == expected,
                    'frozen host delivery evidence changed before completion')
        raw = json.dumps(complete, separators=(',', ':'), allow_nan=False).encode('utf-8')
        output = safe_local(root / 'delivery-complete.json')
        with output.open('xb') as stream:
            stream.write(raw)
            stream.flush()
            os.fsync(stream.fileno())
        require(output.read_bytes() == raw, 'delivery completion readback differs')
    return complete


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-directory', required=True)
    parser.add_argument('--expected-local-ack-sha256', required=True)
    parser.add_argument('--output-directory', type=Path, required=True)
    parser.add_argument('--user-reboot-confirmed', action='store_true')
    parser.add_argument('--finalizer-path', required=True)
    parser.add_argument('--expected-finalizer-sha256', required=True)
    args = parser.parse_args(argv)
    require(args.user_reboot_confirmed is True, 'user reboot confirmation required; no target access')
    source = Path(__file__).with_name('vad-removal')
    finalizer_hash = hash_text(args.expected_finalizer_sha256)
    with ExitStack() as stack:
        original = content_pin(source / 'delivery-finalize.ps1',
                               open_input(stack, source / 'delivery-finalize.ps1', ACK_CAP), ACK_CAP)
        require(original['Sha256'] == finalizer_hash, 'independently pinned finalizer differs from reviewed local source')
        dependencies = []
        for name in ('closed-export.ps1', 'finite-tool.ps1'):
            item = content_pin(source / name, open_input(stack, source / name, ACK_CAP), ACK_CAP)
            item['Path'] = ntpath.join(ntpath.dirname(args.finalizer_path), name)
            dependencies.append(item)
        receipt = deliver_run(args.run_directory, args.expected_local_ack_sha256,
                              args.output_directory, args.finalizer_path, finalizer_hash,
                              dependency_pins=dependencies, user_reboot_confirmed=True)
    print(json.dumps(receipt, separators=(',', ':'), allow_nan=False))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
