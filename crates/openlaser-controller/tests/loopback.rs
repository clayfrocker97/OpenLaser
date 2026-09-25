// SPDX-License-Identifier: GPL-3.0-or-later

//! The task against the simulator over loopback UDP. Every vendor sequence
//! the task sends is pinned write for write, so a refactor that changes a
//! single word fails here.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::cast_possible_truncation,
    reason = "tests drive a known plant"
)]

use openlaser_controller::alarms::Rule;
use openlaser_controller::bindings::{Bindings, HomeOutputs};
use openlaser_controller::operations::motion::Request;
use openlaser_controller::operations::outputs::Plan;
use openlaser_controller::session::Quality;
use openlaser_controller::simulator::{Control, Fault, Simulator};
use openlaser_controller::state::{Connection, ProgramState};
use openlaser_controller::streaming::{Ending, Program};
use openlaser_controller::{Error, Machine};
use openlaser_core::LaserMode;
use openlaser_protocol::records::{self, PulsedFields, Record};
use openlaser_protocol::requests::{self, OutputBank, Write};
use openlaser_protocol::sequences::{self, DualDrive, ModeSwitch, Shutdown};
use std::time::Duration;

/// Native apps use multiple runtime workers. A command reply must make its
/// completed state available immediately, without waiting for another poll.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn command_replies_already_expose_the_completed_connection_and_parameters() {
    let simulator = Simulator::start().await.unwrap();
    let control = simulator.control();
    control.head_reference(true);
    control.rules(&bindings().rules);
    control.time_scale(20.);
    let machine = Machine::spawn(simulator.config());
    for _ in 0..16 {
        let identity = machine.connect().await.unwrap();
        let state = machine.state();
        assert!(matches!(state.connection, Connection::Connected { .. }));
        assert_eq!(state.identity, Some(identity));
        assert!(state.feedback.is_some(), "Connect returned before its first feedback");

        machine.configure(bindings()).await.unwrap();
        assert_eq!(machine.state().configured_mode, Some(LaserMode::Fiber));
        let verified = machine.read_parameters().await.unwrap();
        let state = machine.state();
        assert!(state.operation.is_none());
        assert!(state.session.parameters_verified);
        assert_eq!(state.configuration.unwrap().verified, verified);

        machine.disconnect().await.unwrap();
        let state = machine.state();
        assert!(matches!(state.connection, Connection::Disconnected));
        assert!(state.feedback.is_none());
    }
    machine.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_released_owned_press_cannot_start_or_affect_a_later_press() {
    use openlaser_controller::lease::Lease;
    let (_simulator, control, machine) = homed().await;
    let press = |sequence| Lease { client: "test-browser".into(), sequence };
    let plan = || Plan {
        name: "pointer".into(),
        on: vec![sequences::Step::Write(requests::digital_output(3, true).unwrap())],
        off: vec![requests::digital_output(3, false).unwrap()],
        lease: Duration::from_millis(300),
        duration: None,
    };
    machine.release_owned(Some(press(1))).await.unwrap();
    assert!(
        machine.start_outputs(plan(), Some(press(1)), std::time::Instant::now()).await.is_err()
    );
    let active =
        machine.start_outputs(plan(), Some(press(2)), std::time::Instant::now()).await.unwrap();
    tokio::time::sleep(Duration::from_millis(70)).await;
    machine.release_owned(Some(press(1))).await.unwrap();
    assert_ne!(control.view().outputs & 4, 0, "an old release must not turn off the current press");
    for _ in 0..5 {
        machine.heartbeat_owned(press(1)).unwrap();
        tokio::time::sleep(Duration::from_millis(75)).await;
    }
    tokio::time::timeout(Duration::from_secs(2), active.finished()).await.unwrap().unwrap();
    assert_eq!(control.view().outputs & 4, 0, "an old heartbeat must not renew a later press");
    machine.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_program_requires_its_exact_compiled_configuration() {
    use openlaser_controller::session::Configuration;
    let (_simulator, control, machine) = homed().await;
    let changes: [fn(&mut Configuration); 6] = [
        |c| c.epoch += 1,
        |c| c.binding += 1,
        |c| c.mode = LaserMode::Co2,
        |c| c.verified.scale += 1,
        |c| c.verified.cycle_us += 1,
        |c| c.verified.banks[4][8] += 1,
    ];
    control.clear_writes();
    for change in changes {
        let mut stale = program(&machine, 100, 1.);
        change(&mut stale.configuration);
        assert!(
            machine.run(stale).await.unwrap_err().to_string().contains("configuration changed")
        );
        assert!(control.writes().is_empty(), "stale code cannot send an enabling write");
    }
    machine.shutdown().await.unwrap();
}

/// A fiber machine with a head: the manual signal on port 1, the
/// origin-done output on port 2, ports 1 to 3 switched off by a shutdown,
/// and a door on input 4 that trips when low.
fn bindings() -> Bindings {
    Bindings {
        mode: LaserMode::Fiber,
        head_enabled: true,
        shutdown: Shutdown {
            mode: LaserMode::Fiber,
            head_enabled: true,
            co2_skip_head_cancel: false,
            extended_outputs: false,
            co2_analog_channel: 0,
            gas_channels: [1, 0, 0],
            point_laser_frequency: 0,
            rapid_deceleration: 5999,
            ports: vec![1, 2, 3],
            raise: None,
        },
        home: HomeOutputs { z_origin_done_port: 2, manual_signal_port: 1 },
        mode_switch: ModeSwitch {
            mode: LaserMode::Fiber,
            head_enabled: true,
            cleanup: vec![requests::digital_outputs(OutputBank::Standard, 0x3ff, 0)],
            enable: None,
            xy_limits: [(-500, 1_000_000); 2],
        },
        dual_drive: Some(DualDrive { xy_limits: [(-500, 1_000_000); 2] }),
        rules: vec![Rule {
            id: 1000,
            label: "Door alarm".into(),
            input: 4,
            active_low: true,
            run_only: false,
            latch: false,
            gas_valve: None,
        }],
        co2_pwm_sync_port: None,
    }
}

/// The stop sequence for [`bindings`]: axes decelerated, head cancelled,
/// FIFO stopped, both laser channels off, gas analog to zero, ports off,
/// FIFO cleared.
fn abort() -> Vec<Vec<u32>> {
    vec![
        vec![1, 31, 2, 5999, 200_000],
        vec![101],
        vec![3],
        vec![9999, 3, 0, 0, 0],
        vec![9999, 17, 0, 0, 0],
        vec![9999, 4, 0, 0],
        vec![9999, 2, 7, 0],
        vec![1],
    ]
}

fn words(writes: &[Write]) -> Vec<Vec<u32>> {
    writes.iter().map(|write| write.words.clone()).collect()
}

fn step(delta: [i32; 2], held: bool) -> Request {
    Request { delta, speed: 50_000, acceleration: 5999, held, deceleration: 5999 }
}

/// Item 1, `steps` single-pulse moves in X with the laser on, the end tag
/// and the barrier, in blocks of a hundred records.
fn program(machine: &Machine, steps: usize, expected_seconds: f64) -> Program {
    let mut items = vec![Record::Item(1)];
    let fields = PulsedFields { power: 50, frequency: 5000, tail: 0 };
    items.extend((0..steps).map(|_| Record::pulsed(1, 0, fields)));
    items.extend([Record::Item(0xffff_fffe), Record::Barrier]);
    let blocks =
        items.chunks(100).map(|chunk| records::encode(chunk).expect("valid records")).collect();
    Program {
        prepare_head: false,
        position: {
            let p = machine.state().feedback.unwrap().position_mm;
            [p[0], p[1]]
        },
        configuration: machine.state().configuration.expect("accepted configuration"),
        blocks,
        mode: LaserMode::Fiber,
        co2_pwm_type: 0,
        expected_seconds,
    }
}

async fn connected() -> (Simulator, Control, Machine) {
    let simulator = Simulator::start().await.expect("a loopback socket");
    let control = simulator.control();
    // Most operation tests start with an already referenced head. Startup
    // tests below use the simulator's unreferenced default explicitly.
    control.head_reference(true);
    control.rules(&bindings().rules);
    let machine = Machine::spawn(simulator.config());
    machine.connect().await.expect("connect");
    machine.configure(bindings()).await.expect("configure");
    (simulator, control, machine)
}

async fn homed() -> (Simulator, Control, Machine) {
    let (simulator, control, machine) = connected().await;
    machine.read_parameters().await.expect("parameters");
    machine.home(false).await.expect("home");
    control.clear_writes();
    (simulator, control, machine)
}

async fn process_alarms() -> (Simulator, Control, Machine) {
    let simulator = Simulator::start().await.unwrap();
    let control = simulator.control();
    let mut bound = bindings();
    for (id, input, label) in [(56, 3, "Cooling water"), (60, 5, "Laser source")] {
        bound.rules.push(Rule {
            id,
            input,
            label: label.into(),
            active_low: true,
            run_only: false,
            latch: false,
            gas_valve: None,
        });
    }
    control.rules(&bound.rules);
    control.input(3, Some(false));
    control.input(5, Some(false));
    let machine = Machine::spawn(simulator.config());
    machine.connect().await.unwrap();
    machine.configure(bound).await.unwrap();
    (simulator, control, machine)
}

#[tokio::test]
async fn setup_motion_works_with_process_alarms_but_jobs_and_laser_outputs_do_not() {
    let (_simulator, control, machine) = process_alarms().await;
    machine.read_parameters().await.unwrap();
    machine.switch_mode().await.unwrap();
    machine.read_parameters().await.unwrap();
    machine.home(false).await.unwrap();
    machine.calibrate().await.unwrap();
    machine.home(true).await.unwrap();
    for axis in 0..2 {
        let mut delta = [0; 2];
        delta[axis] = 1000;
        machine
            .travel(Request {
                delta,
                speed: 100_000,
                acceleration: 5999,
                held: false,
                deceleration: 5999,
            })
            .await
            .unwrap();
    }
    machine
        .outputs(Plan {
            name: "manual Z".into(),
            on: vec![sequences::Step::Write(requests::head_move(10, 100))],
            off: vec![requests::head_cancel()],
            lease: Duration::from_millis(300),
            duration: Some(Duration::from_millis(150)),
        })
        .await
        .unwrap();
    machine.home(true).await.unwrap();
    assert!(machine.state().alarms.iter().any(|row| row.id == Some(56) && row.blocking));
    assert!(machine.state().alarms.iter().any(|row| row.id == Some(60) && row.blocking));
    control.clear_writes();
    for prepare_head in [false, true] {
        let mut cut = program(&machine, 100, 1.);
        cut.prepare_head = prepare_head;
        let error = machine.run(cut).await.unwrap_err().to_string();
        assert!(error.contains("Cooling water") || error.contains("Laser source"), "{error}");
        assert!(control.writes().is_empty());
    }
    assert!(
        machine
            .outputs(Plan {
                name: "laser output".into(),
                on: vec![sequences::Step::Write(requests::digital_output(3, true).unwrap())],
                off: vec![requests::digital_output(3, false).unwrap()],
                lease: Duration::from_millis(300),
                duration: Some(Duration::from_millis(100)),
            })
            .await
            .is_err()
    );
    assert!(control.writes().is_empty());
    assert!(control.view().pwm.iter().all(|p| p[1] == 0));
    control.input(3, Some(true));
    until(&machine, |state| !state.alarms.iter().any(|row| row.id == Some(56))).await;
    assert!(machine.run(program(&machine, 100, 1.)).await.is_err());
    control.input(5, Some(true));
    until(&machine, |state| state.blocked.is_none()).await;
    machine.run(program(&machine, 100, 1.)).await.unwrap();
    machine.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_new_process_alarm_keeps_a_manual_jog_running_but_pauses_a_cut() {
    let (_simulator, control, machine) = process_alarms().await;
    control.input(3, Some(true));
    control.input(5, Some(true));
    machine.read_parameters().await.unwrap();
    machine.home(false).await.unwrap();
    let press = openlaser_controller::lease::Lease { client: "process-motion".into(), sequence: 1 };
    let motion = machine
        .start_travel(
            Request {
                delta: [100_000, 0],
                speed: 1000,
                acceleration: 5999,
                held: true,
                deceleration: 5999,
            },
            Some(press.clone()),
            std::time::Instant::now(),
        )
        .await
        .unwrap();
    until(&machine, |state| state.feedback.as_ref().is_some_and(|f| !f.stationary)).await;
    control.input(3, Some(false));
    for _ in 0..6 {
        machine.heartbeat_owned(press.clone()).unwrap();
        tokio::time::sleep(Duration::from_millis(75)).await;
    }
    assert!(machine.state().operation.is_some());
    assert!(machine.state().blocked.is_some());
    machine.release_owned(Some(press)).await.unwrap();
    motion.finished().await.unwrap();
    control.input(3, Some(true));
    until(&machine, |state| state.blocked.is_none()).await;
    let task = tokio::spawn({
        let machine = machine.clone();
        async move { machine.run(program(&machine, 20_000, 5.)).await }
    });
    until(&machine, |state| {
        state.program.as_ref().is_some_and(|p| p.state == ProgramState::Running)
    })
    .await;
    control.input(5, Some(false));
    assert!(matches!(task.await.unwrap().unwrap(), Ending::Held));
    assert!(machine.state().alarms.iter().any(|row| row.id == Some(60)));
    assert!(control.view().pwm.iter().all(|p| p[1] == 0));
    machine.shutdown().await.unwrap();
}

/// Waits until the published state satisfies `done`, within two seconds.
async fn until(machine: &Machine, done: impl Fn(&openlaser_controller::State) -> bool) {
    let mut watch = machine.watch();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while !done(&watch.borrow_and_update()) {
        tokio::time::timeout_at(deadline, watch.changed())
            .await
            .expect("state in time")
            .expect("task alive");
    }
}

/// Connecting reads the identity, refuses nothing on a healthy plant and
/// publishes feedback with the plant's scale and cycle; disconnecting
/// publishes the disconnected state and refuses further commands.
#[tokio::test]
async fn connect_reads_identity_and_publishes_feedback() {
    let (_simulator, _control, machine) = connected().await;
    let state = machine.state();
    assert!(matches!(state.connection, Connection::Connected { epoch: 1, .. }));
    assert_eq!(state.identity.map(|i| (i.product_id, i.program_version)), Some((103, 20_177)));
    let feedback = state.feedback.expect("feedback");
    assert_eq!((feedback.scale, feedback.cycle_us), (1000, 250));
    assert_eq!(feedback.fifo.capacity, Some(100_000));
    assert_eq!(state.configured_mode, Some(LaserMode::Fiber));
    machine.disconnect().await.expect("disconnect");
    assert_eq!(machine.state().connection, Connection::Disconnected);
    assert_eq!(machine.home(false).await, Err(Error::Disconnected));
}

/// Go Origin sends the head home, switches the origin-done output off,
/// starts the XY search once the head reports done, holds the manual
/// signal while the axes search and drops it once both report a reference.
#[tokio::test]
async fn go_origin_runs_the_vendor_sequence() {
    let (_simulator, control, machine) = connected().await;
    assert!(!machine.state().session.homed);
    machine.home(false).await.expect("home");
    assert_eq!(
        words(&control.take_writes()),
        [vec![102], vec![9999, 2, 2, 0], vec![2, 3, 0], vec![9999, 2, 1, 1], vec![9999, 2, 1, 0]]
    );
    let state = machine.state();
    assert!(state.session.homed);
    assert_eq!(state.feedback.expect("feedback").referenced, [true, true]);
    assert_eq!(control.view().referenced, [true, true, false, true, false]);
    assert_eq!(control.view().outputs, 0);
}

/// Manual relative travel is available before referencing, with the same
/// verified limits, output checks and stop behavior. It grants no job reference.
#[tokio::test]
async fn manual_travel_before_home_keeps_limits_and_program_requirements() {
    let (_simulator, control, machine) = connected().await;
    control.head_reference(false);
    machine.read_parameters().await.unwrap();
    until(&machine, |s| s.blocked.is_some()).await;
    control.clear_writes();
    assert_eq!(
        machine.travel(step([2_000_000, 0], false)).await,
        Err(Error::Refused("the target is outside the soft limits".into()))
    );
    assert!(control.take_writes().is_empty());
    machine.travel(step([1_000, 0], false)).await.unwrap();
    assert!((control.view().position_mm[0] - 1.).abs() < 1e-6);
    assert!(!machine.state().session.homed);
    assert_eq!(
        machine.run(program(&machine, 100, 1.)).await,
        Err(Error::Refused("run Go Origin first".into()))
    );
    let held = tokio::spawn({
        let machine = machine.clone();
        async move { machine.travel(step([0, 1_000_000], true)).await }
    });
    until(&machine, |s| s.feedback.as_ref().is_some_and(|f| !f.stationary)).await;
    control.fault(Fault::EmergencyStop, true);
    assert!(held.await.unwrap().is_err());
    assert!(matches!(machine.state().connection, Connection::Faulted { .. }));
    assert!(control.writes().contains(&requests::rapid_stop(5999)));
    let stopped = control.view();
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(
        control
            .view()
            .position_mm
            .iter()
            .zip(stopped.position_mm)
            .all(|(a, b)| (a - b).abs() < 1e-9),
        "faulted jog kept moving"
    );
    assert_eq!(stopped.outputs, 0);
    assert!(!machine.state().session.homed);
    assert!(stopped.position_mm[1] < 1_000.);
    machine.shutdown().await.unwrap();
}

/// A step moves by exactly one jog write and answers when the axis is
/// stationary at the target; a held jog aims at the soft limit and stops
/// on release, or by itself once the heartbeat lapses; a target past the
/// limit is refused before anything is sent.
#[tokio::test]
async fn jogs_follow_the_limits_and_the_heartbeat() {
    let (_simulator, control, machine) = homed().await;
    assert_eq!(
        machine.travel(step([2_000_000, 0], false)).await,
        Err(Error::Refused("the target is outside the soft limits".into()))
    );
    assert!(control.take_writes().is_empty());

    machine.travel(step([10_000, 0], false)).await.expect("step");
    assert_eq!(words(&control.take_writes()), [vec![3, 0, 50_000, 5999, 59_990, 10_000]]);
    assert!((control.view().position_mm[0] - 10.).abs() < 1e-6);

    let held = tokio::spawn({
        let machine = machine.clone();
        async move { machine.travel(step([0, 1_000_000], true)).await }
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    machine.heartbeat().expect("heartbeat");
    tokio::time::sleep(Duration::from_millis(150)).await;
    machine.release().await.expect("release");
    held.await.expect("task").expect("held jog");
    assert_eq!(
        words(&control.take_writes()),
        [vec![3, 1, 50_000, 5999, 59_990, 1_000_000], vec![1, 31, 2, 5999, 200_000]]
    );
    let y = control.view().position_mm[1];
    assert!(y > 1. && y < 100., "released after a short travel, at {y} mm");

    let remaining = 1_000_000 - (y * 1000.).round() as i32;
    machine.travel(step([0, remaining], true)).await.expect("a held jog left to lapse");
    assert_eq!(
        words(&control.take_writes()),
        [
            vec![3, 1, 50_000, 5999, 59_990, remaining.cast_unsigned()],
            vec![1, 31, 2, 5999, 200_000]
        ]
    );
    assert!(control.view().position_mm[1] < 100.);
}

/// A run resets the counter, clears the FIFO, uploads with consecutive
/// stamps, installs the XY mask, starts, and once every record executed
/// stops the FIFO and switches the outputs off; the plant ends where the
/// pulses put it with the laser fields applied.
#[tokio::test]
async fn a_program_streams_and_completes() {
    let (_simulator, control, machine) = homed().await;
    let ending = machine.run(program(&machine, 250, 0.1)).await.expect("run");
    assert_eq!(ending, Ending::Completed);
    let writes = control.take_writes();
    let uploads: Vec<&Write> = writes.iter().filter(|write| write.address == 102).collect();
    assert_eq!(uploads.len(), 3);
    assert_eq!(uploads.iter().map(|write| write.words[0]).collect::<Vec<_>>(), [1, 2, 3]);
    let others: Vec<Vec<u32>> =
        writes.iter().filter(|write| write.address != 102).map(|w| w.words.clone()).collect();
    assert_eq!(
        others,
        [
            vec![9999, 16],
            vec![1],
            vec![9999, 1, 0x6680_0003, 0],
            vec![2],
            vec![3],
            vec![9999, 3, 0, 0, 0],
            vec![9999, 17, 0, 0, 0],
            vec![9999, 4, 0, 0],
            vec![9999, 2, 7, 0],
        ]
    );
    let view = control.view();
    assert!((view.position_mm[0] - 0.25).abs() < 1e-9);
    assert_eq!(view.pwm[0], [0, 0, 0], "the shutdown switched the laser off");
    assert!(!view.running);
    let program = machine.state().program.expect("program view");
    assert_eq!((program.state, program.uploaded, program.total), (ProgramState::Completed, 3, 3));
}

/// A hold sends the vendor's manual stop: the FIFO stop alone while the
/// program runs from the FIFO, the laser, gas and outputs off, the head
/// cancel, then the fiber head's mode. It waits for the axes to settle,
/// captures the checkpoint and clears the FIFO; the run answers held and
/// the view keeps the checkpoint. A stop does the same and answers stopped.
#[tokio::test]
async fn hold_and_stop_capture_checkpoints() {
    let (_simulator, control, machine) = homed().await;
    control.fault(Fault::StoppedFifoActivity, true);
    let run = tokio::spawn({
        let machine = machine.clone();
        async move { machine.run(program(&machine, 20_000, 5.)).await }
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    machine.hold().await.expect("hold");
    assert_eq!(run.await.expect("task"), Ok(Ending::Held));
    let program_view = machine.state().program.expect("program view");
    assert_eq!(program_view.state, ProgramState::Held);
    let checkpoint = program_view.checkpoint.expect("checkpoint");
    assert_eq!(checkpoint.item, 1);
    assert!(checkpoint.progress > 100 && checkpoint.progress < 20_000);
    assert!(checkpoint.position_mm[0] > 0.1);
    let tail: Vec<Vec<u32>> =
        words(&control.take_writes()).into_iter().filter(|w| w[0] != 9999 || w[1] != 16).collect();
    let end = tail.len() - 9;
    assert_eq!(
        tail[end..],
        [
            vec![3],
            vec![9999, 3, 0, 0, 0],
            vec![9999, 17, 0, 0, 0],
            vec![9999, 4, 0, 0],
            vec![9999, 2, 65535, 0],
            vec![101],
            vec![118, 5, 0],
            vec![3],
            vec![1],
        ]
    );
    assert!(!control.view().running);
    assert!(matches!(machine.state().connection, Connection::Connected { .. }));
    assert!(machine.state().session.homed);

    let run = tokio::spawn({
        let machine = machine.clone();
        async move { machine.run(program(&machine, 20_000, 5.)).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    machine.stop().await.expect("stop");
    assert_eq!(run.await.expect("task"), Ok(Ending::Stopped));
    assert_eq!(machine.state().program.expect("program view").state, ProgramState::Stopped);
    assert_eq!(machine.state().operation, None);
}

/// A stop write the controller refuses was not carried out, and the
/// vendor sends it again: laser off refused twice goes through on the
/// third try. A write refused every time is reported, the rest of the stop
/// still goes out, and the connection stays, as the vendor keeps it.
#[tokio::test]
async fn refused_stop_writes_are_retried_and_never_drop_the_connection() {
    let (_simulator, control, machine) = homed().await;
    control.fault(Fault::StoppedFifoActivity, true);
    control.refuse(vec![9999, 3], 2);
    control.refuse(vec![101], 10);
    let run = tokio::spawn({
        let machine = machine.clone();
        async move { machine.run(program(&machine, 20_000, 5.)).await }
    });
    tokio::time::sleep(Duration::from_millis(400)).await;
    machine.hold().await.expect("hold");
    assert_eq!(run.await.expect("task"), Ok(Ending::Held));
    let writes = words(&control.take_writes());
    assert!(writes.contains(&vec![9999, 3, 0, 0, 0]), "accepted on the third try");
    assert!(writes.contains(&vec![9999, 17, 0, 0, 0]), "the rest of the stop went out");
    assert!(!writes.contains(&vec![101]), "refused every time");
    let state = machine.state();
    assert!(matches!(state.connection, Connection::Connected { .. }));
    assert!(state.last_error.is_some_and(|e| e.contains("refused")), "the operator is told");
}

#[tokio::test]
async fn missed_head_reads_after_pause_retry_without_losing_the_saved_job() {
    let (_simulator, control, machine) = homed().await;
    let configuration = machine.state().configuration;
    let run = machine
        .start_run_at(program(&machine, 20_000, 5.), std::time::Instant::now())
        .await
        .unwrap();
    until(&machine, |s| s.program.as_ref().is_some_and(|p| p.started)).await;
    control.drop_reads(10_000, 2);
    machine.hold().await.unwrap();
    assert_eq!(run.finished().await.unwrap(), Ending::Held);
    let state = machine.state();
    assert!(matches!(state.connection, Connection::Connected { .. }));
    assert_eq!(state.configuration, configuration);
    assert!(state.program.as_ref().unwrap().checkpoint.is_some());
    let checkpoint = state.program.unwrap().checkpoint;
    let reads = control.read_count(10_000);
    control.drop_reads(10_000, 1);
    until(&machine, |_| control.read_count(10_000) >= reads + 2).await;
    assert_eq!(machine.state().program.unwrap().checkpoint, checkpoint);
    assert!(matches!(machine.state().connection, Connection::Connected { .. }));
    machine.shutdown().await.unwrap();
}

#[tokio::test]
async fn read_retries_yield_to_stop_and_persistent_head_loss_still_faults() {
    let (_simulator, control, machine) = homed().await;
    let run = machine
        .start_run_at(program(&machine, 20_000, 5.), std::time::Instant::now())
        .await
        .unwrap();
    until(&machine, |s| s.program.as_ref().is_some_and(|p| p.started)).await;
    let reads = control.read_count(10_000);
    control.drop_reads(10_000, 20);
    until(&machine, |_| control.read_count(10_000) > reads).await;
    tokio::time::timeout(Duration::from_millis(800), machine.stop()).await.unwrap().unwrap();
    assert!(!control.view().running);
    assert!(control.view().pwm.iter().all(|p| p[1] == 0));
    assert!(run.finished().await.is_err(), "persistent loss cannot admit a checkpoint");
    assert!(matches!(machine.state().connection, Connection::Faulted { .. }));
    assert!(machine.state().program.unwrap().checkpoint.is_none());
    machine.shutdown().await.unwrap();
}

#[tokio::test]
async fn stop_during_pause_settling_ends_stopped_without_disconnect() {
    let (_simulator, control, machine) = homed().await;
    let run = machine
        .start_run_at(program(&machine, 20_000, 5.), std::time::Instant::now())
        .await
        .unwrap();
    until(&machine, |s| s.program.as_ref().is_some_and(|p| p.started)).await;
    control.fault(Fault::StopFeedbackBusy, true);
    machine.hold().await.unwrap();
    until(&machine, |s| s.program.as_ref().is_some_and(|p| p.state == ProgramState::Held)).await;
    machine.stop().await.unwrap();
    control.fault(Fault::StopFeedbackBusy, false);
    assert_eq!(run.finished().await.unwrap(), Ending::Stopped);
    assert!(matches!(machine.state().connection, Connection::Connected { .. }));
    assert!(machine.state().session.homed);
    assert_eq!(control.view().outputs, 0);
    machine.shutdown().await.unwrap();
}

#[tokio::test]
async fn a_pause_watchdog_keeps_a_responding_controller_connected() {
    let (_simulator, control, machine) = homed().await;
    let run = machine
        .start_run_at(program(&machine, 20_000, 5.), std::time::Instant::now())
        .await
        .unwrap();
    until(&machine, |s| s.program.as_ref().is_some_and(|p| p.started)).await;
    control.fault(Fault::StopFeedbackBusy, true);
    machine.hold().await.unwrap();
    let outcome = tokio::time::timeout(Duration::from_secs(15), run.finished()).await.unwrap();
    assert!(matches!(outcome, Err(Error::Failed(reason)) if reason.contains("did not settle")));
    let state = machine.state();
    assert!(matches!(state.connection, Connection::Connected { .. }));
    assert!(state.session.homed);
    assert_eq!(state.program.as_ref().unwrap().state, ProgramState::Failed);
    assert!(state.program.unwrap().checkpoint.is_none());
    assert_eq!(control.view().queued_bytes, 0);
    machine.stop().await.unwrap();
    machine.shutdown().await.unwrap();
}

/// A blocking alarm during a run pauses it the way a hold does; relief
/// sends the common reset twice, once for the row and once as the
/// controller tail, and the row leaves once the condition is gone.
#[tokio::test]
async fn an_alarm_pauses_the_program_and_relief_clears_it() {
    let (_simulator, control, machine) = homed().await;
    let run = tokio::spawn({
        let machine = machine.clone();
        async move { machine.run(program(&machine, 20_000, 5.)).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    control.fault(Fault::EmergencyStop, true);
    assert_eq!(run.await.expect("task"), Ok(Ending::Held));
    let state = machine.state();
    assert_eq!(state.alarms.iter().map(|row| row.id).collect::<Vec<_>>(), [Some(8030)]);
    assert!(state.blocked.as_deref().is_some_and(|b| b.contains("Emergency")));
    assert!(matches!(machine.home(false).await, Err(Error::Refused(_))));
    control.clear_writes();
    machine.relieve(Some(8030)).await.expect("relief");
    assert_eq!(words(&control.take_writes()), [vec![9999, 5, 0, 0], vec![9999, 5, 0, 0]]);
    assert!(machine.state().blocked.is_some(), "the condition is still present");
    control.fault(Fault::EmergencyStop, false);
    until(&machine, |state| state.alarms.is_empty()).await;
}

/// A door input that trips blocks operations with the rule's label and
/// releases them when it recovers.
#[tokio::test]
async fn a_door_input_blocks_and_recovers() {
    let (_simulator, control, machine) = connected().await;
    control.input(4, Some(false));
    until(&machine, |state| state.blocked.is_some()).await;
    assert_eq!(
        machine.home(false).await,
        Err(Error::Refused("alarms are active: Door alarm".into()))
    );
    control.input(4, None);
    until(&machine, |state| state.blocked.is_none()).await;
    machine.home(false).await.expect("home once the door closes");
}

/// The mode switch sends its cleanup, the head mode, and the limits, then
/// keeps the physical XY reference but asks for the parameters again; the
/// head-only search relieves the head reference alarm without touching XY.
#[tokio::test]
async fn mode_switch_and_head_relief() {
    let (_simulator, control, machine) = homed().await;
    machine.switch_mode().await.expect("switch");
    let writes = control.take_writes();
    assert_eq!(writes.iter().map(|w| w.address).collect::<Vec<_>>(), [101, 101, 50_202, 50_242]);
    assert_eq!(
        words(&writes),
        [
            vec![9999, 2, 0x3ff, 0],
            vec![118, 5, 0],
            vec![(-500i32).cast_unsigned(), 1_000_000],
            vec![(-500i32).cast_unsigned(), 1_000_000],
        ]
    );
    let state = machine.state();
    assert!(state.session.homed, "a mode switch keeps the XY reference");
    assert_eq!(state.session.mode, Some(LaserMode::Fiber));
    assert!(matches!(machine.travel(step([100, 0], false)).await, Err(Error::Refused(_))));

    machine.relieve(Some(99)).await.expect("head relief");
    assert_eq!(words(&control.take_writes()), [vec![102], vec![9999, 2, 2, 0]]);
    assert!(machine.state().session.homed, "the head search leaves XY referenced");
}

/// Stop is accepted while idle and sends the whole stop sequence; a lost
/// link faults the connection on the next poll, after which commands are
/// refused until an explicit reconnect.
#[tokio::test]
async fn stop_is_always_available_and_a_lost_link_faults() {
    let (_simulator, control, machine) = homed().await;
    let pending_home = std::time::Instant::now();
    machine.stop().await.expect("stop");
    assert_eq!(words(&control.take_writes()), abort());
    assert!(machine.home_at(false, pending_home).await.is_err());
    assert!(control.writes().is_empty(), "Stop also cancels a Home press still being prepared");
    control.fault(Fault::Communications, true);
    until(&machine, |state| matches!(state.connection, Connection::Faulted { .. })).await;
    assert_eq!(machine.stop().await, Err(Error::Disconnected));
    control.fault(Fault::Communications, false);
    machine.connect().await.expect("reconnect");
    assert!(matches!(machine.state().connection, Connection::Connected { epoch: 2, .. }));
    assert!(!machine.state().session.homed, "a reconnect invalidates the reference");
}

/// Calibration sends one command and answers with the quality the head
/// reports.
#[tokio::test]
async fn calibration_reports_the_head_quality() {
    let (_simulator, control, machine) = homed().await;
    control.calibration_quality(Quality::Good);
    assert_eq!(machine.calibrate().await, Ok(Quality::Good));
    assert_eq!(words(&control.take_writes()), [vec![107]]);
    assert_eq!(machine.state().session.calibration, Some(Quality::Good));
}

#[tokio::test]
async fn reported_z_limit_blocks_start_and_unknown_summary_is_not_conceded() {
    use openlaser_controller::simulator::AlarmBank;
    let (_simulator, control, machine) = homed().await;
    machine.calibrate().await.unwrap();
    let compiled = program(&machine, 100, 1.);
    control.head_reference(false);
    control.fault(Fault::HeadUpperLimit, true);
    until(&machine, |s| {
        s.alarms.iter().any(|row| row.id == Some(33))
            && s.alarms.iter().any(|row| row.id == Some(99))
    })
    .await;
    assert_eq!(control.view().alarms, [1 << 24, 0, 1 | (1 << 12)]);
    assert!(machine.state().session.calibration.is_none());
    control.clear_writes();
    assert!(machine.run(compiled).await.is_err());
    assert!(control.writes().is_empty(), "an active limit must refuse before any enabling write");
    assert_eq!(control.view().outputs, 0);
    assert!(control.view().pwm.iter().all(|p| p[1] == 0));
    control.fault(Fault::HeadUpperLimit, false);
    until(&machine, |s| !s.alarms.iter().any(|row| row.id == Some(33))).await;
    machine.home(true).await.unwrap();
    until(&machine, |s| s.alarms.is_empty()).await;
    control.alarm_word(AlarmBank::Controller1, 1 << 24);
    until(&machine, |s| s.blocked.is_some()).await;
    assert!(
        machine.home(false).await.is_err(),
        "an unexplained group 1 bit 24 is not a reference warning"
    );
    machine.shutdown().await.unwrap();
}

#[tokio::test]
async fn z_recovery_works_before_home_without_resetting_or_enabling_other_motion() {
    let simulator = Simulator::start().await.unwrap();
    let control = simulator.control();
    control.rules(&bindings().rules);
    control.head_on_limit(true);
    let machine = Machine::spawn(simulator.config());
    machine.connect().await.unwrap();
    machine.configure(bindings()).await.unwrap();
    until(&machine, |s| s.head_jog_blocked[0].is_none() && s.head_recovery).await;
    assert!(machine.state().head_jog_blocked[1].is_some());
    machine.relieve(None).await.unwrap();
    assert_ne!(control.view().alarms[2] & 1, 0, "Reset cannot release a physical limit");
    control.clear_writes();
    assert!(machine.home(false).await.is_err());
    assert!(machine.calibrate().await.is_err());
    let plan = |up| Plan {
        name: "head".into(),
        on: vec![sequences::Step::Write(requests::head_move(
            5000,
            if up { -1_000_000 } else { 1_000_000 },
        ))],
        off: vec![requests::head_cancel()],
        lease: Duration::from_millis(300),
        duration: None,
    };
    assert!(machine.outputs(plan(true)).await.is_err());
    assert!(control.writes().is_empty());
    let active = machine.start_outputs(plan(false), None, std::time::Instant::now()).await.unwrap();
    for _ in 0..15 {
        machine.heartbeat().unwrap();
        tokio::time::sleep(Duration::from_millis(80)).await;
    }
    assert_eq!(control.view().alarms[2] & 1, 0);
    assert!((0.99..=1.).contains(&control.view().position_mm[3]));
    assert!(!control.view().head_referenced);
    assert_eq!(control.view().position_mm[..2], [0.; 2]);
    assert_eq!(control.view().outputs, 0);
    assert_eq!(control.view().pwm, [[0; 3]; 2]);
    machine.release().await.unwrap();
    active.finished().await.unwrap();
    assert_eq!(words(&control.take_writes()), [vec![109, 10, 1000], vec![101]]);
    machine.home(false).await.unwrap();
    until(&machine, |s| s.alarms.is_empty()).await;
    assert!(machine.state().session.homed);
    machine.shutdown().await.unwrap();
}

#[tokio::test]
async fn stopping_or_faulting_during_startup_head_clearance_never_starts_the_fifo() {
    let (_simulator, control, machine) = homed().await;
    machine
        .outputs(Plan {
            name: "lower fixture head".into(),
            on: vec![sequences::Step::Write(requests::head_move(1000, 20_000))],
            off: vec![requests::head_cancel()],
            lease: Duration::from_secs(2),
            duration: Some(Duration::from_millis(400)),
        })
        .await
        .unwrap();
    assert!(control.view().position_mm[3] > 0.);
    control.fault(Fault::HeadStall, true);
    for alarm in [false, true] {
        let mut compiled = program(&machine, 100, 1.);
        compiled.prepare_head = true;
        control.clear_writes();
        let run = tokio::spawn({
            let machine = machine.clone();
            async move { machine.run(compiled).await }
        });
        until(&machine, |_| control.view().head_command == 102).await;
        if alarm {
            control.fault(Fault::HeadUpperLimit, true);
        } else {
            machine.stop().await.unwrap();
        }
        let result =
            tokio::time::timeout(Duration::from_secs(3), run).await.unwrap().unwrap().unwrap();
        assert_eq!(result, if alarm { Ending::Held } else { Ending::Stopped });
        let writes = control.writes();
        assert!(writes.contains(&requests::head_cancel()));
        assert!(!writes.iter().any(|w| w.address == 102));
        assert!(!writes.contains(&requests::fifo_start()));
        assert!(!writes.contains(&requests::home_xy()));
        assert_eq!(control.view().outputs, 0);
        assert_eq!(control.view().head_command, 0);
    }
    machine.shutdown().await.unwrap();
}

/// A manual output runs its plan with the settle delay, stays on while
/// held, and its off-writes run on release.
#[tokio::test]
async fn manual_outputs_hold_until_released() {
    let (_simulator, control, machine) = homed().await;
    let plan = Plan {
        name: "oxygen".into(),
        on: vec![
            sequences::Step::Write(requests::digital_output(3, true).expect("port")),
            sequences::Step::Delay(50),
            sequences::Step::Write(requests::analog_output(1, 2500).expect("channel")),
        ],
        off: vec![
            requests::analog_output(1, 0).expect("channel"),
            requests::digital_output(3, false).expect("port"),
        ],
        lease: Duration::from_secs(2),
        duration: None,
    };
    let held = tokio::spawn({
        let machine = machine.clone();
        async move { machine.outputs(plan).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    let view = control.view();
    assert_eq!((view.outputs, view.analog), (0b100, [2500, 0]));
    assert_eq!(machine.state().operation.map(|o| o.phase), Some("on".into()));
    machine.release().await.expect("release");
    held.await.expect("task").expect("outputs");
    assert_eq!(
        words(&control.take_writes()),
        [vec![9999, 2, 4, 4], vec![9999, 4, 0, 2500], vec![9999, 4, 0, 0], vec![9999, 2, 4, 0]]
    );
    assert_eq!(control.view().outputs, 0);
}

/// Disconnecting while a program runs stops it first: the run answers
/// failed, the plant is stopped and the state is disconnected.
#[tokio::test]
async fn disconnect_stops_a_running_program() {
    let (_simulator, control, machine) = homed().await;
    let run = tokio::spawn({
        let machine = machine.clone();
        async move { machine.run(program(&machine, 20_000, 5.)).await }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    machine.disconnect().await.expect("disconnect");
    assert_eq!(run.await.expect("task"), Err(Error::Failed("disconnected".into())));
    assert!(!control.view().running);
    assert_eq!(machine.state().connection, Connection::Disconnected);
    assert_eq!(machine.state().program.expect("program view").state, ProgramState::Failed);
}

/// A write whose acknowledgement is lost fails the operation without a
/// retry; the stop sequence follows because the write may have applied.
#[tokio::test]
async fn a_lost_acknowledgement_fails_the_operation() {
    let (_simulator, control, machine) = connected().await;
    control.drop_next_ack();
    assert!(matches!(machine.home(false).await, Err(Error::Link(_))));
    let writes = words(&control.take_writes());
    assert_eq!(writes[0], vec![102]);
    assert_eq!(writes[1..], abort());
    assert!(!machine.state().session.homed);
    assert!(machine.state().last_error.is_some());
}

#[tokio::test]
async fn shutdown_attempts_every_write_after_first_middle_or_last_ack_loss() {
    for lost in [0, 3, 7] {
        let (_simulator, control, machine) = homed().await;
        control.drop_ack_after(lost);
        assert!(matches!(machine.stop().await, Err(Error::Link(_))));
        assert_eq!(words(&control.take_writes()), abort(), "lost acknowledgement {lost}");
        let state = machine.state();
        assert!(matches!(state.connection, Connection::Faulted { .. }));
        assert!(!state.session.homed && !state.session.parameters_verified);
        assert_eq!(machine.travel(step([100, 0], false)).await, Err(Error::Disconnected));
        assert!(control.writes().is_empty());
    }
}

#[tokio::test]
async fn upload_failure_counts_acknowledged_blocks_without_replaying_them() {
    for accepted in [0, 1, 9] {
        let (_simulator, control, machine) = homed().await;
        // Reset and clear are acknowledged before the first upload.
        control.drop_ack_after(2 + accepted);
        assert!(matches!(machine.run(program(&machine, 2500, 1.)).await, Err(Error::Link(_))));
        let state = machine.state();
        let program = state.program.expect("failed program");
        assert_eq!(program.uploaded, accepted);
        assert_eq!(program.state, ProgramState::Failed);
        assert_eq!(program.checkpoint, None);
        let writes = control.writes();
        assert_eq!(writes.iter().filter(|write| write.address == 102).count(), accepted + 1);
        assert!(!writes.contains(&requests::fifo_start()));
        assert!(!control.view().running);
    }
}

#[tokio::test]
async fn release_before_first_jog_write_cancels_the_pending_press() {
    let (_simulator, control, machine) = homed().await;
    control.reply_delay(Duration::from_millis(100));
    let travel = machine.travel(step([0, 1_000_000], true));
    tokio::pin!(travel);
    assert!(tokio::time::timeout(Duration::from_millis(10), &mut travel).await.is_err());
    machine.release().await.expect("release");
    travel.await.expect("cancelled before launch");
    assert!(control.writes().is_empty());
    assert!(control.view().position_mm[1].abs() < 1e-9);
}

#[tokio::test]
async fn held_output_lease_expires_between_slow_feedback_reads() {
    let (_simulator, control, machine) = homed().await;
    let plan = Plan {
        name: "pointer".into(),
        on: vec![sequences::Step::Write(requests::digital_output(3, true).unwrap())],
        off: vec![requests::digital_output(3, false).unwrap()],
        lease: Duration::from_millis(350),
        duration: None,
    };
    let held = tokio::spawn({
        let machine = machine.clone();
        async move { machine.outputs(plan).await }
    });
    until(&machine, |state| state.operation.as_ref().is_some_and(|op| op.phase == "on")).await;
    machine.heartbeat().unwrap();
    control.reply_delay(Duration::from_millis(150));
    tokio::time::timeout(Duration::from_millis(650), async {
        loop {
            if control.writes().contains(&requests::digital_output(3, false).unwrap()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("lease checked before the whole capture finishes");
    control.reply_delay(Duration::ZERO);
    held.await.unwrap().unwrap();
    assert_eq!(control.view().outputs, 0);
}

#[tokio::test]
async fn parameter_failures_and_external_drift_remove_authority() {
    let (_simulator, control, machine) = homed().await;
    let mut banks = control.parameters();
    banks[0][2] += 1000;
    control.drop_next_ack();
    assert!(matches!(machine.apply_parameters(vec![(0, banks[0])]).await, Err(Error::Link(_))));
    assert_eq!(control.parameters()[0], banks[0], "the write applied despite its lost reply");
    let state = machine.state();
    assert!(!state.session.parameters_verified && !state.session.homed);

    for reference in [false, true] {
        let (_simulator, control, machine) = homed().await;
        let job = program(&machine, 100, 1.);
        if reference {
            control.axis_reference(0, false);
            until(&machine, |state| !state.session.homed).await;
        } else {
            let mut changed = control.parameters();
            changed[0][8] ^= 1;
            control.set_parameters(changed);
            until(&machine, |state| matches!(state.connection, Connection::Faulted { .. })).await;
            assert!(!machine.state().session.parameters_verified);
        }
        assert!(machine.run(job).await.is_err());
        assert!(control.writes().is_empty());
        if reference {
            machine.travel(step([100, 0], false)).await.unwrap();
            assert!((control.view().position_mm[0] - 0.1).abs() < 1e-6);
            assert!(!machine.state().session.homed);
        } else {
            assert!(machine.travel(step([100, 0], false)).await.is_err());
            assert!(control.writes().is_empty());
        }
    }
}

#[tokio::test]
async fn stop_bypasses_a_normal_command_backlog_during_upload() {
    let (_simulator, control, machine) = homed().await;
    control.reply_delay(Duration::from_millis(80));
    let run = tokio::spawn({
        let machine = machine.clone();
        async move { machine.run(program(&machine, 20_000, 5.)).await }
    });
    tokio::time::timeout(Duration::from_secs(3), async {
        while !control.writes().iter().any(|write| write.address == 102) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    let mut backlog = Vec::new();
    for _ in 0..128 {
        let machine = machine.clone();
        backlog.push(tokio::spawn(async move { machine.read_parameters().await }));
    }
    let stop = tokio::spawn({
        let machine = machine.clone();
        async move { machine.stop().await }
    });
    tokio::time::timeout(Duration::from_millis(300), async {
        while !control.writes().contains(&requests::rapid_stop(5999)) {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("stop starts after one exchange, before the rest of the upload batch");
    control.reply_delay(Duration::ZERO);
    stop.await.unwrap().unwrap();
    assert_eq!(run.await.unwrap(), Ok(Ending::Stopped));
    for request in backlog {
        drop(request.await.unwrap());
    }
    assert!(!control.writes().contains(&requests::fifo_start()));
}

/// [`bindings`] with the pause raise to 15 mm below the head's origin.
async fn raising() -> (Simulator, Control, Machine) {
    let (simulator, control, machine) = connected().await;
    let mut bound = bindings();
    bound.shutdown.raise =
        Some(sequences::Raise { speed_tenths: 1000, height_thousandths: 15_000 });
    machine.configure(bound).await.expect("configure");
    machine.read_parameters().await.expect("parameters");
    machine.home(false).await.expect("home");
    control.clear_writes();
    (simulator, control, machine)
}

/// Waits until the head is at least `depth` millimetres below its origin.
async fn head_below(control: &Control, depth: f64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while control.view().position_mm[3] < depth {
        assert!(tokio::time::Instant::now() < deadline, "the head did not lower");
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// Waits until the head has risen to `depth` millimetres below its origin.
async fn head_at(control: &Control, depth: f64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    while (control.view().position_mm[3] - depth).abs() > 0.01 {
        assert!(
            tokio::time::Instant::now() < deadline,
            "head at {}",
            control.view().position_mm[3]
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
}

/// The position of the first write with these words.
fn position(writes: &[Vec<u32>], words: &[u32]) -> usize {
    writes.iter().position(|w| w == words).unwrap_or_else(|| panic!("{words:?} in {writes:?}"))
}

/// A nozzle touching the plate mid-cut pauses the program as the vendor's
/// manual stop does: the FIFO stops, the laser and gas go off, the head is
/// cancelled and then sent to the safe height. The pause does not wait for
/// the head to arrive.
#[tokio::test]
async fn plate_contact_stops_the_laser_and_gas_then_raises_the_head() {
    let (_simulator, control, machine) = raising().await;
    let mut cut = program(&machine, 20_000, 5.);
    let mut items = vec![
        Record::Item(1),
        Record::HeightAbsolute { speed_tenths: 3000, height_microns: 19_500 },
    ];
    let fields = PulsedFields { power: 50, frequency: 5000, tail: 0 };
    items.extend((0..20_000).map(|_| Record::pulsed(1, 0, fields)));
    items.extend([Record::Item(0xffff_fffe), Record::Barrier]);
    cut.blocks = items.chunks(100).map(|chunk| records::encode(chunk).unwrap()).collect();
    let run = tokio::spawn({
        let machine = machine.clone();
        async move { machine.run(cut).await }
    });
    head_below(&control, 19.4).await;
    control.clear_writes();
    control.fault(Fault::HeadTouch, true);
    assert_eq!(run.await.unwrap(), Ok(Ending::Held));
    let writes = words(&control.take_writes());
    let stop = position(&writes, &[3]);
    let laser = position(&writes, &[9999, 3, 0, 0, 0]);
    let gas = position(&writes, &[9999, 4, 0, 0]);
    let cancel = position(&writes, &[101]);
    let raise = position(&writes, &[103, 1000, 15_000]);
    let mode = position(&writes, &[118, 5, 0]);
    assert!(
        stop < laser && laser < gas && gas < cancel && cancel < raise && raise < mode,
        "{writes:?}"
    );
    assert!(!writes.iter().any(|w| w.starts_with(&[1, 31])), "no rapid stop while the FIFO runs");
    head_at(&control, 15.).await;
    let view = control.view();
    assert_eq!((view.analog, view.pwm[0][2], view.pwm[1][2]), ([0, 0], 0, 0));
    assert!(machine.state().alarms.iter().any(|row| row.id == Some(38)));
    machine.shutdown().await.unwrap();
}

/// A head jogged down onto the plate stops and rises to the safe height
/// even though the operator is still holding the control, and may then be
/// jogged further up while the contact persists.
#[tokio::test]
async fn a_head_jogged_onto_the_plate_rises_to_the_safe_height() {
    let (_simulator, control, machine) = raising().await;
    let down = Plan {
        name: "head down".into(),
        on: vec![sequences::Step::Write(requests::head_move(300, 1_000_000))],
        off: vec![requests::head_cancel()],
        lease: Duration::from_secs(5),
        duration: None,
    };
    let jog = tokio::spawn({
        let machine = machine.clone();
        async move { machine.outputs(down).await }
    });
    head_below(&control, 19.5).await;
    control.fault(Fault::HeadTouch, true);
    jog.await.unwrap().expect("the jog ends by rising, not by failing");
    head_at(&control, 15.).await;
    assert!(matches!(machine.state().connection, Connection::Connected { .. }));
    let up = Plan {
        name: "head up".into(),
        on: vec![sequences::Step::Write(requests::head_move(300, -1000))],
        off: vec![requests::head_cancel()],
        lease: Duration::from_secs(5),
        duration: None,
    };
    machine.outputs(up).await.expect("up is away from the plate");
    machine.shutdown().await.unwrap();
}

/// A head controller that stays busy after the touch does not hold up the
/// pause for long: it ends a few seconds after X and Y stop, with the gas
/// off and its checkpoint, instead of failing at the settle watchdog.
#[tokio::test]
async fn a_pause_does_not_wait_for_a_head_that_stays_busy() {
    let (_simulator, control, machine) = raising().await;
    let mut cut = program(&machine, 20_000, 5.);
    let mut items = vec![
        Record::Item(1),
        Record::HeightAbsolute { speed_tenths: 3000, height_microns: 19_500 },
    ];
    let fields = PulsedFields { power: 50, frequency: 5000, tail: 0 };
    items.extend((0..20_000).map(|_| Record::pulsed(1, 0, fields)));
    items.extend([Record::Item(0xffff_fffe), Record::Barrier]);
    cut.blocks = items.chunks(100).map(|chunk| records::encode(chunk).unwrap()).collect();
    let run = tokio::spawn({
        let machine = machine.clone();
        async move { machine.run(cut).await }
    });
    head_below(&control, 19.4).await;
    control.fault(Fault::HeadStall, true);
    control.fault(Fault::HeadTouch, true);
    let ending = tokio::time::timeout(Duration::from_secs(5), run).await;
    assert_eq!(ending.expect("the pause ends promptly").unwrap(), Ok(Ending::Held));
    assert!(machine.state().program.is_some_and(|p| p.checkpoint.is_some()));
    assert_eq!(control.view().analog, [0, 0]);
    machine.shutdown().await.unwrap();
}
