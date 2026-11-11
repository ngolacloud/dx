//! Subprocess lifecycle: wall-clock deadline (E-1) + whole-group reaping (E-2).
//!
//! `RLIMIT_CPU` bounds CPU, not I/O, so a `build.rs` that blocks on I/O would hang
//! forever. The sandbox now runs each child as the leader of its own process group
//! with a wall-clock deadline; exceeding it kills AND reaps the whole group — the
//! child and its descendants — not just the direct child.

#![cfg(unix)]

use std::time::{Duration, Instant};

use dx_sandbox::{run, Capabilities, ResourceLimits, SandboxSpec};

/// A minimal spec for these tests: no network isolation (so no `unshare`), env not
/// scrubbed (so `sh`/`sleep` resolve through PATH), no resource caps (so a shell
/// that forks a grandchild is not stopped by `RLIMIT_NPROC` — these tests exercise
/// the LIFECYCLE, not the rlimits), and the given wall-clock deadline.
fn spec_with_deadline(secs: u64) -> SandboxSpec {
    SandboxSpec {
        deny_network: false,
        scrub_env: false,
        limits: ResourceLimits {
            cpu_seconds: None,
            file_size_bytes: None,
            max_processes: None,
        },
        wall_clock_timeout: Some(Duration::from_secs(secs)),
        ..Default::default()
    }
}

#[test]
fn a_hung_child_is_killed_at_the_wall_clock_deadline() {
    let caps = Capabilities::detect();
    let started = Instant::now();
    let out = run(&spec_with_deadline(1), "sleep", &["999".to_string()], &caps)
        .expect("the sandbox launches the child");
    let elapsed = started.elapsed();

    assert!(
        out.timed_out,
        "the hung child must be reported as timed out"
    );
    assert_eq!(out.code, None, "a killed child has no normal exit code");
    assert!(
        elapsed < Duration::from_secs(10),
        "the deadline (1s) must fire promptly, not block on the 999s sleep — took {elapsed:?}"
    );
    assert_eq!(
        out.enforced.wall_clock_timeout.as_deref(),
        Some("1s"),
        "the enforced report states the deadline"
    );
}

#[test]
fn the_deadline_reaps_the_whole_process_group_not_just_the_child() {
    let caps = Capabilities::detect();
    let pid_file = std::env::temp_dir().join(format!("dx-e2-{}.pid", std::process::id()));
    let _ = std::fs::remove_file(&pid_file);

    // `sh` backgrounds a grandchild `sleep` (same process group, no job control),
    // records its PID, then blocks. The deadline must kill the grandchild too —
    // proving the whole group is reaped, the heart of "no orphans" (E-2).
    let script = format!("sleep 999 & echo $! > {}; sleep 999", pid_file.display());
    let out = run(
        &spec_with_deadline(1),
        "sh",
        &["-c".to_string(), script],
        &caps,
    )
    .expect("the sandbox launches the shell");
    assert!(out.timed_out, "the shell must hit the deadline");

    // Read the grandchild PID the shell recorded, then confirm it is gone.
    let pid: i32 = std::fs::read_to_string(&pid_file)
        .expect("the shell wrote the grandchild pid")
        .trim()
        .parse()
        .expect("a numeric pid");
    let _ = std::fs::remove_file(&pid_file);

    // Give the kernel a moment to deliver SIGKILL and reap, then probe the pid.
    std::thread::sleep(Duration::from_millis(200));
    // `kill(pid, 0)` returns -1 with ESRCH when no such process exists.
    let alive = unsafe { libc::kill(pid, 0) } == 0;
    assert!(
        !alive,
        "the grandchild (pid {pid}) must be reaped with the group, not orphaned"
    );
}

#[test]
fn a_fast_child_completes_normally_without_tripping_the_deadline() {
    let caps = Capabilities::detect();
    let out = run(&spec_with_deadline(30), "true", &[], &caps).expect("launch");
    assert!(
        !out.timed_out,
        "a fast child must not be reported as timed out"
    );
    assert_eq!(out.code, Some(0), "`true` exits 0");
}
