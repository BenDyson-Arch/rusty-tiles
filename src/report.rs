//! Conversion events and results shared by every converter.
//!
//! Converters report progress, warnings and notes through a [`Reporter`]
//! supplied by the caller instead of reading process environment, and return
//! a [`ConversionResult`] carrying the published path and its report.
use serde_json::{json, Value};
use std::{fmt, path::PathBuf, sync::Arc};

/// One conversion event.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub enum Event<'a> {
    /// `done` of `total` units of a named phase; `total` is `None` when the
    /// phase size is not known in advance.
    Progress {
        phase: &'a str,
        done: u64,
        total: Option<u64>,
    },
    /// Something was skipped or degraded. `detail` is the structured record
    /// (for example a vector feature report), when there is one.
    Warning {
        message: &'a str,
        detail: Option<&'a Value>,
    },
    /// Informational diagnostics such as timings.
    Note { message: &'a str },
}

/// Receiver for conversion events. Implementations must be cheap and must not
/// panic; converters call them from their working threads.
pub trait EventSink: Send + Sync {
    fn event(&self, event: &Event<'_>);
}

#[derive(Clone, Default)]
enum Sink {
    Silent,
    /// Warnings and notes as text on stderr; no progress lines.
    #[default]
    HumanStderr,
    /// Every event as one JSON object per stderr line.
    NdjsonStderr,
    Custom(Arc<dyn EventSink>),
}

/// Where a conversion sends its events. The default writes warnings and notes
/// as text on stderr and discards progress, which is what library callers got
/// before reporters existed.
#[derive(Clone, Default)]
pub struct Reporter(Sink);

impl fmt::Debug for Reporter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self.0 {
            Sink::Silent => "Reporter::silent",
            Sink::HumanStderr => "Reporter::human_stderr",
            Sink::NdjsonStderr => "Reporter::ndjson_stderr",
            Sink::Custom(_) => "Reporter::custom",
        })
    }
}

impl Reporter {
    /// Discard every event.
    pub fn silent() -> Self {
        Self(Sink::Silent)
    }
    /// Warnings and notes as text on stderr; progress is discarded.
    pub fn human_stderr() -> Self {
        Self(Sink::HumanStderr)
    }
    /// Newline-delimited JSON on stderr: `{"event":"progress","phase",...}`,
    /// `{"event":"warning",...}` and `{"event":"log","message":...}`.
    pub fn ndjson_stderr() -> Self {
        Self(Sink::NdjsonStderr)
    }
    /// Forward every event to a caller-supplied sink.
    pub fn custom(sink: Arc<dyn EventSink>) -> Self {
        Self(Sink::Custom(sink))
    }

    /// Whether progress events are observed (lets hot loops skip work).
    pub fn wants_progress(&self) -> bool {
        matches!(self.0, Sink::NdjsonStderr | Sink::Custom(_))
    }

    pub fn emit(&self, event: &Event<'_>) {
        match &self.0 {
            Sink::Silent => {}
            Sink::Custom(sink) => sink.event(event),
            Sink::HumanStderr => match *event {
                Event::Progress { .. } => {}
                Event::Warning { message, .. } | Event::Note { message } => {
                    eprintln!("{message}")
                }
            },
            Sink::NdjsonStderr => eprintln!("{}", ndjson(event)),
        }
    }

    pub fn progress(&self, phase: &str, done: u64, total: impl Into<Option<u64>>) {
        if self.wants_progress() {
            self.emit(&Event::Progress {
                phase,
                done,
                total: total.into(),
            });
        }
    }

    pub fn warn(&self, message: &str, detail: Option<&Value>) {
        self.emit(&Event::Warning { message, detail });
    }

    pub fn note(&self, message: &str) {
        self.emit(&Event::Note { message });
    }
}

/// The NDJSON line for an event. A structured warning keeps its own fields and
/// gains `"event":"warning"`.
pub fn ndjson(event: &Event<'_>) -> Value {
    match *event {
        Event::Progress { phase, done, total } => {
            json!({"event":"progress","phase":phase,"done":done,"total":total})
        }
        Event::Warning {
            detail: Some(detail @ Value::Object(_)),
            ..
        } => {
            let mut value = detail.clone();
            value["event"] = json!("warning");
            value
        }
        Event::Warning { message, .. } => json!({"event":"warning","message":message}),
        Event::Note { message } => json!({"event":"log","message":message}),
    }
}

/// Outcome of a successful conversion.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ConversionResult {
    /// The published output path, as given by the caller.
    pub output: PathBuf,
    /// `true` for a `.3tz` archive, `false` for a directory.
    pub archive: bool,
    /// The published `conversion.json` content, for converters that write one.
    pub report: Option<Value>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn ndjson_lines_keep_the_cli_event_contract() {
        let progress = Event::Progress {
            phase: "terrain",
            done: 3,
            total: Some(9),
        };
        assert_eq!(
            ndjson(&progress).to_string(),
            r#"{"done":3,"event":"progress","phase":"terrain","total":9}"#
        );
        let open = Event::Progress {
            phase: "encoding",
            done: 1,
            total: None,
        };
        assert_eq!(ndjson(&open)["total"], Value::Null);
        let detail = json!({"reason":"bad ring","sourceId":"7"});
        let warning = Event::Warning {
            message: "layer '', feature 7: bad ring",
            detail: Some(&detail),
        };
        assert_eq!(
            ndjson(&warning),
            json!({"event":"warning","reason":"bad ring","sourceId":"7"})
        );
    }

    #[test]
    fn custom_sinks_receive_every_event() {
        #[derive(Default)]
        struct Log(Mutex<Vec<String>>);
        impl EventSink for Log {
            fn event(&self, event: &Event<'_>) {
                self.0.lock().unwrap().push(ndjson(event).to_string());
            }
        }
        let log = Arc::new(Log::default());
        let reporter = Reporter::custom(log.clone());
        assert!(reporter.wants_progress() && !Reporter::default().wants_progress());
        reporter.progress("tiling", 0, 2);
        reporter.warn("careful", None);
        reporter.note("timing");
        assert_eq!(log.0.lock().unwrap().len(), 3);
        Reporter::silent().warn("dropped", None);
    }
}
