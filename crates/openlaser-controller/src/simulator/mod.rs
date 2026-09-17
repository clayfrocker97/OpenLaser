// SPDX-License-Identifier: GPL-3.0-or-later

//! A virtual controller on loopback UDP.
//!
//! It answers every request the task sends with feedback from a modelled
//! plant: axes that move at their commanded speed, a head that searches,
//! calibrates and follows, a FIFO that executes records at the
//! interpolation cycle, outputs, inputs and alarms. It exists for
//! development and tests. It is not vendor firmware and proves nothing
//! about the machine.

mod plant;

pub use plant::{CAPACITY, SCALE, View};

use crate::alarms::Rule;
use crate::config::Config;
use crate::session::Quality;
use openlaser_protocol::MAX_FRAME_BYTES;
use openlaser_protocol::registers::{AXIS_COUNT, PARAMETER_BANK_WORDS};
use openlaser_protocol::requests::Write;
use plant::Plant;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};
use tokio::net::UdpSocket;
use tokio::task::JoinHandle;

/// How often the plant advances when no request arrives.
const TICK: Duration = Duration::from_millis(2);

/// A fault the plant can be given.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Fault {
    /// No datagram is answered.
    Communications,
    /// X and Y stop advancing.
    AxisStall,
    /// The head stops advancing.
    HeadStall,
    /// Group 1 bit 30.
    EmergencyStop,
    /// Group 1 bit 29.
    Servo,
    /// Group 1 bit 25.
    Bus,
    /// Group 2 bit 5.
    FifoStarvation,
    /// Group 2 bit 0.
    IllegalCommand,
    /// Head bit 8.
    HeadFollow,
    /// Head bit 5.
    HeadTouch,
    /// Head bit 0, with the controller's head alarm summary.
    HeadUpperLimit,
    /// Head bit 1, with the controller's head alarm summary.
    HeadLowerLimit,
    /// A stopped, uncleared FIFO keeps advertising activity despite idle axes.
    StoppedFifoActivity,
    /// A stopped, uncleared FIFO reports a nonzero axis speed indefinitely.
    StopFeedbackBusy,
}

/// An alarm word that can be set directly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlarmBank {
    /// Controller aggregate group 1.
    Controller1,
    /// Controller aggregate group 2.
    Controller2,
    /// The head alarm word.
    Head,
}

/// The simulator: a socket, a plant and the task serving one from the other.
pub struct Simulator {
    endpoint: SocketAddrV4,
    plant: Arc<Mutex<Plant>>,
    server: JoinHandle<()>,
}

impl Simulator {
    /// Binds a loopback socket and starts serving.
    pub async fn start() -> std::io::Result<Self> {
        Self::start_at(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0)).await
    }

    /// Starts serving at `endpoint`, as a machine that comes back at the
    /// same address after its cable was pulled.
    pub async fn start_at(endpoint: SocketAddrV4) -> std::io::Result<Self> {
        if !endpoint.ip().is_loopback() {
            return Err(std::io::Error::other("the simulator only binds loopback"));
        }
        let socket = UdpSocket::bind(endpoint).await?;
        let endpoint = match socket.local_addr()? {
            SocketAddr::V4(endpoint) => endpoint,
            SocketAddr::V6(_) => {
                return Err(std::io::Error::other("the simulator needs an IPv4 loopback socket"));
            }
        };
        let plant = Arc::new(Mutex::new(Plant::new()));
        let server = tokio::spawn(serve(socket, Arc::clone(&plant)));
        Ok(Self { endpoint, plant, server })
    }

    /// Where the virtual controller listens.
    #[must_use]
    pub const fn endpoint(&self) -> SocketAddrV4 {
        self.endpoint
    }

    /// The task configuration that reaches this simulator.
    #[must_use]
    pub fn config(&self) -> Config {
        Config::loopback(self.endpoint)
    }

    /// A handle for inspecting and disturbing the plant.
    #[must_use]
    pub fn control(&self) -> Control {
        Control { plant: Arc::clone(&self.plant) }
    }
}

impl Drop for Simulator {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn serve(socket: UdpSocket, plant: Arc<Mutex<Plant>>) {
    let mut buffer = vec![0u8; MAX_FRAME_BYTES];
    let mut last = Instant::now();
    loop {
        let received = tokio::time::timeout(TICK, socket.recv_from(&mut buffer)).await;
        let now = Instant::now();
        let reply = {
            let mut plant = plant.lock().unwrap_or_else(PoisonError::into_inner);
            plant.advance(now.duration_since(last).as_secs_f64());
            match received {
                Ok(Ok((length, SocketAddr::V4(peer)))) if peer.ip().is_loopback() => {
                    plant.receive(&buffer[..length]).map(|bytes| (bytes, peer, plant.reply_delay))
                }
                _ => None,
            }
        };
        last = now;
        if let Some((bytes, peer, delay)) = reply {
            if !delay.is_zero() {
                tokio::time::sleep(delay).await;
            }
            if let Err(error) = socket.send_to(&bytes, SocketAddr::V4(peer)).await {
                tracing::debug!(%error, "simulator reply not sent");
            }
        }
    }
}

/// Inspection and disturbance of the plant.
#[derive(Clone)]
pub struct Control {
    plant: Arc<Mutex<Plant>>,
}

impl Control {
    fn with<T>(&self, act: impl FnOnce(&mut Plant) -> T) -> T {
        act(&mut self.plant.lock().unwrap_or_else(PoisonError::into_inner))
    }

    /// Every write received since the last [`Control::take_writes`], in
    /// order, including writes the plant refused.
    #[must_use]
    pub fn writes(&self) -> Vec<Write> {
        self.with(|plant| plant.writes.clone())
    }

    /// Takes the recorded writes, leaving the log empty.
    #[must_use]
    pub fn take_writes(&self) -> Vec<Write> {
        self.with(|plant| std::mem::take(&mut plant.writes))
    }

    /// Forgets the recorded writes.
    pub fn clear_writes(&self) {
        self.with(|plant| plant.writes.clear());
    }

    /// Drops the next `count` replies to reads of one register, without
    /// affecting write acknowledgements or the plant's motion.
    pub fn drop_reads(&self, address: u32, count: usize) {
        self.with(|plant| {
            plant.drop_reads.insert(address, count);
        });
    }

    /// Number of reads received for one register, including dropped replies.
    #[must_use]
    pub fn read_count(&self, address: u32) -> usize {
        self.with(|plant| plant.read_counts.get(&address).copied().unwrap_or(0))
    }

    /// The plant as it stands.
    #[must_use]
    pub fn view(&self) -> View {
        self.with(Plant::view)
    }

    /// Gives or lifts a fault.
    pub fn fault(&self, fault: Fault, present: bool) {
        self.with(|plant| {
            if present {
                plant.faults.insert(fault);
            } else {
                plant.faults.remove(&fault);
            }
        });
    }

    /// Forces digital input 1 to 24 to a level, or `None` to release it.
    pub fn input(&self, input: u8, level: Option<bool>) {
        self.with(|plant| match level {
            Some(level) => drop(plant.input_overrides.insert(input, level)),
            None => drop(plant.input_overrides.remove(&input)),
        });
    }

    /// Sets an alarm word directly.
    pub fn alarm_word(&self, bank: AlarmBank, value: u32) {
        self.with(|plant| {
            let index = match bank {
                AlarmBank::Controller1 => 0,
                AlarmBank::Controller2 => 1,
                AlarmBank::Head => 2,
            };
            plant.raw_alarms[index] = value;
        });
    }

    /// Sets the six detail bits of axis summary `axis`, 0 to 24.
    pub fn axis_alarm(&self, axis: usize, bits: u32) {
        self.with(|plant| {
            if let Some(word) = plant.axis_alarms.get_mut(axis) {
                *word = bits & 0x3f;
            }
        });
    }

    /// Whether the head reports a reference.
    pub fn head_reference(&self, ready: bool) {
        self.with(|plant| plant.head_reference_ready = ready);
    }

    /// Places the simulated head on a physical limit. Unlike an injected
    /// stuck fault, this switch releases when the head moves away.
    pub fn head_on_limit(&self, upper: bool) {
        self.with(|plant| plant.place_head_on_limit(upper));
    }

    /// Places X or Y on a modeled positive/negative hardware/software limit.
    /// A finite move away releases the contact; resetting alone does not.
    pub fn axis_on_limit(&self, axis: usize, positive: bool, software: bool) {
        self.with(|plant| plant.place_axis_on_limit(axis, positive, software));
    }

    /// Whether axis 0 to 4 reports a reference.
    pub fn axis_reference(&self, axis: usize, ready: bool) {
        self.with(|plant| {
            if let Some(referenced) = plant.referenced.get_mut(axis) {
                *referenced = ready;
            }
        });
    }

    /// The next write is applied but not acknowledged.
    pub fn drop_next_ack(&self) {
        self.drop_ack_after(0);
    }

    /// Applies one write without acknowledging it, after `accepted` writes.
    pub fn drop_ack_after(&self, accepted: usize) {
        self.with(|plant| plant.drop_ack_after = Some(accepted));
    }

    /// Delays replies, to exercise control requests during an exchange.
    pub fn reply_delay(&self, delay: Duration) {
        self.with(|plant| plant.reply_delay = delay);
    }

    /// The quality the next calibration reports.
    pub fn calibration_quality(&self, quality: Quality) {
        self.with(|plant| {
            plant.calibration_quality = match quality {
                Quality::Excellent => 0x10,
                Quality::Good => 0x11,
                Quality::Bad => 0x12,
            };
        });
    }

    /// The product id the plant reports; anything but 103 is not an MCC100.
    pub fn product(&self, id: u32) {
        self.with(|plant| plant.product = id);
    }

    /// How much faster than real time the FIFO executes, 0.1 to 100.
    pub fn time_scale(&self, scale: f64) {
        self.with(|plant| plant.time_scale = scale.clamp(0.1, 100.));
    }

    /// The input rules the plant answers: an active-low input reads high
    /// until forced.
    pub fn rules(&self, rules: &[Rule]) {
        self.with(|plant| plant.rules = rules.to_vec());
    }

    /// The five axis parameter banks as the plant holds them.
    #[must_use]
    pub fn parameters(&self) -> [[u32; PARAMETER_BANK_WORDS]; AXIS_COUNT] {
        self.with(|plant| plant.banks)
    }

    /// Replaces the axis parameter banks, as a machine's own configuration
    /// would have them.
    pub fn set_parameters(&self, banks: [[u32; PARAMETER_BANK_WORDS]; AXIS_COUNT]) {
        self.with(|plant| plant.banks = banks);
    }
}
