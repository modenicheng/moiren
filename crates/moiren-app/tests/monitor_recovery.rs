use moiren_app::monitor::{MonitorFaults, MonitorRecovery, MonitorRecoveryPolicy};
use std::time::Duration;

fn policy() -> MonitorRecoveryPolicy {
    MonitorRecoveryPolicy {
        startup_grace: Duration::from_millis(100),
        window: Duration::from_millis(100),
        max_observation_gap: Duration::from_millis(200),
        consecutive_windows: 3,
        fade_frames: 240,
        fade_timeout: Duration::from_secs(1),
    }
}

fn observation(output_frames: u64, underrun_frames: u64) -> MonitorFaults {
    MonitorFaults {
        output_frames,
        underrun_frames,
        ..MonitorFaults::default()
    }
}

#[test]
fn startup_and_old_fault_totals_do_not_trigger_recovery() {
    let mut monitor = MonitorRecovery::new(policy()).unwrap();
    for tick in 0..=10 {
        assert_eq!(
            monitor.observe(
                Duration::from_millis(tick * 100),
                observation(tick * 4800, 999)
            ),
            None
        );
    }
}

#[test]
fn consecutive_new_faults_trigger_once_with_all_causes_preserved() {
    let mut monitor = MonitorRecovery::new(policy()).unwrap();
    let mut faults = observation(4800, 99);
    assert_eq!(monitor.observe(Duration::from_millis(100), faults), None);
    for tick in 2..=4 {
        faults.output_frames += 4800;
        faults.underrun_frames += 1;
        faults.dropped_frames += 2;
        faults.resets += 1;
        faults.backlog_recoveries += 1;
        faults.age_recoveries += 1;
        faults.discontinuities += 1;
        let result = monitor.observe(Duration::from_millis(tick * 100), faults);
        if tick < 4 {
            assert_eq!(result, None);
        } else {
            let reason = result.unwrap();
            assert_eq!(reason.consecutive_windows, 3);
            assert_eq!(reason.faults.underrun_frames, 3);
            assert_eq!(reason.faults.dropped_frames, 6);
            assert_eq!(reason.faults.resets, 3);
            assert_eq!(reason.faults.backlog_recoveries, 3);
            assert_eq!(reason.faults.age_recoveries, 3);
            assert_eq!(reason.faults.discontinuities, 3);
        }
    }
    faults.underrun_frames += 1;
    assert_eq!(monitor.observe(Duration::from_millis(500), faults), None);
}

#[test]
fn healthy_windows_and_counter_regressions_break_the_streak() {
    let mut monitor = MonitorRecovery::new(policy()).unwrap();
    for (tick, total) in [
        (1, 0),
        (2, 1),
        (3, 2),
        (4, 2),
        (5, 3),
        (6, 1),
        (7, 2),
        (8, 3),
    ] {
        assert_eq!(
            monitor.observe(
                Duration::from_millis(tick * 100),
                observation(tick * 4800, total)
            ),
            None
        );
    }
    assert!(
        monitor
            .observe(Duration::from_millis(900), observation(43_200, 4))
            .is_some()
    );
}

#[test]
fn long_poll_gaps_and_regressed_time_cannot_invent_consecutive_windows() {
    let mut monitor = MonitorRecovery::new(policy()).unwrap();
    for (time, frames, faults) in [
        (100, 4800, 0),
        (200, 9600, 1),
        (300, 14_400, 2),
        (900, 43_200, 9),
        (1000, 48_000, 10),
        (950, 48_000, 10),
        (1100, 52_800, 11),
    ] {
        assert_eq!(
            monitor.observe(Duration::from_millis(time), observation(frames, faults)),
            None
        );
    }
}

#[test]
fn stopped_output_and_duplicate_polls_do_not_count_fault_windows() {
    let mut monitor = MonitorRecovery::new(policy()).unwrap();
    for tick in 1..=20 {
        let faults = observation(4800, tick);
        let now = Duration::from_millis(tick * 100);
        assert_eq!(monitor.observe(now, faults), None);
        assert_eq!(monitor.observe(now, faults), None);
    }
}

#[test]
fn policy_rejects_unbounded_or_invalid_recovery_settings() {
    for invalid in [
        MonitorRecoveryPolicy {
            window: Duration::ZERO,
            ..policy()
        },
        MonitorRecoveryPolicy {
            consecutive_windows: 0,
            ..policy()
        },
        MonitorRecoveryPolicy {
            consecutive_windows: u32::MAX,
            ..policy()
        },
        MonitorRecoveryPolicy {
            max_observation_gap: Duration::from_millis(99),
            ..policy()
        },
        MonitorRecoveryPolicy {
            fade_frames: 0,
            ..policy()
        },
        MonitorRecoveryPolicy {
            fade_frames: 48_001,
            ..policy()
        },
        MonitorRecoveryPolicy {
            fade_timeout: Duration::ZERO,
            ..policy()
        },
    ] {
        assert!(MonitorRecovery::new(invalid).is_err());
    }
}

#[test]
fn fault_windows_begin_after_the_entire_startup_grace() {
    let mut monitor = MonitorRecovery::new(MonitorRecoveryPolicy {
        startup_grace: Duration::from_millis(300),
        ..policy()
    })
    .unwrap();
    for tick in 0..=4 {
        assert_eq!(
            monitor.observe(
                Duration::from_millis(tick * 100),
                observation(tick * 4800, tick)
            ),
            None
        );
    }
    assert_eq!(
        monitor.observe(Duration::from_millis(500), observation(24_000, 5)),
        None
    );
    assert!(
        monitor
            .observe(Duration::from_millis(600), observation(28_800, 6))
            .is_some()
    );
}

#[test]
fn every_supported_fault_can_trigger_without_an_underrun() {
    for cause in 0..6 {
        let mut monitor = MonitorRecovery::new(policy()).unwrap();
        for tick in 1..=4 {
            let mut faults = observation(tick * 4800, 0);
            match cause {
                0 => faults.dropped_frames = tick,
                1 => faults.resets = tick,
                2 => faults.backlog_recoveries = tick,
                3 => faults.age_recoveries = tick,
                4 => faults.discontinuities = tick,
                _ => faults.priming_frames = tick,
            }
            let result = monitor.observe(Duration::from_millis(tick * 100), faults);
            assert_eq!(result.is_some(), tick == 4);
        }
    }
}
