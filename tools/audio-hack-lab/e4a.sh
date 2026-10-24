#!/bin/bash
# E4a: procloop capture of tone.exe; toggle ENDPOINT master mute mid-run.
set -u
cd "$(dirname "$0")/lab"
LAB=./target/release/audio-hack-lab.exe

$LAB epmute --off
./$LAB tone --dur 16 --freq 997 > ../tone_e4a.log 2>&1 &
sleep 2
TPID=$(grep -oP 'PID=\K[0-9]+' ../tone_e4a.log | head -1)
echo "tone pid=$TPID"
./$LAB procloop --pid "$TPID" --dur 12 > ../procloop_e4a.log 2>&1 &
sleep 3.5
$LAB epmute --on
sleep 3.5
$LAB epmute --off
wait
$LAB epmute --on
echo "== procloop (endpoint mute toggled at ~3.5s on, ~7s off) =="
cat ../procloop_e4a.log
