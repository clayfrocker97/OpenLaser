// SPDX-License-Identifier: GPL-3.0-or-later

//! The socket task and the handle that drives it.
//!
//! One task owns the link. Commands arrive on a channel and are answered
//! when they complete: a home command answers once the reference is
//! established, a run once the program ends. Between commands the task
//! polls the controller, feeds the alarm monitor, advances the active
//! operation and publishes the state. Stop, hold, release and heartbeat
//! are accepted while an operation runs; anything else is refused as busy.

use crate::alarms::{Concession, Monitor};
use crate::bindings::Bindings;
use crate::config::Config;
use crate::lease::{Lease, Leases};
use crate::link::{Link, LinkError};
use crate::operations::calibrate::Calibrate;
use crate::operations::home::{self, Home};
use crate::operations::mode::Switch;
use crate::operations::motion::{self, Motion};
use crate::operations::outputs::{self, Outputs, Plan};
use crate::operations::relief::Relief;
use crate::operations::{Operation, Step, admit_alarms, parameters};
use crate::session::{Quality, Session, Verified};
use crate::snapshot::{Capture, Snapshot};
use crate::state::{
    Connection, Feedback, FifoView, HeadView, Identity, OperationView, ProgramState, ProgramView,
    SessionView, State,
};
use crate::streaming::{Ending, Program, Run};
use crate::{Error, Result};
use openlaser_protocol::alarms::{self as catalogue, DetailCache};
use openlaser_protocol::feedback;
use openlaser_protocol::registers::{self, PARAMETER_BANK_WORDS};
use openlaser_protocol::requests::{Read, Write};
use openlaser_protocol::sequences;
use std::collections::VecDeque;
use std::net::{Ipv4Addr, SocketAddrV4};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, oneshot, watch};
use tokio::time::MissedTickBehavior;

/// The product id the MCC100 reports.
const MCC100: u32 = 103;
/// How long a head-only read waits for its reply before it is skipped, so
/// a lost datagram cannot hold up the program upload.
const HEAD_WATCH_TIMEOUT: Duration = Duration::from_millis(20);
/// Consecutive unanswered feedback reads after which the link is faulted.
/// Each earlier miss already holds a running cut; three polls are 150 ms.
const MAX_MISSED_POLLS: u8 = 3;
/// Commands waiting for the controller task before senders wait too. Stop
/// and lease renewals bypass this queue.
const COMMAND_QUEUE: usize = 64;

/// A parameter bank to apply: the axis index and its fourteen words.
pub type Bank = (u8, [u32; PARAMETER_BANK_WORDS]);

type Reply<T> = oneshot::Sender<Result<T>>;

#[derive(Clone, Copy)]
enum Travel {
    Jog,
    Position,
    Table([i32; 2]),
}

enum Command {
    Connect(Option<(SocketAddrV4, Ipv4Addr)>, Reply<Identity>),
    Configure(Box<Bindings>, Reply<()>),
    Home { head_only: bool, reply: Reply<()> },
    Calibrate(Reply<Quality>),
    Travel(motion::Request, Travel, Option<Lease>, Reply<()>, Reply<()>),
    Run(Box<Program>, Reply<Ending>, Reply<()>),
    SwitchMode(Reply<()>),
    Relieve(Option<u32>, Reply<()>),
    Outputs(Plan, Option<Lease>, Reply<()>, Reply<()>),
    ReadParameters(Reply<Verified>),
    ApplyParameters(Vec<Bank>, Reply<Verified>),
    Initialize(Box<parameters::Initialization>, Reply<Verified>),
    Disconnect(Reply<()>),
    Release(Option<Lease>, Reply<()>),
    Hold(Reply<()>),
    Stop(Reply<()>),
    Shutdown(Reply<()>),
}

impl Command {
    fn enabling(&self) -> bool {
        matches!(
            self,
            Command::Home { .. }
                | Command::Calibrate(..)
                | Command::Travel(..)
                | Command::Run(..)
                | Command::SwitchMode(..)
                | Command::Relieve(..)
                | Command::Outputs(..)
                | Command::ApplyParameters(..)
                | Command::Initialize(..)
        )
    }

    const fn urgent(&self) -> bool {
        matches!(
            self,
            Self::Disconnect(_)
                | Self::Release(..)
                | Self::Hold(_)
                | Self::Stop(_)
                | Self::Shutdown(_)
        )
    }

    fn refuse(self, error: Error) {
        match self {
            Self::Connect(_, reply) => drop(reply.send(Err(error))),
            Self::Configure(_, reply)
            | Self::Home { reply, .. }
            | Self::SwitchMode(reply)
            | Self::Relieve(_, reply)
            | Self::Disconnect(reply)
            | Self::Release(_, reply)
            | Self::Hold(reply)
            | Self::Stop(reply)
            | Self::Shutdown(reply) => drop(reply.send(Err(error))),
            Self::Calibrate(reply) => drop(reply.send(Err(error))),
            Self::Travel(_, _, _, reply, admitted) | Self::Outputs(_, _, reply, admitted) => {
                drop(admitted.send(Err(error.clone())));
                drop(reply.send(Err(error)));
            }
            Self::Run(_, reply, admitted) => {
                drop(admitted.send(Err(error.clone())));
                drop(reply.send(Err(error)));
            }
            Self::ReadParameters(reply)
            | Self::ApplyParameters(_, reply)
            | Self::Initialize(_, reply) => {
                drop(reply.send(Err(error)));
            }
        }
    }
}

/// The handle. Clones share one task; the task ends when the last clone is
/// dropped, stopping whatever was running and closing the socket.
#[derive(Clone)]
pub struct Machine {
    commands: mpsc::Sender<(Instant, Command)>,
    controls: mpsc::UnboundedSender<(Instant, Command)>,
    heartbeat: watch::Sender<(Option<Lease>, Instant)>,
    state: watch::Receiver<State>,
}

/// An admitted run; waiting for it does not hold the controller command queue.
pub struct Completion<T>(oneshot::Receiver<Result<T>>);

impl<T> Completion<T> {
    /// Waits for completion, a settled hold, stop, or failure.
    pub async fn finished(self) -> Result<T> {
        self.0.await.map_err(|_| Error::Gone)?
    }
}

impl Machine {
    /// Spawns the task on the current runtime.
    #[must_use]
    pub fn spawn(config: Config) -> Self {
        Self::spawn_with_alarm_events(config).0
    }

    /// Spawns with an ordered transition stream for a separate history worker.
    /// Sending never waits for disk or a user interface.
    #[must_use]
    pub fn spawn_with_alarm_events(
        config: Config,
    ) -> (Self, mpsc::UnboundedReceiver<crate::alarms::Observation>) {
        let (commands, receiver) = mpsc::channel(COMMAND_QUEUE);
        let (controls, urgent) = mpsc::unbounded_channel();
        let (heartbeat, renewals) = watch::channel((None, Instant::now()));
        let (publisher, state) = watch::channel(State::default());
        let (alarms, observations) = mpsc::unbounded_channel();
        tokio::spawn(Task::new(config, publisher, alarms).run(receiver, urgent, renewals));
        (Self { commands, controls, heartbeat, state }, observations)
    }

    /// The state as last published.
    #[must_use]
    pub fn state(&self) -> State {
        self.state.borrow().clone()
    }

    /// A receiver that wakes on every publication.
    #[must_use]
    pub fn watch(&self) -> watch::Receiver<State> {
        self.state.clone()
    }

    /// Opens the socket, checks the controller's identity and takes the
    /// first snapshot.
    pub async fn connect(&self) -> Result<Identity> {
        self.ask(|reply| Command::Connect(None, reply)).await
    }

    /// Connects to `endpoint` from `host` instead of the configured route.
    pub async fn connect_to(&self, endpoint: SocketAddrV4, host: Ipv4Addr) -> Result<Identity> {
        self.ask(|reply| Command::Connect(Some((endpoint, host)), reply)).await
    }

    /// Runs bounded cleanup and ends the socket task, even while handles remain.
    pub async fn shutdown(&self) -> Result<()> {
        self.ask(Command::Shutdown).await
    }

    /// Stops whatever runs and closes the socket.
    pub async fn disconnect(&self) -> Result<()> {
        self.ask(Command::Disconnect).await
    }

    /// Installs the machine bindings; allowed whenever nothing is running.
    pub async fn configure(&self, bindings: Bindings) -> Result<()> {
        self.ask(|reply| Command::Configure(Box::new(bindings), reply)).await
    }

    /// Go Origin, or the head-only search that relieves the head reference
    /// alarms. Answers when the reference is established.
    pub async fn home(&self, head_only: bool) -> Result<()> {
        self.home_at(head_only, Instant::now()).await
    }

    /// Preserves the Home press time while interrupted startup is completed,
    /// so a later Stop also cancels a reference search still being prepared.
    pub async fn home_at(&self, head_only: bool, requested: Instant) -> Result<()> {
        self.ask_at(requested, |reply| Command::Home { head_only, reply }).await
    }

    /// Calibrates the head; answers with the quality it reports.
    pub async fn calibrate(&self) -> Result<Quality> {
        self.ask(Command::Calibrate).await
    }

    /// A jog or a positioning move; answers when the axes are stationary at
    /// the target, or after a held jog is released.
    pub async fn travel(&self, request: motion::Request) -> Result<()> {
        self.start_travel(request, None, Instant::now()).await?.finished().await
    }

    /// Ends a held jog or switches a manual output off.
    pub async fn release(&self) -> Result<()> {
        self.release_owned(None).await
    }

    /// Renews the lease of a held jog or a manual output.
    pub fn heartbeat(&self) -> Result<()> {
        self.heartbeat.send((None, Instant::now())).map_err(|_| Error::Gone)
    }

    /// Runs a program; answers with how it ended.
    pub async fn run(&self, program: Program) -> Result<Ending> {
        self.start_run(program).await?.finished().await
    }

    /// Answers only after the controller task admits this exact program.
    pub async fn start_run(&self, program: Program) -> Result<Completion<Ending>> {
        self.start_run_at(program, Instant::now()).await
    }

    /// Uses the host request time so a stop also cancels work built off-task.
    pub async fn start_run_at(
        &self,
        program: Program,
        requested: Instant,
    ) -> Result<Completion<Ending>> {
        let (reply, completion) = oneshot::channel();
        self.ask_at(requested, |admitted| Command::Run(Box::new(program), reply, admitted)).await?;
        Ok(Completion(completion))
    }

    /// Pauses the running program, keeping its checkpoint for a
    /// continuation.
    pub async fn hold(&self) -> Result<()> {
        self.ask(Command::Hold).await
    }

    /// The stop sequence: always available while connected.
    pub async fn stop(&self) -> Result<()> {
        self.ask(Command::Stop).await
    }

    /// Applies the configured mode switch; the reference must then be
    /// established again.
    pub async fn switch_mode(&self) -> Result<()> {
        self.switch_mode_at(Instant::now()).await
    }

    /// Preserves the request time while the host constructs new bindings.
    pub async fn switch_mode_at(&self, requested: Instant) -> Result<()> {
        self.ask_at(requested, Command::SwitchMode).await
    }

    /// Relieves one alarm row by the vendor's id, or a row without an id.
    pub async fn relieve(&self, id: Option<u32>) -> Result<()> {
        self.ask(|reply| Command::Relieve(id, reply)).await
    }

    /// Holds a manual output on until released or its lease lapses.
    pub async fn outputs(&self, plan: Plan) -> Result<()> {
        self.start_outputs(plan, None, Instant::now()).await?.finished().await
    }

    /// Reads the parameter banks back.
    pub async fn read_parameters(&self) -> Result<Verified> {
        self.ask(Command::ReadParameters).await
    }

    /// Writes the banks that differ, activates them and reads them back.
    pub async fn apply_parameters(&self, banks: Vec<Bank>) -> Result<Verified> {
        self.ask(|reply| Command::ApplyParameters(banks, reply)).await
    }

    /// Applies an ordered machine initialization and verifies all its registers.
    pub async fn initialize(&self, plan: parameters::Initialization) -> Result<Verified> {
        self.ask(|reply| Command::Initialize(Box::new(plan), reply)).await
    }

    /// Admits a move owned by a press, or a discrete move with no lease.
    pub async fn start_travel(
        &self,
        request: motion::Request,
        lease: Option<Lease>,
        requested: Instant,
    ) -> Result<Completion<()>> {
        self.start_move(request, Travel::Jog, lease, requested).await
    }

    /// Positions XY, first returning a lowered head to its upper reference.
    /// Both stages belong to one operation and share cancellation and alarms.
    pub async fn start_positioning(
        &self,
        request: motion::Request,
        requested: Instant,
    ) -> Result<Completion<()>> {
        self.start_move(request, Travel::Position, None, requested).await
    }

    /// Admits a configured table move under the same lease and stop lifecycle.
    pub async fn start_table(
        &self,
        request: motion::Request,
        limits: [i32; 2],
        lease: Option<Lease>,
        requested: Instant,
    ) -> Result<Completion<()>> {
        self.start_move(request, Travel::Table(limits), lease, requested).await
    }

    async fn start_move(
        &self,
        request: motion::Request,
        travel: Travel,
        lease: Option<Lease>,
        requested: Instant,
    ) -> Result<Completion<()>> {
        let (reply, completion) = oneshot::channel();
        self.ask_at(requested, |admitted| Command::Travel(request, travel, lease, reply, admitted))
            .await?;
        Ok(Completion(completion))
    }

    /// Admits an output held by this press.
    pub async fn start_outputs(
        &self,
        plan: Plan,
        lease: Option<Lease>,
        requested: Instant,
    ) -> Result<Completion<()>> {
        let (reply, completion) = oneshot::channel();
        self.ask_at(requested, |admitted| Command::Outputs(plan, lease, reply, admitted)).await?;
        Ok(Completion(completion))
    }

    /// A terminal release affects only the matching press.
    pub async fn release_owned(&self, lease: Option<Lease>) -> Result<()> {
        self.ask(|reply| Command::Release(lease, reply)).await
    }

    /// Only the matching active press may renew its lease.
    pub fn heartbeat_owned(&self, lease: Lease) -> Result<()> {
        self.heartbeat.send((Some(lease), Instant::now())).map_err(|_| Error::Gone)
    }

    async fn ask<T>(&self, command: impl FnOnce(Reply<T>) -> Command) -> Result<T> {
        self.ask_at(Instant::now(), command).await
    }

    async fn ask_at<T>(
        &self,
        requested: Instant,
        command: impl FnOnce(Reply<T>) -> Command,
    ) -> Result<T> {
        let (reply, answer) = oneshot::channel();
        let command = command(reply);
        let urgent = command.urgent();
        let envelope = (requested, command);
        if urgent {
            self.controls.send(envelope).map_err(|_| Error::Gone)?;
        } else {
            self.commands.send(envelope).await.map_err(|_| Error::Gone)?;
        }
        answer.await.map_err(|_| Error::Gone)?
    }
}

/// The active operation with the requester waiting on it.
enum Job {
    Home(Home, Reply<()>),
    Calibrate(Calibrate, Reply<Quality>),
    Motion(Box<Motion>, Reply<()>),
    Mode(Switch, Reply<()>),
    Relief(Relief, Option<u32>, Reply<()>),
    Outputs(Outputs, Reply<()>),
    Run(Box<Run>, Reply<Ending>),
    Parameters(Box<parameters::Parameters>, Option<u64>, Reply<Verified>),
}

impl Job {
    fn operation(&mut self) -> &mut dyn Operation {
        match self {
            Self::Home(op, _) => op,
            Self::Calibrate(op, _) => op,
            Self::Motion(op, _) => op.as_mut(),
            Self::Mode(op, _) => op,
            Self::Relief(op, _, _) => op,
            Self::Outputs(op, _) => op,
            Self::Run(op, _) => op.as_mut(),
            Self::Parameters(op, ..) => op.as_mut(),
        }
    }

    fn view(&self) -> &dyn Operation {
        match self {
            Self::Home(op, _) => op,
            Self::Calibrate(op, _) => op,
            Self::Motion(op, _) => op.as_ref(),
            Self::Mode(op, _) => op,
            Self::Relief(op, _, _) => op,
            Self::Outputs(op, _) => op,
            Self::Run(op, _) => op.as_ref(),
            Self::Parameters(op, ..) => op.as_ref(),
        }
    }

    /// What this job may see past when it asks whether it is blocked.
    fn concession(&self, snapshot: &Snapshot) -> Concession {
        match self {
            Self::Run(run, _) if run.preparing_head() => {
                Concession::HeadHome { other_head_faults: home::other_head_faults(snapshot) }
            }
            Self::Home(..) | Self::Parameters(..) | Self::Mode(..) => {
                Concession::ManualPositioning {
                    other_head_faults: home::other_head_faults(snapshot),
                }
            }
            Self::Motion(motion, _) => motion.concession(snapshot),
            Self::Calibrate(..) => Concession::ProcessOnly,
            Self::Outputs(outputs, _) => outputs
                .head_jog()
                .map_or(Concession::None, |up| outputs::head_concession(snapshot, up)),
            _ => Concession::None,
        }
    }

    /// Answers the requester with a failure.
    fn fail(self, error: Error) {
        match self {
            Self::Home(_, reply)
            | Self::Motion(_, reply)
            | Self::Mode(_, reply)
            | Self::Relief(_, _, reply)
            | Self::Outputs(_, reply) => drop(reply.send(Err(error))),
            Self::Calibrate(_, reply) => drop(reply.send(Err(error))),
            Self::Run(_, reply) => drop(reply.send(Err(error))),
            Self::Parameters(_, _, reply) => drop(reply.send(Err(error))),
        }
    }
}

struct Active {
    job: Job,
    started: Instant,
    /// Whether any write went out for this job; a failure before that is a
    /// refusal and needs no stop sequence.
    sent: bool,
    pending: VecDeque<Write>,
    finishing: bool,
    written_at: Instant,
    lease: Option<Lease>,
}

struct Task {
    config: Config,
    /// Writes the controller refused during the current operation, told to
    /// the operator when it ends.
    refused: Option<String>,
    publisher: watch::Sender<State>,
    alarm_events: mpsc::UnboundedSender<crate::alarms::Observation>,
    previous_alarms: Vec<crate::state::AlarmView>,
    previous_fault: Option<String>,
    alarm_revision: u64,
    revision: u64,
    connection: Connection,
    link: Option<Link>,
    details: DetailCache,
    bindings: Option<Bindings>,
    binding_revision: u64,
    closing: bool,
    monitor: Monitor,
    leases: Leases,
    session: Session,
    identity: Option<Identity>,
    snapshot: Option<Snapshot>,
    capture: Option<Capture>,
    missed_polls: u8,
    read_next: bool,
    /// Set when a head-only read went unanswered, until the next full
    /// poll, so a slow controller never queues more than one of them.
    head_watch_paused: bool,
    stopped_at: Option<Instant>,
    released_at: Option<Instant>,
    active: Option<Active>,
    program: Option<ProgramView>,
    last_error: Option<String>,
}

impl Task {
    fn new(
        config: Config,
        publisher: watch::Sender<State>,
        alarm_events: mpsc::UnboundedSender<crate::alarms::Observation>,
    ) -> Self {
        Self {
            config,
            publisher,
            alarm_events,
            previous_alarms: Vec::new(),
            previous_fault: None,
            alarm_revision: 0,
            revision: 0,
            connection: Connection::Disconnected,
            link: None,
            details: DetailCache::default(),
            bindings: None,
            binding_revision: 0,
            closing: false,
            monitor: Monitor::default(),
            leases: Leases::default(),
            session: Session::default(),
            identity: None,
            snapshot: None,
            capture: None,
            missed_polls: 0,
            read_next: false,
            head_watch_paused: false,
            stopped_at: None,
            released_at: None,
            active: None,
            program: None,
            last_error: None,
            refused: None,
        }
    }

    async fn run(
        mut self,
        mut commands: mpsc::Receiver<(Instant, Command)>,
        mut controls: mpsc::UnboundedReceiver<(Instant, Command)>,
        mut heartbeat: watch::Receiver<(Option<Lease>, Instant)>,
    ) {
        let mut poll = tokio::time::interval(self.config.poll_interval);
        poll.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            if self.closing {
                break;
            }
            let (owner, renewed) = heartbeat.borrow_and_update().clone();
            if self.active.as_ref().is_some_and(|a| a.lease == owner) {
                self.heartbeat(renewed);
            }
            if self.lease_expired() {
                if let Err(error) = self.release().await {
                    self.last_error = Some(error.to_string());
                }
                self.publish();
            }
            // Control requests already waiting always win the next exchange.
            if let Ok((requested, command)) = controls.try_recv() {
                self.handle(requested, command).await;
                continue;
            }
            let work = self.link.is_some()
                && (self.capture.is_some()
                    || self.watching_head()
                    || self.active.as_ref().is_some_and(|active| !active.pending.is_empty()));
            tokio::select! {
                command = commands.recv() => match command {
                    Some((requested, command)) => self.handle(requested, command).await,
                    None => break,
                },
                control = controls.recv() => match control {
                    Some((requested, command)) => self.handle(requested, command).await,
                    None => break,
                },
                changed = heartbeat.changed() => if changed.is_err() { break; },
                _ = poll.tick(), if self.link.is_some() && self.capture.is_none() => {
                    self.capture = Some(Capture::new());
                },
                () = std::future::ready(()), if work => self.exchange().await,
            }
        }
        // Every handle is gone: leave the machine stopped and the socket
        // closed.
        if self.link.is_some() {
            if let Err(error) = self.stop().await {
                tracing::warn!(%error, "stop on shutdown");
            }
            if let Err(error) = self.disconnect().await {
                tracing::warn!(%error, "disconnect on shutdown");
            }
        }
    }

    fn command_refusal(&self, requested: Instant, command: &Command) -> Option<&'static str> {
        let held = matches!(&command, Command::Travel(request, _, None, _, _) if request.held)
            || matches!(&command, Command::Outputs(_, None, _, _));
        let enabling = command.enabling();
        if enabling && self.missed_polls > 0 {
            return Some("waiting for fresh controller feedback");
        }
        if (enabling && self.stopped_at.is_some_and(|stopped| requested <= stopped))
            || (held && self.released_at.is_some_and(|released| requested <= released))
        {
            return Some("the queued operation was cancelled");
        }
        None
    }

    async fn handle(&mut self, requested: Instant, command: Command) {
        if let Some(reason) = self.command_refusal(requested, &command) {
            command.refuse(Error::Refused(reason.into()));
            return;
        }
        match command {
            Command::Connect(route, reply) => {
                if let Some((endpoint, host)) = route {
                    self.config.endpoint = endpoint;
                    self.config.host = host;
                }
                let outcome = self.connect().await;
                self.answer(reply, outcome);
            }
            Command::Configure(bindings, reply) => {
                let outcome = self.configure(*bindings);
                self.answer(reply, outcome);
            }
            Command::Home { head_only, reply } => {
                self.launch(self.prepare_home(head_only), reply, Job::Home);
            }
            Command::Calibrate(reply) => {
                let prepared = self.prepare_calibrate();
                if prepared.is_ok() {
                    self.session.calibration = None;
                }
                self.launch(prepared, reply, Job::Calibrate);
            }
            Command::Travel(request, table, lease, reply, admitted) => {
                let prepared = self
                    .own_press(lease.as_ref())
                    .and_then(|()| self.prepare_travel(request, table, requested));
                self.launch_owned(prepared, reply, admitted, lease, |motion, reply| {
                    Job::Motion(Box::new(motion), reply)
                });
            }
            Command::Run(program, reply, admitted) => {
                let prepared = self.prepare_run(*program);
                let outcome = prepared.as_ref().map(|_| ()).map_err(Clone::clone);
                self.launch(prepared, reply, |run, reply| Job::Run(Box::new(run), reply));
                drop(admitted.send(outcome));
            }
            Command::SwitchMode(reply) => {
                self.launch(self.prepare_switch(), reply, Job::Mode);
            }
            Command::Relieve(id, reply) => {
                if id.is_some_and(|id| catalogue::relief(id) == catalogue::Relief::HeadHome) {
                    self.launch(self.prepare_home(true), reply, Job::Home);
                } else {
                    let job = move |relief, reply| Job::Relief(relief, id, reply);
                    self.launch(self.prepare_relief(id), reply, job);
                }
            }
            Command::Outputs(plan, lease, reply, admitted) => {
                let prepared = self
                    .own_press(lease.as_ref())
                    .and_then(|()| self.prepare_outputs(plan, requested));
                self.launch_owned(prepared, reply, admitted, lease, Job::Outputs);
            }
            Command::ReadParameters(reply) => {
                self.parameters(None, reply);
            }
            Command::ApplyParameters(banks, reply) => {
                self.parameters(Some(banks), reply);
            }
            Command::Initialize(plan, reply) => self.initialize(*plan, reply),
            Command::Shutdown(reply) => {
                self.closing = true;
                self.stopped_at = Some(Instant::now());
                let outcome = self.disconnect().await;
                self.answer(reply, outcome);
            }
            Command::Disconnect(reply) => {
                self.stopped_at = Some(Instant::now());
                let outcome = self.disconnect().await;
                self.answer(reply, outcome);
            }
            Command::Stop(reply) => {
                self.stopped_at = Some(Instant::now());
                let outcome = self.stop().await;
                self.answer(reply, outcome);
            }
            Command::Hold(reply) => {
                let outcome = self.hold().await;
                self.answer(reply, outcome);
            }
            Command::Release(lease, reply) => {
                let outcome = self.release_press(lease).await;
                self.answer(reply, outcome);
            }
        }
    }

    /// Answers an immediate command and records its failure, if any.
    fn answer<T>(&mut self, reply: Reply<T>, outcome: Result<T>) {
        if let Err(error) = &outcome {
            self.last_error = Some(error.to_string());
        }
        // A caller on another runtime worker may inspect state immediately
        // after its reply. Publish first so it cannot observe the old session.
        self.publish();
        drop(reply.send(outcome));
    }

    /// Makes a prepared operation active, or answers with why it could not
    /// be prepared.
    fn launch<T, O>(
        &mut self,
        prepared: Result<O>,
        reply: Reply<T>,
        job: impl FnOnce(O, Reply<T>) -> Job,
    ) {
        match prepared {
            Ok(operation) => {
                let job = job(operation, reply);
                let started = Instant::now();
                self.active = Some(Active {
                    job,
                    started,
                    sent: false,
                    pending: VecDeque::new(),
                    finishing: false,
                    written_at: started,
                    lease: None,
                });
                self.capture = Some(Capture::new());
                self.publish();
            }
            Err(error) => self.answer(reply, Err(error)),
        }
    }

    fn own_press(&mut self, lease: Option<&Lease>) -> Result<()> {
        if let Some(lease) = lease {
            self.leases.start(lease, Instant::now())?;
        }
        Ok(())
    }

    async fn release_press(&mut self, lease: Option<Lease>) -> Result<()> {
        if let Some(lease) = &lease {
            self.leases.close(lease, Instant::now())?;
        } else {
            self.released_at = Some(Instant::now());
        }
        if self.active.as_ref().is_some_and(|a| a.lease == lease) {
            self.release().await?;
        }
        Ok(())
    }

    fn launch_owned<T, O>(
        &mut self,
        prepared: Result<O>,
        reply: Reply<T>,
        admitted: Reply<()>,
        lease: Option<Lease>,
        job: impl FnOnce(O, Reply<T>) -> Job,
    ) {
        let outcome = prepared.as_ref().map(|_| ()).map_err(Clone::clone);
        self.launch(prepared, reply, job);
        if outcome.is_ok()
            && let Some(active) = &mut self.active
        {
            active.lease = lease;
        }
        drop(admitted.send(outcome));
    }

    // Preconditions -----------------------------------------------------------

    fn idle(&self) -> Result<()> {
        match &self.active {
            Some(active) => Err(Error::Busy(active.job.view().kind().label())),
            None => Ok(()),
        }
    }

    fn connected(&self) -> Result<&Snapshot> {
        self.snapshot.as_ref().ok_or(Error::Disconnected)
    }

    /// Whether the latest feedback reports both X and Y referenced. An
    /// operation completes on this snapshot after it was validated, so a
    /// reference carried across a completion is checked here, not a poll
    /// later.
    fn xy_referenced(&self) -> bool {
        self.snapshot.as_ref().is_some_and(|snapshot| snapshot.axes.xy_referenced())
    }

    fn configured(&self) -> Result<&Bindings> {
        self.bindings.as_ref().ok_or(Error::Unconfigured)
    }

    /// Idle, connected and configured: what every operation needs.
    fn ready(&self) -> Result<(&Snapshot, &Bindings)> {
        self.idle()?;
        Ok((self.connected()?, self.configured()?))
    }

    fn homed_with_parameters(&self) -> Result<&Verified> {
        if !self.session.is_homed() {
            return Err(Error::Refused("run Go Origin first".into()));
        }
        self.verified_parameters()
    }

    fn verified_parameters(&self) -> Result<&Verified> {
        self.session
            .parameters
            .as_ref()
            .ok_or_else(|| Error::Refused("read the parameters first".into()))
    }

    fn prepare_home(&self, head_only: bool) -> Result<Home> {
        let (_, bindings) = self.ready()?;
        if head_only && !bindings.head_enabled {
            return Err(Error::Refused("no head controller is configured".into()));
        }
        Ok(Home::new(bindings, head_only))
    }

    fn prepare_calibrate(&self) -> Result<Calibrate> {
        let (_, bindings) = self.ready()?;
        if !bindings.head_enabled {
            return Err(Error::Refused("no head controller is configured".into()));
        }
        Ok(Calibrate::new())
    }

    fn prepare_travel(
        &self,
        request: motion::Request,
        travel: Travel,
        requested: Instant,
    ) -> Result<Motion> {
        let (snapshot, bindings) = self.ready()?;
        // An initial limit can prevent settings initialization. A recovery
        // pulse uses the measured axis units without accepting them for jobs.
        let verified = if matches!(travel, Travel::Jog) && snapshot.xy_limit_alarms() {
            snapshot.parameters().map_err(|error| Error::Refused(error.to_string()))?
        } else {
            *self.verified_parameters()?
        };
        let motion = match travel {
            Travel::Table(limits) => {
                Motion::table(request, &verified, bindings.head_enabled, limits, requested)
            }
            Travel::Jog => Motion::new(request, &verified, bindings.head_enabled, requested),
            Travel::Position => Motion::positioning(request, &verified, bindings, requested),
        }
        .map_err(Error::Refused)?;
        if let Some((axis, positive)) = motion.jog_direction() {
            let blocked = self.monitor.blocked_for(motion.concession(snapshot));
            admit_alarms(snapshot.xy_jog_alarms_clear(axis, positive, false), blocked.as_deref())
                .map_err(Error::Refused)?;
        }
        Ok(motion)
    }

    fn prepare_run(&self, program: Program) -> Result<Run> {
        let (_, bindings) = self.ready()?;
        self.homed_with_parameters()?;
        if self.configuration() != Some(program.configuration) {
            return Err(Error::Refused(
                "the program's machine configuration changed; compile again".into(),
            ));
        }
        if program.mode != bindings.mode {
            return Err(Error::Refused("the program was compiled for another laser".into()));
        }
        if let Some(blocked) = self.monitor.blocked() {
            return Err(Error::Refused(format!("alarms are active: {blocked}")));
        }
        let capacity = self.connected()?.capacity;
        Run::new(program, capacity, bindings).map_err(Error::Refused)
    }

    fn prepare_switch(&self) -> Result<Switch> {
        let (_, bindings) = self.ready()?;
        Switch::new(&bindings.mode_switch).map_err(Error::Refused)
    }

    fn prepare_relief(&self, id: Option<u32>) -> Result<Relief> {
        let (snapshot, bindings) = self.ready()?;
        let status = &snapshot.status;
        let plan = sequences::relief(
            id,
            status.alarm_group_1(),
            status.alarm_group_2(),
            bindings.dual_drive,
        )
        .map_err(|error| Error::Refused(error.to_string()))?;
        Ok(Relief::new(plan))
    }

    fn prepare_outputs(&self, plan: Plan, requested: Instant) -> Result<Outputs> {
        let (snapshot, bindings) = self.ready()?;
        let outputs = Outputs::new(plan, requested).rising_on_touch(bindings.shutdown.raise);
        if let Some(up) = outputs.head_jog() {
            if !bindings.head_enabled {
                return Err(Error::Refused("no head controller is configured".into()));
            }
            let blocked = self.monitor.blocked_for(outputs::head_concession(snapshot, up));
            // Reject an alarmed direction immediately. Mechanical admission
            // uses the next poll, after any preceding operation's cleanup.
            admit_alarms(snapshot.head_jog_alarms_clear(up), blocked.as_deref())
                .map_err(Error::Refused)?;
        }
        Ok(outputs)
    }

    // Immediate commands --------------------------------------------------------

    async fn connect(&mut self) -> Result<Identity> {
        if self.link.is_some() {
            return Err(Error::Refused("already connected".into()));
        }
        let mut link = Link::open(&self.config).await?;
        let words = link.read(Read::block(registers::IDENTITY)).await.map_err(Error::from)?;
        let identity =
            feedback::Identity::decode(&words).map_err(|error| Error::Link(error.to_string()))?;
        if identity.product_id != MCC100 {
            return Err(Error::Refused(format!(
                "the controller reports product {}, not the MCC100",
                identity.product_id
            )));
        }
        let snapshot =
            Snapshot::capture(&mut link, &mut self.details).await.map_err(Error::from)?;
        snapshot.divisor().map_err(|error| Error::Refused(error.to_string()))?;
        snapshot
            .system
            .interpolation_cycle_us()
            .map_err(|error| Error::Refused(error.to_string()))?;
        self.session.connected();
        self.missed_polls = 0;
        let identity = Identity::from(identity);
        self.identity = Some(identity);
        self.snapshot = Some(snapshot);
        self.monitor.operation = 0;
        self.monitor.observe(&snapshot, &self.details);
        self.connection =
            Connection::Connected { epoch: self.session.epoch, endpoint: link.peer().to_string() };
        self.link = Some(link);
        self.program = None;
        Ok(identity)
    }

    async fn disconnect(&mut self) -> Result<()> {
        let active = self.active.take();
        let mut writes = active.as_ref().map_or_else(Vec::new, |a| a.job.view().cancel());
        writes.extend(self.abort_writes(active.is_some()));
        let outcome = if self.link.is_some() { self.cleanup(&writes).await } else { Ok(()) };
        if let Some(active) = active {
            self.settle(active, Error::Failed("disconnected".into()));
        }
        self.invalidate("disconnected");
        self.link = None;
        self.snapshot = None;
        self.capture = None;
        self.connection = Connection::Disconnected;
        outcome
    }

    fn configure(&mut self, bindings: Bindings) -> Result<()> {
        self.idle()?;
        let mut ids = std::collections::BTreeSet::new();
        if bindings.rules.iter().any(|rule| {
            !ids.insert(rule.id)
                || !(1..=24).contains(&rule.input)
                || rule.gas_valve.is_some_and(|port| {
                    openlaser_protocol::requests::OutputPort::new(port).is_err()
                })
        }) {
            return Err(Error::Refused("alarm rules have duplicate ids or invalid ports".into()));
        }
        sequences::abort(&bindings.shutdown, true)
            .map_err(|error| Error::Refused(format!("the shutdown policy is invalid: {error}")))?;
        self.monitor.configure(bindings.rules.clone(), bindings.head_enabled);
        self.binding_revision += 1;
        self.session.rebind();
        self.bindings = Some(bindings);
        Ok(())
    }

    async fn stop(&mut self) -> Result<()> {
        self.connected()?;
        let Some(active) = self.active.take() else {
            let writes = self.abort_writes(false);
            let outcome = self.cleanup(&writes).await;
            self.stop_held(&outcome);
            return outcome;
        };
        if matches!(&active.job, Job::Run(..)) {
            return self.end_run(active, Ending::Stopped, true).await;
        }
        let mut writes = active.job.view().cancel();
        writes.extend(self.abort_writes(active.sent));
        let outcome = self.cleanup(&writes).await;
        self.settle(active, Error::Failed("stopped".into()));
        self.stop_held(&outcome);
        outcome
    }

    fn stop_held(&mut self, outcome: &Result<()>) {
        if let Some(program) = &mut self.program
            && program.state == ProgramState::Held
        {
            program.state =
                if outcome.is_ok() { ProgramState::Stopped } else { ProgramState::Failed };
            program.error = outcome.as_ref().err().map(ToString::to_string);
        }
    }

    async fn hold(&mut self) -> Result<()> {
        self.connected()?;
        match self.active.take() {
            Some(active) if matches!(&active.job, Job::Run(run, _) if run.active()) => {
                self.end_run(active, Ending::Held, true).await
            }
            Some(active) if matches!(&active.job, Job::Run(run, _) if run.state() == ProgramState::Held) =>
            {
                self.active = Some(active);
                Ok(())
            }
            None if self.program.as_ref().is_some_and(|p| p.state == ProgramState::Held) => Ok(()),
            other => {
                self.active = other;
                Err(Error::Refused("no program is running".into()))
            }
        }
    }

    /// Begins a hold or a stop of the running program; the run then settles
    /// over the next ticks and answers its requester with the ending.
    async fn end_run(&mut self, mut active: Active, ending: Ending, raise: bool) -> Result<()> {
        let Job::Run(run, _) = &mut active.job else {
            self.active = Some(active);
            return Err(Error::Refused("no program is running".into()));
        };
        let Some(snapshot) = self.snapshot else {
            self.active = Some(active);
            return Err(Error::Disconnected);
        };
        match run.end(ending, &snapshot, raise) {
            Ok(writes) => match self.cleanup(&writes).await {
                Ok(()) => {
                    active.pending.clear();
                    active.finishing = false;
                    active.written_at = Instant::now();
                    self.active = Some(active);
                    Ok(())
                }
                Err(error) => {
                    self.fail(active, error.clone()).await;
                    Err(error)
                }
            },
            Err(reason) => {
                self.active = Some(active);
                Err(Error::Refused(reason))
            }
        }
    }

    async fn release(&mut self) -> Result<()> {
        self.connected()?;
        let Some(mut active) = self.active.take() else {
            return Ok(());
        };
        let writes = match &mut active.job {
            Job::Motion(motion, _) if motion.held() => motion.release(Instant::now()),
            Job::Outputs(outputs, _) => outputs.release(),
            _ => {
                self.active = Some(active);
                return Err(Error::Refused("nothing is held".into()));
            }
        };
        active.pending.clear();
        active.finishing = false;
        match self.cleanup(&writes).await {
            Ok(()) => {
                active.sent |= !writes.is_empty();
                active.written_at = Instant::now();
                self.active = Some(active);
                Ok(())
            }
            Err(error) => {
                self.fail(active, error.clone()).await;
                Err(error)
            }
        }
    }

    fn heartbeat(&mut self, now: Instant) {
        if self.active.as_ref().is_none_or(|active| now < active.started) {
            return;
        }
        match self.active.as_mut().map(|active| &mut active.job) {
            Some(Job::Motion(motion, _)) => motion.heartbeat(now),
            Some(Job::Outputs(outputs, _)) => outputs.heartbeat(now),
            _ => {}
        }
    }

    fn lease_expired(&self) -> bool {
        let now = Instant::now();
        match self.active.as_ref().map(|active| &active.job) {
            Some(Job::Motion(motion, _)) => motion.expired(now),
            Some(Job::Outputs(outputs, _)) => outputs.expired(now),
            _ => false,
        }
    }

    fn initialize(&mut self, plan: parameters::Initialization, reply: Reply<Verified>) {
        let ready = self.idle().and_then(|()| self.connected().map(|_| ()));
        if let Err(error) = ready {
            return self.answer(reply, Err(error));
        }
        if self.session.parameters != Some(plan.before) {
            return self.answer(
                reply,
                Err(Error::Refused("read the parameters before initialization".into())),
            );
        }
        // The reference is set aside, like a read's, and returns only if
        // the written configuration keeps it.
        let homed = self.session.homed.take();
        self.session.rebind();
        self.launch(Ok(parameters::Parameters::initialize(plan)), reply, |operation, reply| {
            Job::Parameters(Box::new(operation), homed, reply)
        });
    }

    fn parameters(&mut self, banks: Option<Vec<Bank>>, reply: Reply<Verified>) {
        let ready = self.idle().and_then(|()| self.connected().map(|_| ()));
        if let Err(error) = ready {
            return self.answer(reply, Err(error));
        }
        if banks.is_some() && self.session.parameters.is_none() {
            return self.answer(reply, Err(Error::Refused("read the parameters first".into())));
        }
        // A failed read or partial apply must never leave the old acceptance.
        let before = self.session.parameters.take();
        let homed = self.session.homed.take();
        self.launch(
            Ok(parameters::Parameters::new(before.as_ref(), banks)),
            reply,
            |operation, reply| Job::Parameters(Box::new(operation), homed, reply),
        );
    }

    // One exchange. The run loop services urgent controls and leases before
    // returning here; no write is cancelled halfway through an exchange.

    async fn exchange(&mut self) {
        let pending = self.active.as_ref().is_some_and(|active| !active.pending.is_empty());
        let watching = self.watching_head();
        if pending
            && self.missed_polls == 0
            && (!self.read_next || (self.capture.is_none() && !watching))
        {
            self.read_next = true;
            self.write_one().await;
        } else if self.capture.is_some() {
            self.read_next = false;
            self.read_one().await;
        } else if watching {
            self.read_next = false;
            self.watch_head().await;
            return;
        }
        self.publish();
    }

    /// Whether a program is cutting or framing with a head configured. The
    /// head alone is then read between full feedback polls, so a nozzle
    /// touching the plate pauses the program within a read or two rather
    /// than a poll.
    fn watching_head(&self) -> bool {
        self.missed_polls == 0
            && !self.head_watch_paused
            && self.bindings.as_ref().is_some_and(|bindings| bindings.head_enabled)
            && self.active.as_ref().is_some_and(
                |active| matches!(&active.job, Job::Run(run, _) if run.active() && !run.preparing_head()),
            )
    }

    /// One head-only read. Any head fault, including a lost reference,
    /// holds the program at once; the full poll that follows records the
    /// alarm. A read without a reply is only skipped, and head reads wait
    /// for the next full poll: losing feedback is the full poll's to judge.
    async fn watch_head(&mut self) {
        let Some(link) = self.link.as_mut() else { return };
        let read = link.read(Read::block(registers::HEAD));
        let head = match tokio::time::timeout(HEAD_WATCH_TIMEOUT, read).await {
            Ok(Ok(words)) => feedback::Head::decode(&words).ok(),
            _ => None,
        };
        let Some(head) = head else {
            self.head_watch_paused = true;
            return;
        };
        if catalogue::head_word(head.alarm_word(), true) == 0 && head.referenced() {
            return;
        }
        tracing::warn!(
            head_alarms = format_args!("{:#06x}", head.alarm_word() & 0xffff),
            referenced = head.referenced(),
            "head fault while cutting: pausing"
        );
        if let Some(active) = self.active.take()
            && let Err(error) = self.end_run(active, Ending::Held, true).await
        {
            self.last_error = Some(error.to_string());
        }
        if self.link.is_some() {
            self.capture = Some(Capture::new());
        }
        self.publish();
    }

    async fn write_one(&mut self) {
        let Some(mut active) = self.active.take() else { return };
        let Some(write) = active.pending.pop_front() else {
            self.active = Some(active);
            return;
        };
        active.sent = true;
        let outcome = match self.link.as_mut() {
            Some(link) => link.write(&write).await.map_err(Error::from),
            None => Err(Error::Disconnected),
        };
        match outcome {
            Ok(()) => {
                active.job.operation().acknowledged(&write);
                active.written_at = Instant::now();
                if active.finishing && active.pending.is_empty() {
                    self.complete(active);
                } else {
                    self.active = Some(active);
                }
            }
            Err(error) => self.fail(active, error).await,
        }
    }

    async fn read_one(&mut self) {
        let Some(link) = self.link.as_mut() else { return };
        let Some(mut capture) = self.capture.take() else { return };
        let Some(read) = capture.next() else { return };
        let outcome = match link.read(read).await {
            Ok(words) => capture.accept(words, &mut self.details),
            Err(error) => Err(error),
        };
        match outcome {
            Ok(Some(snapshot)) => {
                self.missed_polls = 0;
                self.head_watch_paused = false;
                self.snapshot = Some(snapshot);
                self.monitor.operation = self.operation_state();
                self.monitor.observe(&snapshot, &self.details);
                self.advance(&snapshot).await;
            }
            Ok(None) => self.capture = Some(capture),
            Err(error @ LinkError::NoReply("read", _)) => {
                self.missed_polls += 1;
                if self.missed_polls >= MAX_MISSED_POLLS {
                    self.fault(format!("{error} after {MAX_MISSED_POLLS} feedback attempts")).await;
                    return;
                }
                tracing::warn!(%error, attempt = self.missed_polls, "retrying controller feedback");
                // Stop cutting on missing feedback, retaining the run for
                // continuation if the controller answers the next poll.
                if self
                    .active
                    .as_ref()
                    .is_some_and(|active| matches!(&active.job, Job::Run(run, _) if run.active()))
                    && let Some(active) = self.active.take()
                {
                    let _ = self.end_run(active, Ending::Held, false).await;
                }
                if self.link.is_some() {
                    // Discard the whole partial capture. A successful
                    // retry must supply fresh, consistent feedback. Return
                    // to the task loop between reads so Stop still wins.
                    self.capture = Some(Capture::new());
                }
            }
            Err(error) => self.fault(error.to_string()).await,
        }
    }

    /// The vendor's operation state for arming input rules: 2 while a
    /// program runs, 3 while one is held, 4 during a manual operation.
    fn operation_state(&self) -> u32 {
        match &self.active {
            Some(active) => active.job.view().operation_state(),
            None if self.program.as_ref().is_some_and(|p| p.state == ProgramState::Held) => 3,
            None => 0,
        }
    }

    async fn advance(&mut self, snapshot: &Snapshot) {
        let pause = self.monitor.take_pause_request();
        if let Err(reason) = self.validate_snapshot(snapshot) {
            self.fault(reason).await;
            return;
        }
        let Some(mut active) = self.active.take() else { return };
        // A blocking alarm or a fresh gas row pauses the program before
        // advancing it, so the run can settle and capture its checkpoint.
        let blocked = self.monitor.blocked_for(active.job.concession(snapshot));
        if let Job::Run(run, _) = &mut active.job
            && run.active()
            && (pause || blocked.is_some())
        {
            if let Err(error) = self.end_run(active, Ending::Held, true).await {
                self.last_error = Some(error.to_string());
            }
            return;
        }
        if !active.pending.is_empty() || snapshot.taken <= active.written_at {
            self.active = Some(active);
            return;
        }
        self.advance_operation(active, snapshot).await;
    }

    fn validate_snapshot(&mut self, snapshot: &Snapshot) -> std::result::Result<(), String> {
        if !snapshot.head.referenced() {
            self.session.calibration = None;
        }
        if self.session.parameters.is_some_and(|accepted| snapshot.parameters() != Ok(accepted)) {
            return Err(
                "the controller configuration changed; reconnect and read the parameters".into()
            );
        }
        // The reference is lost with either axis's reference bit, or when the
        // live configuration no longer keeps the one Go Origin used.
        let moved = snapshot.parameters().is_ok_and(|live| {
            self.session.reference.is_some_and(|reference| !reference.keeps_reference(&live))
        });
        if self.session.is_homed()
            && (!snapshot.axes.axis(0).referenced() || !snapshot.axes.axis(1).referenced() || moved)
        {
            self.session.homed = None;
            if self
                .active
                .as_ref()
                .is_some_and(|active| matches!(&active.job, Job::Run(..) | Job::Motion(..)))
            {
                return Err("the XY reference was lost".into());
            }
        }
        Ok(())
    }

    async fn advance_operation(&mut self, mut active: Active, snapshot: &Snapshot) {
        let parameters = match &active.job {
            Job::Run(..) => self.homed_with_parameters().map(|_| ()),
            Job::Motion(motion, _) if motion.recovering_limit(snapshot) => Ok(()),
            Job::Motion(..) => self.verified_parameters().map(|_| ()),
            _ => Ok(()),
        };
        if let Err(error) = parameters {
            self.fail(active, error).await;
            return;
        }
        let blocked = self.monitor.blocked_for(active.job.concession(snapshot));
        let step = active.job.operation().step(snapshot, blocked.as_deref(), Instant::now());
        match step {
            Ok(Step::Wait) => self.active = Some(active),
            Ok(step @ (Step::Send(_) | Step::Finish(_))) => {
                active.finishing = matches!(&step, Step::Finish(_));
                if let Step::Send(writes) | Step::Finish(writes) = step {
                    active.pending = writes.into();
                }
                if active.finishing && active.pending.is_empty() {
                    self.complete(active);
                } else {
                    self.active = Some(active);
                }
            }
            Err(reason) => {
                let error =
                    if active.sent { Error::Failed(reason) } else { Error::Refused(reason) };
                self.fail(active, error).await;
            }
        }
    }

    /// A poll failed: the connection is faulted until an explicit reconnect.
    /// Whatever ran is stopped as far as the dead link allows.
    async fn fault(&mut self, reason: String) {
        tracing::warn!(%reason, "connection lost");
        if let Some(active) = self.active.take() {
            self.fail(active, Error::Link(reason.clone())).await;
        }
        self.mark_fault(reason);
    }

    fn mark_fault(&mut self, reason: String) {
        self.invalidate(&reason);
        self.link = None;
        self.snapshot = None;
        self.capture = None;
        self.connection = Connection::Faulted { reason };
    }

    /// OFF/stop/clear writes are all attempted, even after an acknowledgement
    /// is lost. Each exchange keeps the link's deadline and is sent once.
    ///
    /// A write the controller refuses, after the link's retries, was not
    /// carried out: the vendor carries on with the rest and keeps the
    /// connection, and so does this, telling the operator. A write with no
    /// reply, or an unreadable one, leaves the outputs uncertain and faults
    /// the connection.
    async fn cleanup(&mut self, writes: &[Write]) -> Result<()> {
        let link = self.link.as_mut().ok_or(Error::Disconnected)?;
        let (mut uncertain, mut refused) = (Vec::new(), Vec::new());
        for write in writes {
            match link.write(write).await {
                Ok(()) => {}
                Err(error @ LinkError::Refused(..)) => refused.push(error.to_string()),
                Err(error) => uncertain.push(error.to_string()),
            }
        }
        if !refused.is_empty() {
            let told = format!("the controller refused: {}", refused.join("; "));
            self.last_error = Some(told.clone());
            self.refused = Some(told);
        }
        if uncertain.is_empty() {
            return Ok(());
        }
        let reason = format!("shutdown uncertain: {}", uncertain.join("; "));
        self.mark_fault(reason.clone());
        Err(Error::Link(reason))
    }

    fn invalidate(&mut self, reason: &str) {
        self.session.invalidate();
        if let Some(program) = &mut self.program {
            program.state = ProgramState::Failed;
            program.checkpoint = None;
            program.error = Some(reason.to_owned());
        }
        self.last_error = Some(reason.to_owned());
    }

    /// The stop sequence for the configured machine.
    fn abort_writes(&self, uncertain: bool) -> Vec<Write> {
        let Some(bindings) = &self.bindings else { return Vec::new() };
        let head_active =
            uncertain || self.snapshot.is_some_and(|snapshot| snapshot.head.command() != 0);
        sequences::abort(&bindings.shutdown, head_active).unwrap_or_default()
    }

    /// Ends the active operation with `error`: its cancel writes and the
    /// stop sequence when anything was sent, then the bookkeeping.
    async fn fail(&mut self, active: Active, mut error: Error) {
        // A host pause/stop watchdog is not a transport failure. Keep
        // polling a responding controller after acknowledged cleanup so
        // the operator can see its position/alarms and stop it again.
        // No checkpoint is admitted from this failed ending.
        let ending_failed = matches!(&active.job, Job::Run(run, _)
            if matches!(run.state(), ProgramState::Held | ProgramState::Stopped));
        if active.sent {
            if self.link.is_some() {
                let mut writes = active.job.view().cancel();
                writes.extend(self.abort_writes(true));
                if let Err(cleanup) = self.cleanup(&writes).await {
                    error = Error::Link(format!("{error}; {cleanup}"));
                }
            }
            if !ending_failed || !matches!(error, Error::Failed(_)) {
                self.mark_fault(error.to_string());
            }
        }
        self.settle(active, error);
    }

    /// Records a failed operation and answers its requester.
    fn settle(&mut self, active: Active, error: Error) {
        if let Job::Run(run, _) = &active.job {
            self.program = Some(Self::program_view(
                run,
                ProgramState::Failed,
                Some(error.to_string()),
                self.snapshot.as_ref(),
            ));
        }
        self.last_error = Some(error.to_string());
        self.refused = None;
        self.publish();
        active.job.fail(error);
    }

    /// Records a finished operation and answers its requester.
    fn complete(&mut self, active: Active) {
        let epoch = self.session.epoch;
        self.last_error = self.refused.take();
        match active.job {
            Job::Home(home, reply) => {
                if home.reference_verified() {
                    self.session.homed = Some(epoch);
                    self.session.reference =
                        self.snapshot.as_ref().and_then(|snapshot| snapshot.parameters().ok());
                }
                self.answer(reply, Ok(()));
            }
            Job::Calibrate(calibrate, reply) => match calibrate.quality() {
                // Recorded, so setup shows it; the connection stands.
                Some(Quality::Bad) => {
                    self.session.calibration = Some((Quality::Bad, epoch));
                    self.answer(
                        reply,
                        Err(Error::Failed("the head reported a bad calibration".into())),
                    );
                }
                Some(quality) => {
                    self.session.calibration = Some((quality, epoch));
                    self.answer(reply, Ok(quality));
                }
                None => self.answer(
                    reply,
                    Err(Error::Failed("the calibration reported no quality".into())),
                ),
            },
            Job::Motion(_, reply) | Job::Outputs(_, reply) => self.answer(reply, Ok(())),
            Job::Mode(switch, reply) => {
                self.session.mode_applied(switch.mode());
                if !self.xy_referenced() {
                    self.session.homed = None;
                }
                self.answer(reply, Ok(()));
            }
            Job::Relief(relief, id, reply) => {
                if relief.clears_host_rows() {
                    self.monitor.common_reset_acknowledged();
                }
                // A dual-drive reset restores the limits; the vendor then
                // asks for a fresh reference.
                if id.is_some_and(|id| {
                    catalogue::relief(id) == catalogue::Relief::DualDriveAndHomeDecision
                }) {
                    self.session.homed = None;
                }
                self.answer(reply, Ok(()));
            }
            Job::Run(run, reply) => {
                let ending = run.ending().unwrap_or(Ending::Completed);
                self.program =
                    Some(Self::program_view(&run, run.state(), None, self.snapshot.as_ref()));
                self.answer(reply, Ok(ending));
            }
            Job::Parameters(parameters, homed, reply) => {
                if let Some(verified) = parameters.verified() {
                    if parameters.initialized()
                        && let Some(bindings) = &self.bindings
                    {
                        self.session.mode_applied(bindings.mode);
                    }
                    self.session.parameters = Some(verified);
                    if parameters.unchanged()
                        || (self.xy_referenced() && self.session.reference_holds(&verified))
                    {
                        self.session.homed = homed;
                    }
                    self.answer(reply, Ok(verified));
                } else {
                    self.answer(
                        reply,
                        Err(Error::Failed("the parameter readback is incomplete".into())),
                    );
                }
            }
        }
    }

    fn program_view(
        run: &Run,
        state: ProgramState,
        error: Option<String>,
        snapshot: Option<&Snapshot>,
    ) -> ProgramView {
        ProgramView {
            state,
            started: run.has_started(snapshot),
            uploaded: run.uploaded(),
            total: run.total(),
            checkpoint: run
                .checkpoint()
                .filter(|_| matches!(state, ProgramState::Held | ProgramState::Stopped)),
            error,
        }
    }

    fn configuration(&self) -> Option<crate::session::Configuration> {
        Some(crate::session::Configuration {
            epoch: self.session.epoch,
            binding: self.binding_revision,
            mode: self.bindings.as_ref()?.mode,
            verified: self.session.parameters?,
        })
    }

    fn publish(&mut self) {
        if let Some(Active { job: Job::Run(run, _), .. }) = &self.active {
            self.program = Some(Self::program_view(run, run.state(), None, self.snapshot.as_ref()));
        }
        let mut alarms = self.monitor.views();
        for alarm in &mut alarms {
            alarm.age_seconds = 0;
        }
        let fault = match &self.connection {
            Connection::Faulted { reason } => Some(reason.clone()),
            _ => None,
        };
        if alarms != self.previous_alarms || fault != self.previous_fault {
            self.alarm_revision += 1;
            self.previous_alarms.clone_from(&alarms);
            self.previous_fault.clone_from(&fault);
            let at = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs());
            let _ = self.alarm_events.send(crate::alarms::Observation { at, alarms, fault });
        }
        let mut state = State {
            alarm_revision: self.alarm_revision,
            configuration: self.configuration(),
            observed_parameters: self.snapshot.as_ref().and_then(|s| s.parameters().ok()),
            revision: self.revision,
            connection: self.connection.clone(),
            identity: self.identity,
            configured_mode: self.bindings.as_ref().map(|bindings| bindings.mode),
            feedback: self.snapshot.as_ref().and_then(feedback_view),
            session: SessionView {
                homed: self.session.is_homed(),
                mode: self.session.applied_mode(),
                parameters_verified: self.session.parameters.is_some(),
                calibration: self.session.calibration(),
            },
            alarms: self.monitor.views(),
            blocked: self.monitor.blocked(),
            motion_blocked: self.monitor.blocked_for(Concession::ManualPositioning {
                other_head_faults: self.snapshot.as_ref().is_some_and(home::other_head_faults),
            }),
            setup_blocked: self.monitor.blocked_for(Concession::ProcessOnly),
            xy_jog_blocked: std::array::from_fn(|axis| {
                [false, true].map(|positive| self.xy_jog_blocked(axis, positive))
            }),
            xy_recovery: self.snapshot.as_ref().is_some_and(Snapshot::xy_limit_alarms),
            head_jog_blocked: [false, true].map(|up| {
                self.snapshot.as_ref().map_or_else(
                    || Some("connect the machine first".into()),
                    |snapshot| {
                        let blocked =
                            self.monitor.blocked_for(outputs::head_concession(snapshot, up));
                        outputs::admit_head_jog(snapshot, blocked.as_deref(), up).err()
                    },
                )
            }),
            head_recovery: self.snapshot.as_ref().is_some_and(|snapshot| {
                snapshot.head_limit_alarms() != 0
                    && (snapshot.head_jog_alarms_clear(false)
                        || snapshot.head_jog_alarms_clear(true))
            }),
            operation: self.active.as_ref().map(|active| OperationView {
                kind: active.job.view().kind(),
                phase: active.job.view().phase().to_owned(),
                age_seconds: active.started.elapsed().as_secs(),
            }),
            program: self.program.clone(),
            last_error: self.last_error.clone(),
        };
        // Every poll lands here, well over a hundred times a second; one
        // that changed nothing wakes no one. The state is compared with the
        // published one at its revision, which only moves with a change, and
        // with the snapshot's age, which is a millisecond or two on every
        // poll, taken as the published one's.
        {
            let published = self.publisher.borrow();
            let age = state.feedback.as_ref().map(|f| f.age_ms);
            if let (Some(feedback), Some(old)) =
                (state.feedback.as_mut(), published.feedback.as_ref())
            {
                feedback.age_ms = old.age_ms;
            }
            if *published == state {
                return;
            }
            if let (Some(feedback), Some(age)) = (state.feedback.as_mut(), age) {
                feedback.age_ms = age;
            }
        }
        self.revision += 1;
        state.revision = self.revision;
        self.publisher.send_replace(state);
    }

    fn xy_jog_blocked(&self, axis: usize, positive: bool) -> Option<String> {
        let snapshot = self.snapshot.as_ref()?;
        let blocked =
            self.monitor.blocked_for(motion::xy_concession(snapshot, axis, positive, false));
        motion::admit_xy_jog(snapshot, blocked.as_deref(), axis, positive).err()
    }
}

fn feedback_view(snapshot: &Snapshot) -> Option<Feedback> {
    let scale = snapshot.divisor().ok()?;
    let cycle_us = snapshot.system.interpolation_cycle_us().ok()?;
    let position_mm = snapshot.position_mm().ok()?;
    let status = &snapshot.status;
    let head = &snapshot.head;
    Some(Feedback {
        position_mm,
        table_mm: snapshot.axes.axis(4).position_mm(scale),
        table_stationary: snapshot.axes.axis(4).speed == 0 && snapshot.axes.axis(4).phase() == 0,
        scale,
        cycle_us,
        referenced: [snapshot.axes.axis(0).referenced(), snapshot.axes.axis(1).referenced()],
        stationary: snapshot.xy_stationary(),
        head: HeadView {
            referenced: head.referenced(),
            height_mm: f64::from(head.height()) / 1000.,
            command: head.command(),
            status: head.status_byte(),
        },
        inputs: status.inputs(),
        outputs: status.outputs(),
        extended_outputs: status.extended_outputs(),
        fifo: FifoView {
            free: status.fifo_free(),
            capacity: Some(snapshot.capacity),
            activity: status.fifo_activity(),
            stamp: status.transfer_stamp(),
            item: status.word(23).cast_signed(),
            progress: status.word(24).cast_signed(),
        },
        alarm_groups: [status.alarm_group_1(), status.alarm_group_2()],
        age_ms: u64::try_from(snapshot.taken.elapsed().as_millis()).unwrap_or(u64::MAX),
    })
}
