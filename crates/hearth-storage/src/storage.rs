//! Session-facing storage boundary: one writer thread and a read pool.

use std::path::Path;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Condvar, Mutex};
use std::thread::{self, JoinHandle};

use rusqlite::Connection;

use crate::{StorageError, open, open_read_only};

type Job = Box<dyn FnOnce(&mut Connection) + Send>;

/// The durable storage boundary. One writer connection runs on a dedicated
/// thread; four read-only connections serve readers.
pub struct Storage {
    sender: Mutex<Option<Sender<Job>>>,
    readers: Mutex<Vec<Connection>>,
    reader_ready: Condvar,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Storage {
    /// Open the database, apply migrations, and start the writer and readers.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let connection = open(path)?;
        let mut readers = Vec::with_capacity(4);
        for _ in 0..4 {
            let reader = open_read_only(path)?;
            reader.pragma_update(None, "busy_timeout", 5000)?;
            readers.push(reader);
        }

        let (sender, receiver) = mpsc::channel::<Job>();
        let thread = thread::spawn(move || writer_loop(receiver, connection));
        Ok(Self {
            sender: Mutex::new(Some(sender)),
            readers: Mutex::new(readers),
            reader_ready: Condvar::new(),
            thread: Mutex::new(Some(thread)),
        })
    }

    /// Run one closure inside one transaction on the writer thread.
    pub fn write<T, E, F>(&self, run: F) -> Result<T, E>
    where
        T: Send + 'static,
        E: From<StorageError> + Send + 'static,
        F: for<'a> FnOnce(&Writer<'a>) -> Result<T, E> + Send + 'static,
    {
        let (done, result) = mpsc::channel::<Result<T, E>>();
        let job: Job = Box::new(move |connection| {
            let outcome = run_transaction::<T, E, F>(connection, run);
            let _ = done.send(outcome);
        });

        {
            let sender = self
                .sender
                .lock()
                .map_err(|_| E::from(StorageError::Poisoned))?;
            let sender = sender
                .as_ref()
                .ok_or_else(|| E::from(StorageError::Closed))?;
            sender
                .send(job)
                .map_err(|_| E::from(StorageError::Closed))?;
        }

        result.recv().map_err(|_| E::from(StorageError::Closed))?
    }

    /// Run a closure on one pooled read-only connection.
    pub fn read<T, F>(&self, run: F) -> Result<T, StorageError>
    where
        F: FnOnce(&Reader<'_>) -> Result<T, StorageError>,
    {
        let mut readers = self.readers.lock().map_err(|_| StorageError::Poisoned)?;
        while readers.is_empty() {
            readers = self
                .reader_ready
                .wait(readers)
                .map_err(|_| StorageError::Poisoned)?;
        }
        let connection = readers.pop().expect("the pool is non-empty");
        drop(readers);
        let outcome = run(&Reader {
            connection: &connection,
        });
        if let Ok(mut readers) = self.readers.lock() {
            readers.push(connection);
            self.reader_ready.notify_one();
        }
        outcome
    }

    /// Read the schema version from a pooled read connection.
    pub fn schema_version(&self) -> Result<i64, StorageError> {
        self.read(|reader| reader.query_row("PRAGMA user_version", [], |row| row.get(0)))
    }
}

impl Drop for Storage {
    fn drop(&mut self) {
        if let Ok(mut sender) = self.sender.lock() {
            sender.take();
        }
        if let Ok(mut thread) = self.thread.lock()
            && let Some(handle) = thread.take()
        {
            let _ = handle.join();
        }
    }
}

fn writer_loop(receiver: Receiver<Job>, mut connection: Connection) {
    while let Ok(job) = receiver.recv() {
        job(&mut connection);
    }
}

fn run_transaction<T, E, F>(connection: &mut Connection, run: F) -> Result<T, E>
where
    E: From<StorageError>,
    F: for<'a> FnOnce(&Writer<'a>) -> Result<T, E>,
{
    let transaction = connection
        .transaction()
        .map_err(|error| E::from(StorageError::from(error)))?;
    let value = {
        let writer = Writer {
            transaction: &transaction,
        };
        run(&writer)?
    };
    crate::crash_if("before_commit");
    transaction
        .commit()
        .map_err(|error| E::from(StorageError::from(error)))?;
    crate::crash_if("after_commit");
    Ok(value)
}

/// A write view of the open transaction.
pub struct Writer<'a> {
    transaction: &'a rusqlite::Transaction<'a>,
}

impl Writer<'_> {
    /// Execute a statement.
    pub fn execute(&self, sql: &str, params: impl rusqlite::Params) -> Result<usize, StorageError> {
        Ok(self.transaction.execute(sql, params)?)
    }

    /// Query exactly one row.
    pub fn query_row<T, P, F>(&self, sql: &str, params: P, map: F) -> Result<T, StorageError>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        Ok(self.transaction.query_row(sql, params, map)?)
    }

    /// Query zero or one row.
    pub fn query_optional<T, P, F>(
        &self,
        sql: &str,
        params: P,
        map: F,
    ) -> Result<Option<T>, StorageError>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        optional(self.transaction.query_row(sql, params, map))
    }

    /// Query many rows.
    pub fn query<T, P, F>(&self, sql: &str, params: P, mut map: F) -> Result<Vec<T>, StorageError>
    where
        P: rusqlite::Params,
        F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        let mut statement = self.transaction.prepare(sql)?;
        let rows = statement.query_map(params, |row| map(row))?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row?);
        }
        Ok(values)
    }

    /// The rowid of the most recent successful insert.
    pub fn last_insert_rowid(&self) -> i64 {
        self.transaction.last_insert_rowid()
    }
}

/// A read-only view of a pooled connection.
pub struct Reader<'a> {
    connection: &'a Connection,
}

impl Reader<'_> {
    /// Query exactly one row.
    pub fn query_row<T, P, F>(&self, sql: &str, params: P, map: F) -> Result<T, StorageError>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        Ok(self.connection.query_row(sql, params, map)?)
    }

    /// Query zero or one row.
    pub fn query_optional<T, P, F>(
        &self,
        sql: &str,
        params: P,
        map: F,
    ) -> Result<Option<T>, StorageError>
    where
        P: rusqlite::Params,
        F: FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        optional(self.connection.query_row(sql, params, map))
    }

    /// Query many rows.
    pub fn query<T, P, F>(&self, sql: &str, params: P, mut map: F) -> Result<Vec<T>, StorageError>
    where
        P: rusqlite::Params,
        F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
    {
        let mut statement = self.connection.prepare(sql)?;
        let rows = statement.query_map(params, |row| map(row))?;
        let mut values = Vec::new();
        for row in rows {
            values.push(row?);
        }
        Ok(values)
    }
}

fn optional<T>(result: rusqlite::Result<T>) -> Result<Option<T>, StorageError> {
    match result {
        Ok(value) => Ok(Some(value)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(error) => Err(error.into()),
    }
}
