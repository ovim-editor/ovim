use super::{
    EventEnvelope, EventId, NewRunEvent, RunEventSink, RunId, RunLogError, RunStorageLayout,
    SqliteRunEventSink,
};
use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::sync::{Arc, Mutex};

/// Durable local run history routed to one SQLite database per run.
///
/// This is the application-facing storage boundary: callers use the ordinary
/// [`RunEventSink`] interface and need not know paths or SQLite connection
/// details. Databases and directories are created only on the first append.
pub struct LocalRunStore {
    layout: RunStorageLayout,
    sinks: Mutex<HashMap<RunId, Arc<SqliteRunEventSink>>>,
}

impl LocalRunStore {
    pub fn discover() -> Result<Self, RunLogError> {
        Ok(Self::new(RunStorageLayout::discover()?))
    }

    pub fn new(layout: RunStorageLayout) -> Self {
        Self {
            layout,
            sinks: Mutex::new(HashMap::new()),
        }
    }

    pub fn layout(&self) -> &RunStorageLayout {
        &self.layout
    }

    fn sink_for_append(&self, run_id: &RunId) -> Result<Arc<SqliteRunEventSink>, RunLogError> {
        if let Some(sink) = self.cached_sink(run_id)? {
            return Ok(sink);
        }
        self.layout.ensure_run_directory(run_id)?;
        self.open_and_cache(run_id)
    }

    fn sink_for_read(
        &self,
        run_id: &RunId,
    ) -> Result<Option<Arc<SqliteRunEventSink>>, RunLogError> {
        if let Some(sink) = self.cached_sink(run_id)? {
            return Ok(Some(sink));
        }
        let database = self.layout.event_database(run_id);
        match fs::metadata(&database) {
            Ok(metadata) if metadata.is_file() => self.open_and_cache(run_id).map(Some),
            Ok(_) => Err(RunLogError::Storage {
                operation: "open run event database".into(),
                detail: format!("{} is not a file", database.display()),
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(io_error("inspect run event database", &database, error)),
        }
    }

    fn cached_sink(&self, run_id: &RunId) -> Result<Option<Arc<SqliteRunEventSink>>, RunLogError> {
        self.sinks
            .lock()
            .map_err(|_| RunLogError::Poisoned)
            .map(|sinks| sinks.get(run_id).cloned())
    }

    fn open_and_cache(&self, run_id: &RunId) -> Result<Arc<SqliteRunEventSink>, RunLogError> {
        // Hold the cache lock through opening so first use cannot race schema
        // migration within one process. Separate processes remain coordinated
        // by SQLite's transaction and busy timeout.
        let mut sinks = self.sinks.lock().map_err(|_| RunLogError::Poisoned)?;
        if let Some(sink) = sinks.get(run_id) {
            return Ok(Arc::clone(sink));
        }
        let opened = Arc::new(SqliteRunEventSink::open(
            self.layout.event_database(run_id),
        )?);
        sinks.insert(run_id.clone(), Arc::clone(&opened));
        Ok(opened)
    }

    fn discover_disk_runs(&self) -> Result<Vec<(String, RunId)>, RunLogError> {
        let mut discovered = Vec::new();
        let entries = match fs::read_dir(self.layout.root()) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(discovered),
            Err(error) => return Err(io_error("discover local runs", self.layout.root(), error)),
        };
        for entry in entries {
            let entry = entry
                .map_err(|error| io_error("read local run directory", self.layout.root(), error))?;
            let path = entry.path();
            let Some(name) = path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            if !name.starts_with("run-") || !path.join("events.sqlite3").is_file() {
                continue;
            }
            let sink = SqliteRunEventSink::open(path.join("events.sqlite3"))?;
            let stored_runs = sink.runs()?;
            for run_id in stored_runs {
                if self.layout.run_directory(&run_id) != path {
                    return Err(RunLogError::Corruption {
                        detail: format!(
                            "run {} is stored in unexpected directory {}",
                            run_id,
                            path.display()
                        ),
                    });
                }
                let created_at = sink
                    .events(&run_id)?
                    .into_iter()
                    .next()
                    .ok_or_else(|| RunLogError::Corruption {
                        detail: format!("run {run_id} has no initial event"),
                    })?
                    .recorded_at;
                discovered.push((created_at, run_id));
            }
        }
        Ok(discovered)
    }
}

impl RunEventSink for LocalRunStore {
    fn append(&self, event: NewRunEvent) -> Result<EventEnvelope, RunLogError> {
        let sink = self.sink_for_append(&event.run_id)?;
        sink.append(event)
    }

    fn append_if_last(
        &self,
        event: NewRunEvent,
        expected_last_sequence: Option<u64>,
    ) -> Result<EventEnvelope, RunLogError> {
        // Delegate so the guard stays atomic inside the SQLite transaction;
        // the default check-then-append would reopen the race between
        // cooperating processes.
        let sink = self.sink_for_append(&event.run_id)?;
        sink.append_if_last(event, expected_last_sequence)
    }

    fn event(
        &self,
        run_id: &RunId,
        event_id: &EventId,
    ) -> Result<Option<EventEnvelope>, RunLogError> {
        match self.sink_for_read(run_id)? {
            Some(sink) => sink.event(run_id, event_id),
            None => Ok(None),
        }
    }

    fn events(&self, run_id: &RunId) -> Result<Vec<EventEnvelope>, RunLogError> {
        match self.sink_for_read(run_id)? {
            Some(sink) => sink.events(run_id),
            None => Ok(Vec::new()),
        }
    }

    fn last_sequence(&self, run_id: &RunId) -> Result<Option<u64>, RunLogError> {
        match self.sink_for_read(run_id)? {
            Some(sink) => sink.last_sequence(run_id),
            None => Ok(None),
        }
    }

    fn runs(&self) -> Result<Vec<RunId>, RunLogError> {
        let mut runs = self.discover_disk_runs()?;
        runs.sort();
        Ok(runs.into_iter().map(|(_, run_id)| run_id).collect())
    }
}

fn io_error(operation: &str, path: &Path, error: std::io::Error) -> RunLogError {
    RunLogError::Storage {
        operation: operation.into(),
        detail: format!("{}: {error}", path.display()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_log::{EventActor, EventKind, RunLifecycleEvent, RunLifecycleState};

    fn created(run_id: RunId) -> NewRunEvent {
        NewRunEvent::new(
            run_id,
            EventActor::User,
            EventKind::RunLifecycle(RunLifecycleEvent {
                state: RunLifecycleState::Created,
                objective: Some("persist this run".into()),
                detail: None,
                creation: None,
            }),
        )
    }

    #[test]
    fn append_is_lazy_and_reopens_from_the_same_layout() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(temporary.path().join("runs"));
        let run_id = RunId::new();
        let store = LocalRunStore::new(layout.clone());
        assert!(!layout.root().exists());

        let first = store.append(created(run_id.clone())).unwrap();
        drop(store);
        let reopened = LocalRunStore::new(layout);

        assert_eq!(reopened.events(&run_id).unwrap(), vec![first]);
        assert_eq!(reopened.last_sequence(&run_id).unwrap(), Some(1));
    }

    #[test]
    fn routes_independent_runs_to_independent_databases() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(temporary.path().join("runs"));
        let store = LocalRunStore::new(layout.clone());
        let first_run = RunId::new();
        let second_run = RunId::new();

        store.append(created(first_run.clone())).unwrap();
        store.append(created(second_run.clone())).unwrap();
        store.append(created(first_run.clone())).unwrap();

        assert_ne!(
            layout.event_database(&first_run),
            layout.event_database(&second_run)
        );
        assert_eq!(store.last_sequence(&first_run).unwrap(), Some(2));
        assert_eq!(store.last_sequence(&second_run).unwrap(), Some(1));
        assert_eq!(store.runs().unwrap(), vec![first_run, second_run]);
    }

    #[test]
    fn missing_run_reads_do_not_create_storage() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(temporary.path().join("runs"));
        let store = LocalRunStore::new(layout.clone());

        assert!(store.events(&RunId::new()).unwrap().is_empty());
        assert!(!layout.root().exists());
    }

    #[test]
    fn concurrent_first_use_opens_and_migrates_once() {
        let temporary = tempfile::tempdir().unwrap();
        let layout = RunStorageLayout::new(temporary.path().join("runs"));
        let store = Arc::new(LocalRunStore::new(layout));
        let run_id = RunId::new();
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let run_id = run_id.clone();
                std::thread::spawn(move || store.append(created(run_id)).unwrap())
            })
            .collect();

        for worker in workers {
            worker.join().unwrap();
        }
        assert_eq!(store.events(&run_id).unwrap().len(), 8);
        assert_eq!(store.last_sequence(&run_id).unwrap(), Some(8));
    }
}
