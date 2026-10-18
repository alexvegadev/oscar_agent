use crate::error::OscarError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    fs::File,
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    sync::{mpsc, oneshot},
};

/// Metadata omits payloads and their hashes (which could reveal low-entropy data).
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    #[default]
    Metadata,
    Content,
}

static SESSION_SEQUENCE: AtomicU64 = AtomicU64::new(0);
/// Host-generated identity; path names and model-supplied IDs are never used.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(transparent)]
pub struct SessionId(String);
impl SessionId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
    fn new() -> Result<Self, OscarError> {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| io_error())?;
        Ok(Self(format!(
            "{}-{}-{}",
            std::process::id(),
            time.as_nanos(),
            SESSION_SEQUENCE.fetch_add(1, Ordering::Relaxed)
        )))
    }
}

#[derive(Clone, Debug)]
pub struct JournalLimits {
    pub max_record_bytes: usize,
    pub max_total_bytes: u64,
    pub queue_capacity: usize,
    pub io_timeout_ms: u64,
}
impl Default for JournalLimits {
    fn default() -> Self {
        Self {
            max_record_bytes: 262_144,
            max_total_bytes: 67_108_864,
            queue_capacity: 8,
            io_timeout_ms: 5_000,
        }
    }
}
impl JournalLimits {
    fn validate(&self) -> Result<(), OscarError> {
        if !(1024..=4_194_304).contains(&self.max_record_bytes)
            || self.max_total_bytes < self.max_record_bytes as u64
            || self.max_total_bytes > 1_073_741_824
            || !(1..=64).contains(&self.queue_capacity)
            || !(1..=60_000).contains(&self.io_timeout_ms)
        {
            return Err(OscarError::Config("invalid journal limits".into()));
        }
        Ok(())
    }
}

/// No Debug implementation: captured content may contain sensitive user data.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub bytes: usize,
    pub sha256: Option<String>,
    pub content: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AttemptOrigin {
    pub run_id: String,
    pub task_id: String,
    pub attempt: usize,
}

/// Correlation values are host-generated. Never put raw diagnostics in status.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum JournalEvent {
    Opened {
        session_id: SessionId,
        capture: CaptureMode,
    },
    RunStarted {
        run_id: String,
    },
    RunFinished {
        run_id: String,
        outcome: crate::execution::Outcome,
    },
    Request {
        provider: crate::planning::ProviderPreference,
        origin: Option<AttemptOrigin>,
        payload: Payload,
    },
    WireRequest {
        call: u64,
        payload: Payload,
    },
    ResponseDelta {
        call: u64,
        payload: Payload,
    },
    Response {
        call: u64,
        payload: Payload,
    },
    CallFailed {
        call: u64,
    },
    ToolRequested {
        payload: Payload,
    },
    ToolFinished {
        request: u64,
        succeeded: bool,
        payload: Payload,
    },
    ContextReset,
    Validation {
        run_id: String,
        task: String,
        attempt: usize,
        passed: bool,
        strong: bool,
    },
    Closed,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct JournalRecord {
    pub version: u32,
    pub sequence: u64,
    pub previous_hash: String,
    pub event: JournalEvent,
    pub record_hash: String,
}

struct Pending {
    event: JournalEvent,
    ack: Option<oneshot::Sender<Result<u64, OscarError>>>,
}

/// One bounded writer per journal. Dropping all handles drains queued records.
/// Call `close` to await durable completion. Never reopen an existing log to append.
#[derive(Clone)]
pub struct Journal {
    id: SessionId,
    sender: mpsc::Sender<Pending>,
    failed: Arc<AtomicBool>,
    closed: Arc<AtomicBool>,
    capture: CaptureMode,
    limits: JournalLimits,
}

fn io_error() -> OscarError {
    OscarError::Io("session journal unavailable or incomplete".into())
}
fn limit_error() -> OscarError {
    OscarError::Limit("session journal capacity exhausted".into())
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Serializer with an allocation ceiling, including JSON escaping.
struct BoundedBytes {
    bytes: Vec<u8>,
    limit: usize,
}
impl Write for BoundedBytes {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if data.len() > self.limit.saturating_sub(self.bytes.len()) {
            return Err(std::io::Error::other("encoded journal record too large"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
pub(super) fn encode(value: &impl Serialize, limit: usize) -> Result<Vec<u8>, OscarError> {
    let mut output = BoundedBytes {
        bytes: Vec::new(),
        limit,
    };
    serde_json::to_writer(&mut output, value).map_err(|_| limit_error())?;
    Ok(output.bytes)
}

impl Journal {
    /// Host must choose a private, trusted directory outside tool-writable roots.
    /// Creation is exclusive; an existing file or symlink is never overwritten.
    pub async fn create(
        path: &Path,
        capture: CaptureMode,
        limits: JournalLimits,
    ) -> Result<Self, OscarError> {
        limits.validate()?;
        let mut options = tokio::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        options.mode(0o600);
        let file = tokio::time::timeout(
            Duration::from_millis(limits.io_timeout_ms),
            options.open(path),
        )
        .await
        .map_err(|_| io_error())?
        .map_err(|_| io_error())?;
        let (sender, receiver) = mpsc::channel(limits.queue_capacity);
        let journal = Self {
            id: SessionId::new()?,
            sender,
            failed: Arc::new(AtomicBool::new(false)),
            closed: Arc::new(AtomicBool::new(false)),
            capture,
            limits,
        };
        tokio::spawn(writer(
            file,
            receiver,
            journal.failed.clone(),
            journal.limits.clone(),
        ));
        journal
            .append(JournalEvent::Opened {
                session_id: journal.id.clone(),
                capture,
            })
            .await?;
        Ok(journal)
    }

    pub fn id(&self) -> &SessionId {
        &self.id
    }

    pub fn payload(&self, text: &str) -> Result<Payload, OscarError> {
        if text.len() > self.limits.max_record_bytes {
            return Err(limit_error());
        }
        Ok(Payload {
            bytes: text.len(),
            sha256: (self.capture == CaptureMode::Content).then(|| digest(text.as_bytes())),
            content: (self.capture == CaptureMode::Content).then(|| text.to_owned()),
        })
    }

    pub(super) fn serialized_payload(&self, value: &impl Serialize) -> Result<Payload, OscarError> {
        let bytes = encode(value, self.limits.max_record_bytes)?;
        let text = String::from_utf8(bytes).map_err(|_| io_error())?;
        self.payload(&text)
    }

    fn check(&self, event: &JournalEvent) -> Result<(), OscarError> {
        if self.failed.load(Ordering::SeqCst) || self.closed.load(Ordering::SeqCst) {
            return Err(io_error());
        }
        // Account for the fixed envelope before enqueueing a potentially large payload.
        encode(event, self.limits.max_record_bytes.saturating_sub(512))?;
        // Even hosts cannot accidentally send captured content to a metadata log.
        let payload = match event {
            JournalEvent::Request { payload, .. }
            | JournalEvent::WireRequest { payload, .. }
            | JournalEvent::ResponseDelta { payload, .. }
            | JournalEvent::Response { payload, .. } => Some(payload),
            JournalEvent::ToolRequested { payload }
            | JournalEvent::ToolFinished { payload, .. } => Some(payload),
            _ => None,
        };
        if self.capture == CaptureMode::Metadata
            && payload.is_some_and(|p| p.content.is_some() || p.sha256.is_some())
        {
            return Err(OscarError::Config(
                "content forbidden in metadata journal".into(),
            ));
        }
        Ok(())
    }

    /// Completes only after write, flush and sync_data. A timeout poisons the log:
    /// pending OS I/O cannot be cancelled reliably, so no new work may use it.
    pub async fn append(&self, event: JournalEvent) -> Result<u64, OscarError> {
        self.check(&event)?;
        let (tx, rx) = oneshot::channel();
        let operation = async {
            self.sender
                .send(Pending {
                    event,
                    ack: Some(tx),
                })
                .await
                .map_err(|_| io_error())?;
            rx.await.map_err(|_| io_error())?
        };
        match tokio::time::timeout(Duration::from_millis(self.limits.io_timeout_ms), operation)
            .await
        {
            Ok(result) => result,
            Err(_) => {
                self.failed.store(true, Ordering::SeqCst);
                Err(io_error())
            }
        }
    }

    /// Nonblocking streaming hook. Backpressure aborts inference instead of losing
    /// content. Queued deltas are provisional until a subsequent append is acked.
    pub(super) fn try_append(&self, event: JournalEvent) -> Result<(), OscarError> {
        self.check(&event)?;
        self.sender
            .try_send(Pending { event, ack: None })
            .map_err(|_| {
                self.failed.store(true, Ordering::SeqCst);
                io_error()
            })
    }

    pub async fn close(&self) -> Result<(), OscarError> {
        let result = self.append(JournalEvent::Closed).await.map(|_| ());
        self.closed.store(true, Ordering::SeqCst);
        result
    }
}

async fn writer(
    mut file: File,
    mut receiver: mpsc::Receiver<Pending>,
    failed: Arc<AtomicBool>,
    limits: JournalLimits,
) {
    let mut sequence = 0u64;
    let mut previous_hash = String::new();
    let mut total = 0u64;
    let mut closed = false;
    while let Some(pending) = receiver.recv().await {
        let result = async {
            if closed || failed.load(Ordering::SeqCst) {
                return Err(io_error());
            }
            sequence = sequence.checked_add(1).ok_or_else(limit_error)?;
            let mut record = JournalRecord {
                version: 1,
                sequence,
                previous_hash: previous_hash.clone(),
                event: pending.event,
                record_hash: String::new(),
            };
            record.record_hash = digest(&encode(&record, limits.max_record_bytes)?);
            let mut bytes = encode(&record, limits.max_record_bytes.saturating_sub(1))?;
            bytes.push(b'\n');
            total = total
                .checked_add(bytes.len() as u64)
                .ok_or_else(limit_error)?;
            if total > limits.max_total_bytes {
                return Err(limit_error());
            }
            let operation = async {
                file.write_all(&bytes).await?;
                file.flush().await?;
                file.sync_data().await
            };
            tokio::time::timeout(Duration::from_millis(limits.io_timeout_ms), operation)
                .await
                .map_err(|_| io_error())?
                .map_err(|_| io_error())?;
            previous_hash = record.record_hash;
            closed = matches!(record.event, JournalEvent::Closed);
            Ok(sequence)
        }
        .await;
        if result.is_err() {
            failed.store(true, Ordering::SeqCst);
        }
        if let Some(ack) = pending.ack {
            let _ = ack.send(result);
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct Verification {
    pub records: u64,
    pub last_hash: String,
    pub closed: bool,
    pub incomplete_tail: bool,
}

/// Bounded, read-only integrity inspection. Does not repair, replay or truncate.
/// A hash chain is tamper evidence relative to a trusted checkpoint, not a signature.
pub async fn verify(path: &Path, limits: JournalLimits) -> Result<Verification, OscarError> {
    limits.validate()?;
    let operation = async {
        let file = File::open(path).await.map_err(|_| io_error())?;
        let mut reader = BufReader::new(file);
        let mut state = Verification {
            records: 0,
            last_hash: String::new(),
            closed: false,
            incomplete_tail: false,
        };
        let mut total = 0u64;
        loop {
            let mut line = Vec::new();
            (&mut reader)
                .take(limits.max_record_bytes as u64 + 1)
                .read_until(b'\n', &mut line)
                .await
                .map_err(|_| io_error())?;
            if line.is_empty() {
                break;
            }
            total = total
                .checked_add(line.len() as u64)
                .ok_or_else(limit_error)?;
            if line.len() > limits.max_record_bytes || total > limits.max_total_bytes {
                return Err(limit_error());
            }
            if state.closed {
                return Err(io_error());
            }
            if !line.ends_with(b"\n") {
                state.incomplete_tail = true;
                break;
            }
            let mut record: JournalRecord =
                serde_json::from_slice(&line).map_err(|_| io_error())?;
            let hash = std::mem::take(&mut record.record_hash);
            if record.version != 1
                || record.sequence != state.records + 1
                || record.previous_hash != state.last_hash
                || digest(&encode(&record, limits.max_record_bytes)?) != hash
                || (state.records == 0 && !matches!(record.event, JournalEvent::Opened { .. }))
                || (state.records > 0 && matches!(record.event, JournalEvent::Opened { .. }))
            {
                return Err(io_error());
            }
            state.records += 1;
            state.last_hash = hash;
            state.closed = matches!(record.event, JournalEvent::Closed);
        }
        if state.records == 0 {
            return Err(io_error());
        }
        Ok(state)
    };
    tokio::time::timeout(Duration::from_millis(limits.io_timeout_ms), operation)
        .await
        .map_err(|_| io_error())?
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn failed_write_poisoning_rejects_every_subsequent_record() {
        // A read-only handle deterministically exercises a real OS write error,
        // without filling a disk or granting write access to the fixture.
        let file = File::open(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
            .await
            .unwrap();
        let (sender, receiver) = mpsc::channel(1);
        let failed = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn(writer(
            file,
            receiver,
            failed.clone(),
            JournalLimits::default(),
        ));
        for _ in 0..2 {
            let (tx, rx) = oneshot::channel();
            sender
                .send(Pending {
                    event: JournalEvent::ContextReset,
                    ack: Some(tx),
                })
                .await
                .unwrap();
            assert!(matches!(rx.await.unwrap(), Err(OscarError::Io(_))));
            assert!(failed.load(Ordering::SeqCst));
        }
        drop(sender);
        task.await.unwrap();
    }
}
