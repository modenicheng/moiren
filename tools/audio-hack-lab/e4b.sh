#!/bin/bash
# E4b (control): classic loopback capture; toggle ENDPOINT master mute mid-run.
set -u
cd "$(dirname "$0")/lab"
LAB=./target/release/audio-hack-lab.exe

$LAB epmute --off
./$LAB tone --dur 16 --freq 997 > ../tone_e4b.log 2>&1 &
sleep 2
./$LAB loopback --dur 12 > ../loopback_e4b.log 2>&1 &
sleep 3.5
$LAB epmute --on
sleep 3.5
$LAB epmute --off
wait
$LAB epmute --on
echo "== classic loopback (endpoint mute toggled at ~3.5s on, ~7s off) =="
cat ../loopback_e4b.log
