# SPDX-License-Identifier: MIT
"""Associate bounded decoded WPP stages with one retained QUERY snapshot.

No native operations. Decoder statistics do not attest raw ETW completeness,
and Cleanup/Destroy callback entry never substitutes for a fresh Removed QUERY.
"""
import argparse
import json
from pathlib import Path
import re
import sys

STAGES = ('remove_enter', 'remove_return', 'release_enter', 'release_return',
          'cleanup_entry', 'destroy_entry')
KINDS = ('first', 'last', 'first_failure')
U64 = (1 << 64) - 1
U32 = (1 << 32) - 1
MAX_BYTES = 16 * 1024 * 1024
MAX_OUTPUT = 1024 * 1024
FIELDS = {
    'stage': ('endpoint session stage count sequence frequency saturated').split(),
    'observation': ('endpoint session stage kind sequence qpc has_status status native_call').split(),
    'overwritten': ('endpoint session count old_endpoint old_session old_sequence old_pending saturated').split(),
}


def uint(value, maximum=U64, minimum=0):
    if isinstance(value, str) and re.fullmatch(r'[0-9]+', value):
        value = int(value)
    if type(value) is not int or not minimum <= value <= maximum:
        raise ValueError('invalid unsigned scalar')
    return value


def unique_object(pairs):
    value = {}
    for key, item in pairs:
        if key in value:
            raise ValueError('report contains a duplicate JSON key')
        value[key] = item
    return value


def reject_constant(_value):
    raise ValueError('report contains a nonfinite JSON constant')


def analyze(text, summary, report):
    result = dict(schema_version=1, qualified=False, trace_association_available=False,
                  direct_removed_observed=False, source_stop_join_observed=False,
                  engine_closed_observed=False, native_error=None, runtime=None, session=None,
                  observed_stages=[], unobserved_stages=list(STAGES), observations=[],
                  stage_headers=[], overwrites=[], foreign_fragments=0, reasons=[],
                  decoder_statistics={}, raw_trace_loss='unknown',
                  limitations=[
                      'Decoded association is not raw ETW completeness or native qualification.',
                      'Cleanup/Destroy mean callback entry, not completion.',
                      'Release entry without return includes pre-ACX quiescence; no stall location inferred.',
                      'First/last/count cannot pair every removal attempt; unobserved stages remain unknown.',
                      'Only the retained fresh QUERY may establish direct Removed.'])

    def refuse(reason):
        if reason not in result['reasons']:
            result['reasons'].append(reason)

    if max(len(text), len(summary)) > MAX_BYTES:
        refuse('input text exceeds hard cap')
        return result
    for label in ('Events Processed', 'Events Lost', 'Format Errors', 'Formats Unknown'):
        lines = [line for line in summary.splitlines() if re.match(
            r'^Total\s+' + label.replace(' ', r'\s+') + r'\b', line)]
        match = re.fullmatch(r'Total\s+' + label.replace(' ', r'\s+') + r'\s+([0-9]+)\s*',
                             lines[0]) if len(lines) == 1 else None
        if match is None:
            refuse('missing, duplicate or malformed decoder statistic: ' + label)
            continue
        try:
            result['decoder_statistics'][label] = uint(match.group(1))
        except ValueError:
            refuse('decoder statistic out of range: ' + label)
    for label, value in result['decoder_statistics'].items():
        if label != 'Events Processed' and value:
            refuse('decoder reports nonzero ' + label)
    if result['decoder_statistics'].get('Events Processed') == 0:
        refuse('decoder reports no processed events')

    try:
        native = report['native']
        result['native_error'] = native.get('error')
        cleanup = native['vad_cleanup']
        snapshot = cleanup['snapshot']
        stamp = snapshot['stamp']
        if not isinstance(stamp, list) or len(stamp) != 5 or any(type(v) is not int for v in stamp):
            raise ValueError('invalid stamp')
        for v in stamp:
            uint(v)
        runtime, session = stamp[:2]
        if runtime == 0 or session == 0 or stamp[2] != runtime:
            raise ValueError('invalid runtime/session')
        result['runtime'], result['session'] = runtime, session
        fresh = (cleanup.get('diagnostic_provenance') == 'Fresh' and
                 'error' in cleanup and cleanup['error'] is None)
        if not fresh:
            refuse('retained QUERY is cached, failed or missing provenance')
        result['direct_removed_observed'] = fresh and snapshot.get('state') == 'Removed'
        playback = native.get('vad_playback') or {}
        result['source_stop_join_observed'] = (playback.get('worker_joined') is True and
                                              playback.get('native_stop_succeeded') is True and
                                              'failure' in playback and playback['failure'] is None)
        engine = native.get('engine_cleanup') or {}
        result['engine_closed_observed'] = (engine.get('host') == 'Closed' and
                                           engine.get('fault') == 'None' and
                                           engine.get('cleanup_error') == 'None')
    except (KeyError, TypeError, ValueError, AttributeError):
        refuse('missing or malformed exact runtime/session QUERY stamp')
        return result

    headers = {}
    observations = {}
    frequencies = set()
    fragments = 0
    for line in text.splitlines():
        if 'Removal ' not in line:
            continue
        fragments += 1
        if fragments > 4096 or len(line) > 4096:
            refuse('decoded fragment count or line cap exceeded')
            break
        try:
            payload = line[line.index('Removal '):]
            match = re.fullmatch(r'Removal (stage|observation|overwritten) (.+)', payload)
            if not match:
                raise ValueError('unknown removal fragment')
            kind, fields = match.groups()
            pairs = [item.split('=', 1) for item in fields.split()]
            if any(len(pair) != 2 for pair in pairs) or [pair[0] for pair in pairs] != FIELDS[kind]:
                raise ValueError('malformed fragment fields')
            row = dict(pairs)
            for key in row:
                if key == 'status':
                    if not re.fullmatch(r'STATUS_[A-Z0-9_]+|0[xX][0-9A-Fa-f]{1,8}', row[key]):
                        raise ValueError('malformed NTSTATUS text')
                else:
                    row[key] = uint(row[key])
            target_overwritten = (kind == 'overwritten' and row['old_endpoint'] == runtime and
                                  row['old_session'] == session)
            if target_overwritten:
                result['overwrites'].append(row)
                refuse('pending target lifetime evidence was overwritten by another runtime')
            if row['endpoint'] != runtime:
                result['foreign_fragments'] += 1
                continue
            if row['session'] != session:
                refuse('same runtime has another session in decoded input')
                continue
            if kind in ('stage', 'overwritten'):
                uint(row['saturated'], 1)
                if row['saturated']:
                    refuse('diagnostic counter saturation reported')
            if kind == 'overwritten':
                uint(row['count'], U64, 1)
                uint(row['old_pending'], 63, 1)
                result['overwrites'].append(row)
                refuse('pending lifetime evidence was overwritten')
                continue
            stage = uint(row['stage'], len(STAGES) - 1)
            uint(row['sequence'], U64, 1)
            if kind == 'stage':
                uint(row['count'], U32, 1)
                uint(row['frequency'], U64, 1)
                if row['count'] > row['sequence']:
                    raise ValueError('stage count exceeds scalar sequence watermark')
                previous = headers.get(stage)
                if previous is not None and (row['count'] < previous['count'] or
                                             row['sequence'] < previous['sequence']):
                    raise ValueError('stage counter or export watermark regressed')
                if previous is not None and (row['count'] - previous['count'] >
                                             row['sequence'] - previous['sequence']):
                    raise ValueError('stage count grew without enough scalar sequence increments')
                frequencies.add(row['frequency'])
                headers[stage] = row
                result['stage_headers'].append(row)
                continue
            uint(row['kind'], len(KINDS) - 1)
            uint(row['qpc'], U64, 1)
            uint(row['has_status'], 1)
            uint(row['native_call'], 1)
            if row['kind'] == 2:
                status = row['status']
                # Record() retains FirstFailure only with HasStatus and bit 31.
                # Preserve unknown symbolic names without inventing their bits.
                if (not row['has_status'] or status == 'STATUS_SUCCESS' or
                        (status.lower().startswith('0x') and not int(status, 16) & (1 << 31))):
                    raise ValueError('first failure contradicts status flags or numeric status')
            header = headers.get(stage)
            if header is None or row['sequence'] > header['sequence']:
                raise ValueError('observation lacks matching exported stage watermark')
            # First/last/failure may describe the same scalar event. Retain kinds,
            # not duplicate attempts or invented missing retry observations.
            event_kind = KINDS[row.pop('kind')]
            sequence = row['sequence']
            row['stage'] = STAGES[stage]
            row['has_status'] = bool(row['has_status'])
            row['native_call'] = bool(row['native_call'])
            if sequence in observations:
                previous = {k: v for k, v in observations[sequence].items() if k != 'kinds'}
                if previous != row:
                    raise ValueError('same sequence carries conflicting observations')
                if event_kind not in observations[sequence]['kinds']:
                    observations[sequence]['kinds'].append(event_kind)
            else:
                observations[sequence] = dict(row, kinds=[event_kind])
        except (TypeError, ValueError):
            refuse('malformed or conflicting decoded removal fragment')
    if len(frequencies) > 1:
        refuse('QPC frequency changed within exact lifetime')
    result['observations'] = [observations[key] for key in sorted(observations)]
    if any(a['qpc'] > b['qpc'] for a, b in zip(result['observations'], result['observations'][1:])):
        refuse('QPC regressed in scalar sequence order')
    observed = {row['stage'] for row in result['observations']}
    result['observed_stages'] = [name for name in STAGES if name in observed]
    result['unobserved_stages'] = [name for name in STAGES if name not in observed]
    if not observations:
        refuse('no decoded scalar observations for exact runtime/session')
    if any(STAGES[stage] not in observed for stage in headers):
        refuse('exported stage has no decoded scalar observation')
    result['trace_association_available'] = not result['reasons']
    return result


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for field in ('trace', 'summary', 'report', 'output'):
        parser.add_argument('--' + field, type=Path, required=True)
    parser.add_argument('--max-input-bytes', type=int, required=True)
    args = parser.parse_args(argv)
    try:
        if not 1 <= args.max_input_bytes <= MAX_BYTES:
            raise ValueError('input cap must be 1..16777216 bytes per file')
        if args.output.exists() or args.output.is_symlink():
            raise ValueError('output already exists')
        content = []
        for path in (args.trace, args.summary, args.report):
            with path.open('rb') as stream:
                raw = stream.read(args.max_input_bytes + 1)
            if len(raw) > args.max_input_bytes:
                raise ValueError('input exceeds explicit byte cap')
            content.append(raw.decode('utf-8-sig'))
        report = json.loads(content[2], object_pairs_hook=unique_object,
                            parse_constant=reject_constant)
        value = analyze(content[0], content[1], report)
        encoded = (json.dumps(value, indent=2, allow_nan=False) + '\n').encode('ascii')
        if len(encoded) > MAX_OUTPUT:
            raise ValueError('output exceeds independent 1 MiB cap')
        with args.output.open('xb') as stream:
            stream.write(encoded)
        return 0 if value['trace_association_available'] else 1
    except (OSError, ValueError, TypeError, RecursionError) as error:
        print(str(error), file=sys.stderr)
        return 2


if __name__ == '__main__':
    raise SystemExit(main())
