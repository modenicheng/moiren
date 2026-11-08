use super::*;
use std::cell::{Cell, RefCell};

fn arguments(directory: &Path) -> Vec<String> {
    [
        "managed-removal",
        "--run-dir",
        directory.to_str().unwrap(),
        "--nonce",
        "0123456789abcdef0123456789abcdef",
        "--controller-pid",
        "11",
        "--controller-creation-ticks",
        "639000000000000001",
        "--worker-pid",
        "12",
        "--worker-creation-ticks",
        "639000000000000002",
        "--admission-timeout-ms",
        "30",
        "--",
        "live-vad",
        "--seconds",
        "8",
    ]
    .map(str::to_owned)
    .to_vec()
}

fn receipt(config: &Config) -> Receipt {
    Receipt {
        schema_version: 1,
        nonce: config.nonce.clone(),
        probe_pid: 13,
        probe_creation_utc_ticks: 639_000_000_000_000_003,
        controller_pid: config.controller_pid,
        controller_creation_utc_ticks: config.controller_ticks,
        worker_pid: config.worker_pid,
        worker_creation_utc_ticks: config.worker_ticks,
        session_id: 1,
    }
}

type PollAction = RefCell<Box<dyn FnMut(&Fake)>>;

struct Fake {
    receipt: Receipt,
    elapsed: Cell<Duration>,
    live: Cell<bool>,
    on_poll: PollAction,
}

impl Fake {
    fn new(config: &Config) -> Self {
        Self {
            receipt: receipt(config),
            elapsed: Cell::new(Duration::ZERO),
            live: Cell::new(true),
            on_poll: RefCell::new(Box::new(|_| {})),
        }
    }
}

impl Boundary for Fake {
    fn receipt(&self, _: &Config) -> Receipt {
        self.receipt.clone()
    }
    fn owners_alive(&self) -> Result<()> {
        ensure!(self.live.get(), "fake owner exited");
        Ok(())
    }
    fn elapsed(&self) -> Duration {
        self.elapsed.get()
    }
    fn poll(&mut self) {
        self.elapsed
            .set(self.elapsed.get() + Duration::from_millis(10));
        (self.on_poll.borrow_mut())(self);
    }
}

fn never_enter(_: Vec<String>) -> Result<i32> {
    panic!("unadmitted native CLI called")
}

#[test]
fn malformed_managed_prefix_cannot_enter_cli() {
    let mut entered = false;
    let result = run(vec!["managed-removal".into()], |_| {
        entered = true;
        Ok(0)
    });
    assert!(result.is_err(), "unadmitted managed command was accepted");
    assert!(!entered, "managed command entered CLI before admission");
}

#[test]
fn ordinary_commands_keep_their_arguments_and_exit_code() {
    let arguments = vec!["--help".into()];
    assert_eq!(
        run(arguments.clone(), |actual| {
            assert_eq!(actual, arguments);
            Ok(7)
        })
        .unwrap(),
        7
    );
}

#[test]
fn config_accepts_exact_protocol_without_float_ticks() {
    let directory = crate::tests::Fixture::new();
    let config = Config::parse(&arguments(&directory.path("."))).unwrap();
    assert_eq!(config.controller_ticks, 639_000_000_000_000_001);
    assert_eq!(config.timeout, Duration::from_millis(30));
    assert_eq!(config.arguments, ["live-vad", "--seconds", "8"]);
}

#[test]
fn config_refuses_duplicate_unknown_invalid_identity_and_non_vad() {
    let directory = crate::tests::Fixture::new();
    let original = arguments(&directory.path("."));
    for (index, value) in [
        (1, "--worker-pid"),
        (1, "--unexpected"),
        (4, "BAD"),
        (6, "0"),
        (6, "12"),
        (8, "6.39e17"),
        (8, "3155378976000000000"),
        (14, "15001"),
        (14, "0"),
        (16, "live-process-loopback"),
    ] {
        let mut bad = original.clone();
        bad[index] = value.into();
        assert!(Config::parse(&bad).is_err(), "accepted {index}={value}");
    }
}

#[test]
fn missing_ack_times_out_with_ready_retained_without_cli() {
    let directory = crate::tests::Fixture::new();
    let config = Config::parse(&arguments(&directory.path("."))).unwrap();
    let mut boundary = Fake::new(&config);
    assert!(
        admitted_run(config, &mut boundary, never_enter)
            .unwrap_err()
            .to_string()
            .contains("timed out")
    );
    let ready: Receipt =
        serde_json::from_slice(&fs::read(directory.path("probe-ready.json")).unwrap()).unwrap();
    assert_eq!(ready, boundary.receipt);
}

#[test]
fn matching_ack_enters_once_with_unchanged_args_and_propagates_exit() {
    let directory = crate::tests::Fixture::new();
    let config = Config::parse(&arguments(&directory.path("."))).unwrap();
    let expected_arguments = config.arguments.clone();
    let mut boundary = Fake::new(&config);
    let path = directory.path("probe-admit.json");
    boundary.on_poll = RefCell::new(Box::new(move |fake| {
        fs::write(&path, serde_json::to_vec(&fake.receipt).unwrap()).unwrap();
    }));
    let mut calls = 0;
    assert_eq!(
        admitted_run(config, &mut boundary, |args| {
            calls += 1;
            assert_eq!(args, expected_arguments);
            Ok(1)
        })
        .unwrap(),
        1
    );
    assert_eq!(calls, 1);
}

#[test]
fn dead_owner_even_with_matching_ack_cannot_enter() {
    let directory = crate::tests::Fixture::new();
    let config = Config::parse(&arguments(&directory.path("."))).unwrap();
    let mut boundary = Fake::new(&config);
    let path = directory.path("probe-admit.json");
    boundary.on_poll = RefCell::new(Box::new(move |fake| {
        fs::write(&path, serde_json::to_vec(&fake.receipt).unwrap()).unwrap();
        fake.live.set(false);
    }));
    assert!(admitted_run(config, &mut boundary, never_enter).is_err());
}

#[test]
fn ack_at_deadline_cannot_enter() {
    let directory = crate::tests::Fixture::new();
    let config = Config::parse(&arguments(&directory.path("."))).unwrap();
    let mut boundary = Fake::new(&config);
    let path = directory.path("probe-admit.json");
    boundary.on_poll = RefCell::new(Box::new(move |fake| {
        fs::write(&path, serde_json::to_vec(&fake.receipt).unwrap()).unwrap();
        fake.elapsed.set(Duration::from_millis(30));
    }));
    assert!(admitted_run(config, &mut boundary, never_enter).is_err());
}

#[test]
fn stale_receipts_cannot_be_reused_or_overwritten() {
    for name in [
        "probe-ready.json",
        "probe-ready.json.partial",
        "probe-admit.json",
        "probe-admit.json.partial",
    ] {
        let directory = crate::tests::Fixture::new();
        fs::write(directory.path(name), b"retained").unwrap();
        let config = Config::parse(&arguments(&directory.path("."))).unwrap();
        let mut boundary = Fake::new(&config);
        assert!(admitted_run(config, &mut boundary, never_enter).is_err());
        assert_eq!(fs::read(directory.path(name)).unwrap(), b"retained");
    }
}

#[test]
fn ack_must_match_every_field_and_be_strict_bounded_json() {
    for field in [
        "schema_version",
        "nonce",
        "probe_pid",
        "probe_creation_utc_ticks",
        "controller_pid",
        "controller_creation_utc_ticks",
        "worker_pid",
        "worker_creation_utc_ticks",
        "session_id",
        "unknown",
        "duplicate",
        "float",
        "trailing",
        "oversized",
    ] {
        let directory = crate::tests::Fixture::new();
        let config = Config::parse(&arguments(&directory.path("."))).unwrap();
        let mut boundary = Fake::new(&config);
        let path = directory.path("probe-admit.json");
        boundary.on_poll = RefCell::new(Box::new(move |fake| {
            let mut value = serde_json::to_value(&fake.receipt).unwrap();
            let bytes = match field {
                "duplicate" => serde_json::to_vec(&value)
                    .unwrap()
                    .into_iter()
                    .chain(b" ".iter().copied())
                    .collect::<Vec<_>>(),
                "float" => {
                    value["probe_creation_utc_ticks"] = serde_json::json!(639000000000000003.0);
                    serde_json::to_vec(&value).unwrap()
                }
                "trailing" => {
                    let mut b = serde_json::to_vec(&value).unwrap();
                    b.extend(b"null");
                    b
                }
                "oversized" => vec![b' '; CAP as usize + 1],
                "nonce" => {
                    value[field] = serde_json::json!("ffffffffffffffffffffffffffffffff");
                    serde_json::to_vec(&value).unwrap()
                }
                "unknown" => {
                    value[field] = serde_json::json!(1);
                    serde_json::to_vec(&value).unwrap()
                }
                _ => {
                    value[field] = serde_json::json!(value[field].as_u64().unwrap() + 1);
                    serde_json::to_vec(&value).unwrap()
                }
            };
            let bytes = if field == "duplicate" {
                let text = String::from_utf8(bytes).unwrap();
                text.replacen('{', "{\"probe_pid\":13,", 1).into_bytes()
            } else {
                bytes
            };
            fs::write(&path, bytes).unwrap();
        }));
        assert!(
            admitted_run(config, &mut boundary, never_enter).is_err(),
            "accepted {field}"
        );
    }
}
