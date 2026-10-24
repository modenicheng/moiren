#!/bin/bash
# E3: procloop capture of tone.exe; toggle its SESSION mute mid-run.
set -u
cd "$(dirname "$0")/lab"
LAB=./target/release/audio-hack-lab.exe

echo "== restore-safety: unmute endpoint for the test =="
$LAB epmute --off

$LAB tone --dur 16 --freq 997 > ../tone_e3.log 2>&1 &
sleep 2
TPID=$(grep -oP 'PID=\K[0-9]+' ../tone_e3.log | head -1)
echo "tone pid=$TPID"

$LAB procloop --pid "$TPID" --dur 12 > ../procloop_e3.log 2>&1 &
sleep 3
echo "--- before mute:"; $LAB sessions --pid "$TPID"
$LAB mute --pid "$TPID" --on
sleep 3
echo "--- muted:"; $LAB sessions --pid "$TPID"
$LAB mute --pid "$TPID" --off
sleep 3
echo "--- unmuted:"; $LAB sessions --pid "$TPID"
wait
echo "== restoring endpoint mute =="
$LAB epmute --on
echo "== tone log =="; cat ../tone_e3.log
echo "== procloop log =="; cat ../procloop_e3.log
