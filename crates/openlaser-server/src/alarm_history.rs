// SPDX-License-Identifier: GPL-3.0-or-later

//! Read-only alarm records, one file per application start. Disk I/O runs
//! on its own worker and never participates in controller admission.

use crate::{Error, Result};
use openlaser_controller::alarms::Observation;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio::sync::{oneshot, watch};

/// Recording health and the revision used to refresh an open history panel.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Default, Serialize)]
pub struct HistoryStatus {
    /// Changes after every recorded transition or reset result.
    pub revision: u64,
    /// A storage failure; never silently represented as successful recording.
    pub error: Option<String>,
}

/// One alarm's occurrences during an application session.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlarmRecord {
    /// Stable source and native ID, independent of the translated label.
    pub key: String,
    /// Native alarm ID, if available.
    pub id: Option<u32>,
    /// Where the alarm was observed.
    pub source: String,
    /// Latest plain name, as operators read it.
    pub label: String,
    /// First trip in seconds since the epoch.
    pub first: u64,
    /// Most recent trip.
    pub last: u64,
    /// Distinct inactive-to-active transitions.
    pub count: u64,
    /// Last observed recovery, if any. Restart does not imply recovery.
    pub recovered: Option<u64>,
    /// Whether the cause was present at the last observation in this session.
    pub active: bool,
    /// Most recent reset attempt for this row.
    pub reset: Option<ResetRecord>,
}

/// A reset result kept as history, never an acknowledgement requirement.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResetRecord {
    /// Time the attempt finished.
    pub at: u64,
    /// Whether the controller operation succeeded.
    pub ok: bool,
    /// Failure detail, if the operation failed.
    pub error: Option<String>,
}

/// Alarms grouped under one application restart separator.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AlarmSession {
    /// File schema version.
    pub version: u32,
    /// Unique application start ID and pagination cursor.
    pub id: String,
    /// Application start time in seconds since the epoch.
    pub started: u64,
    /// Alarm summaries; their active state is historical for earlier sessions.
    pub alarms: Vec<AlarmRecord>,
}

/// A page of restart groups, newest first.
#[cfg_attr(feature = "typescript", derive(ts_rs::TS), ts(export))]
#[derive(Clone, Debug, Serialize)]
pub struct HistoryPage {
    /// Current application session, for labelling rather than live alarm state.
    pub current: String,
    /// Up to ten restart groups.
    pub sessions: Vec<AlarmSession>,
    /// Cursor for older groups, if more remain.
    pub before: Option<String>,
}

enum Command {
    Observe(Observation),
    Reset(Vec<String>, ResetRecord),
    Page(Option<String>, oneshot::Sender<Result<HistoryPage>>),
    Flush(oneshot::Sender<Result<()>>),
}

/// Handle to the journal worker.
#[derive(Clone)]
pub struct History {
    sender: std::sync::mpsc::Sender<Command>,
    status: watch::Receiver<HistoryStatus>,
    drained: watch::Receiver<bool>,
    /// This application start, also used to invalidate preflight after restart.
    pub session: String,
}

impl History {
    /// Opens a restart group and starts a worker for ordered, durable writes.
    pub fn start(
        root: &Path,
        mut observations: tokio::sync::mpsc::UnboundedReceiver<Observation>,
    ) -> Result<Self> {
        let dir = root.join("alarm-history");
        std::fs::create_dir_all(&dir).map_err(|e| Error::Refused(format!("alarm history: {e}")))?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| Error::Refused(e.to_string()))?
            .as_nanos();
        let session = AlarmSession {
            version: 1,
            id: format!("{nanos:032x}"),
            started: openlaser_library::now(),
            alarms: Vec::new(),
        };
        let mut journal = Journal {
            dir,
            session: session.clone(),
            pending_resets: std::collections::BTreeMap::new(),
        };
        journal.save()?;
        let (sender, receiver) = std::sync::mpsc::channel();
        let (publisher, status) = watch::channel(HistoryStatus::default());
        tokio::task::spawn_blocking(move || {
            while let Ok(command) = receiver.recv() {
                match command {
                    Command::Page(before, reply) => {
                        let _ = reply.send(journal.page(before.as_deref()));
                    }
                    Command::Flush(reply) => {
                        let _ = reply.send(journal.save());
                    }
                    command => {
                        match command {
                            Command::Observe(observation) => journal.observe(observation),
                            Command::Reset(keys, reset) => {
                                for key in keys {
                                    if let Some(row) =
                                        journal.session.alarms.iter_mut().find(|row| row.key == key)
                                    {
                                        row.reset = Some(reset.clone());
                                    } else {
                                        journal.pending_resets.insert(key, reset.clone());
                                    }
                                }
                            }
                            _ => unreachable!(),
                        }
                        let error = journal.save().err().map(|e| e.to_string());
                        if let Some(error) = &error {
                            tracing::error!(%error, "alarm history write failed");
                        }
                        publisher.send_modify(|status| {
                            status.revision += 1;
                            status.error = error;
                        });
                    }
                }
            }
        });
        let (done, drained) = watch::channel(false);
        let forward = sender.clone();
        tokio::spawn(async move {
            while let Some(observation) = observations.recv().await {
                if forward.send(Command::Observe(observation)).is_err() {
                    break;
                }
            }
            done.send_replace(true);
        });
        Ok(Self { sender, status, drained, session: session.id })
    }

    /// Current recording status.
    #[must_use]
    pub fn status(&self) -> HistoryStatus {
        self.status.borrow().clone()
    }

    /// Watch recording changes without polling disk.
    #[must_use]
    pub fn watch(&self) -> watch::Receiver<HistoryStatus> {
        self.status.clone()
    }

    /// Retains the result of an explicit live reset request.
    pub fn reset(
        &self,
        rows: &[openlaser_controller::state::AlarmView],
        id: Option<u32>,
        result: &std::result::Result<(), openlaser_controller::Error>,
    ) {
        let reset = ResetRecord {
            at: openlaser_library::now(),
            ok: result.is_ok(),
            error: result.as_ref().err().map(ToString::to_string),
        };
        let keys = rows
            .iter()
            .filter(|row| id.is_none_or(|id| row.id == Some(id)))
            .map(|row| alarm_key(&row.source, row.id))
            .collect();
        if self.sender.send(Command::Reset(keys, reset)).is_err() {
            tracing::error!("alarm history worker stopped");
        }
    }

    /// Reads a page without loading the entire historical record into the app.
    pub async fn page(&self, before: Option<String>) -> Result<HistoryPage> {
        if before.as_ref().is_some_and(|id| !valid_id(id)) {
            return Err(Error::Request("invalid alarm history cursor".into()));
        }
        let (reply, answer) = oneshot::channel();
        self.sender
            .send(Command::Page(before, reply))
            .map_err(|e| Error::Refused(e.to_string()))?;
        answer.await.map_err(|e| Error::Refused(e.to_string()))?
    }

    /// Drains final controller observations after its task has stopped.
    pub async fn flush(&self) -> Result<()> {
        let mut drained = self.drained.clone();
        if !*drained.borrow_and_update() {
            tokio::time::timeout(std::time::Duration::from_secs(2), drained.changed())
                .await
                .map_err(|_| Error::Refused("alarm history did not drain".into()))?
                .map_err(|e| Error::Refused(e.to_string()))?;
        }
        let (reply, answer) = oneshot::channel();
        self.sender.send(Command::Flush(reply)).map_err(|e| Error::Refused(e.to_string()))?;
        answer.await.map_err(|e| Error::Refused(e.to_string()))?
    }
}

fn alarm_key(source: &str, id: Option<u32>) -> String {
    if source == "connection" { source.to_owned() } else { format!("{source}:{id:?}") }
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

struct Journal {
    dir: PathBuf,
    session: AlarmSession,
    pending_resets: std::collections::BTreeMap<String, ResetRecord>,
}

impl Journal {
    fn save(&self) -> Result<()> {
        let bytes = serde_json::to_vec(&self.session).map_err(|e| Error::Refused(e.to_string()))?;
        openlaser_library::atomic_write(&self.dir.join(format!("{}.json", self.session.id)), &bytes)
            .map_err(Error::from)
    }

    fn observe(&mut self, observation: Observation) {
        let mut present = Vec::new();
        for alarm in observation.alarms {
            let key = alarm_key(&alarm.source, alarm.id);
            present.push(key.clone());
            self.trip(alarm.id, alarm.source, alarm.title, alarm.active, observation.at);
        }
        if let Some(reason) = observation.fault {
            present.push("connection".into());
            self.trip(
                None,
                "connection".into(),
                format!("Controller link lost: {reason}"),
                true,
                observation.at,
            );
        }
        for row in &mut self.session.alarms {
            if row.active && !present.contains(&row.key) {
                row.active = false;
                row.recovered = Some(observation.at);
            }
        }
    }

    fn trip(&mut self, id: Option<u32>, source: String, label: String, active: bool, at: u64) {
        let key = alarm_key(&source, id);
        if let Some(row) = self.session.alarms.iter_mut().find(|row| row.key == key) {
            if active && !row.active {
                row.count += 1;
                row.last = at;
                row.recovered = None;
            }
            if !active && row.active {
                row.recovered = Some(at);
            }
            row.active = active;
            row.label = label;
        } else {
            let reset = self.pending_resets.remove(&key);
            self.session.alarms.push(AlarmRecord {
                key,
                id,
                source,
                label,
                first: at,
                last: at,
                count: 1,
                recovered: (!active).then_some(at),
                active,
                reset,
            });
        }
    }

    fn page(&self, before: Option<&str>) -> Result<HistoryPage> {
        let failed = |e: std::io::Error| Error::Refused(format!("alarm history: {e}"));
        let mut ids: Vec<_> = std::fs::read_dir(&self.dir)
            .map_err(failed)?
            .map(|entry| entry.map_err(failed))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .filter_map(|entry| {
                let name = entry.file_name();
                let id = name.to_str()?.strip_suffix(".json")?;
                (valid_id(id) && before.is_none_or(|before| id < before)).then(|| id.to_owned())
            })
            .collect();
        ids.sort_by(|a, b| b.cmp(a));
        let more = ids.len() > 10;
        ids.truncate(10);
        let mut sessions = Vec::new();
        for id in &ids {
            if id == &self.session.id {
                sessions.push(self.session.clone());
                continue;
            }
            let bytes = std::fs::read(self.dir.join(format!("{id}.json"))).map_err(failed)?;
            let session: AlarmSession = serde_json::from_slice(&bytes)
                .map_err(|e| Error::Refused(format!("alarm history {id}: {e}")))?;
            if session.version != 1 || session.id != *id {
                return Err(Error::Refused(format!("unsupported or corrupt alarm history {id}")));
            }
            sessions.push(session);
        }
        Ok(HistoryPage {
            current: self.session.id.clone(),
            sessions,
            before: more.then(|| ids.last().cloned()).flatten(),
        })
    }
}
