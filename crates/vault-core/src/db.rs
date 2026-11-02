//! Thin, safe wrapper over the SQLite C API.
//!
//! Only what Mothball needs: prepared statements with positional
//! parameters, typed values, transactions, and busy handling. Every statement
//! is finalized on drop; every error carries SQLite's message.

use std::ffi::{c_char, c_int, c_void, CStr, CString};
use std::path::Path;
use std::ptr;

#[allow(non_camel_case_types)]
type sqlite3 = c_void;
#[allow(non_camel_case_types)]
type sqlite3_stmt = c_void;

const SQLITE_OK: c_int = 0;
const SQLITE_ROW: c_int = 100;
const SQLITE_DONE: c_int = 101;
const SQLITE_INTEGER: c_int = 1;
const SQLITE_FLOAT: c_int = 2;
const SQLITE_TEXT: c_int = 3;
const SQLITE_BLOB: c_int = 4;
const SQLITE_OPEN_READWRITE: c_int = 0x02;
const SQLITE_OPEN_CREATE: c_int = 0x04;
const SQLITE_OPEN_FULLMUTEX: c_int = 0x0001_0000;
const SQLITE_OPEN_READONLY: c_int = 0x01;

// SQLITE_TRANSIENT tells SQLite to copy bound buffers immediately.
fn transient() -> Option<unsafe extern "C" fn(*mut c_void)> {
    // SQLITE_TRANSIENT is ((sqlite3_destructor_type)-1)
    unsafe { std::mem::transmute::<isize, Option<unsafe extern "C" fn(*mut c_void)>>(-1) }
}

extern "C" {
    fn sqlite3_open_v2(f: *const c_char, db: *mut *mut sqlite3, flags: c_int, vfs: *const c_char) -> c_int;
    fn sqlite3_close_v2(db: *mut sqlite3) -> c_int;
    fn sqlite3_errmsg(db: *mut sqlite3) -> *const c_char;
    fn sqlite3_exec(
        db: *mut sqlite3,
        sql: *const c_char,
        cb: Option<unsafe extern "C" fn(*mut c_void, c_int, *mut *mut c_char, *mut *mut c_char) -> c_int>,
        arg: *mut c_void,
        err: *mut *mut c_char,
    ) -> c_int;
    fn sqlite3_free(p: *mut c_void);
    fn sqlite3_prepare_v2(
        db: *mut sqlite3,
        sql: *const c_char,
        n: c_int,
        stmt: *mut *mut sqlite3_stmt,
        tail: *mut *const c_char,
    ) -> c_int;
    fn sqlite3_finalize(s: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_step(s: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_reset(s: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_bind_null(s: *mut sqlite3_stmt, i: c_int) -> c_int;
    fn sqlite3_bind_int64(s: *mut sqlite3_stmt, i: c_int, v: i64) -> c_int;
    fn sqlite3_bind_double(s: *mut sqlite3_stmt, i: c_int, v: f64) -> c_int;
    fn sqlite3_bind_text(
        s: *mut sqlite3_stmt,
        i: c_int,
        v: *const c_char,
        n: c_int,
        d: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    fn sqlite3_bind_blob(
        s: *mut sqlite3_stmt,
        i: c_int,
        v: *const c_void,
        n: c_int,
        d: Option<unsafe extern "C" fn(*mut c_void)>,
    ) -> c_int;
    fn sqlite3_bind_parameter_count(s: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_column_count(s: *mut sqlite3_stmt) -> c_int;
    fn sqlite3_column_name(s: *mut sqlite3_stmt, i: c_int) -> *const c_char;
    fn sqlite3_column_type(s: *mut sqlite3_stmt, i: c_int) -> c_int;
    fn sqlite3_column_int64(s: *mut sqlite3_stmt, i: c_int) -> i64;
    fn sqlite3_column_double(s: *mut sqlite3_stmt, i: c_int) -> f64;
    fn sqlite3_column_text(s: *mut sqlite3_stmt, i: c_int) -> *const u8;
    fn sqlite3_column_blob(s: *mut sqlite3_stmt, i: c_int) -> *const c_void;
    fn sqlite3_column_bytes(s: *mut sqlite3_stmt, i: c_int) -> c_int;
    fn sqlite3_changes(db: *mut sqlite3) -> c_int;
    fn sqlite3_last_insert_rowid(db: *mut sqlite3) -> i64;
    fn sqlite3_busy_timeout(db: *mut sqlite3, ms: c_int) -> c_int;
    fn sqlite3_libversion() -> *const c_char;
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Int(i64),
    Real(f64),
    Text(String),
    Blob(Vec<u8>),
}

impl Value {
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Real(f) => Some(*f as i64),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        if let Value::Text(s) = self { Some(s) } else { None }
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_string())
    }
}
impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}
impl From<&String> for Value {
    fn from(s: &String) -> Self {
        Value::Text(s.clone())
    }
}
impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Int(i)
    }
}
impl From<i32> for Value {
    fn from(i: i32) -> Self {
        Value::Int(i as i64)
    }
}
impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Int(b as i64)
    }
}
impl From<f64> for Value {
    fn from(f: f64) -> Self {
        Value::Real(f)
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(o: Option<T>) -> Self {
        o.map(Into::into).unwrap_or(Value::Null)
    }
}

/// `p![a, b, c]` builds a parameter slice.
#[macro_export]
macro_rules! p {
    () => { &[] as &[$crate::db::Value] };
    ($($x:expr),+ $(,)?) => { &[$($crate::db::Value::from($x)),+] as &[$crate::db::Value] };
}

#[derive(Debug, Clone)]
pub struct Row {
    pub cols: std::rc::Rc<Vec<String>>,
    pub vals: Vec<Value>,
}

impl Row {
    fn idx(&self, name: &str) -> usize {
        self.cols.iter().position(|c| c == name).unwrap_or_else(|| panic!("no column {name}"))
    }
    pub fn get(&self, name: &str) -> &Value {
        &self.vals[self.idx(name)]
    }
    pub fn s(&self, name: &str) -> String {
        match self.get(name) {
            Value::Text(s) => s.clone(),
            Value::Int(i) => i.to_string(),
            Value::Real(f) => f.to_string(),
            _ => String::new(),
        }
    }
    pub fn os(&self, name: &str) -> Option<String> {
        match self.get(name) {
            Value::Null => None,
            _ => Some(self.s(name)),
        }
    }
    pub fn i(&self, name: &str) -> i64 {
        self.get(name).as_i64().unwrap_or(0)
    }
    pub fn oi(&self, name: &str) -> Option<i64> {
        self.get(name).as_i64()
    }
    pub fn b(&self, name: &str) -> bool {
        self.i(name) != 0
    }
}

pub type DbResult<T> = Result<T, String>;

pub struct Db {
    raw: *mut sqlite3,
}

// The connection is opened with SQLITE_OPEN_FULLMUTEX (serialized mode), and
// the application additionally guards it with a Mutex.
unsafe impl Send for Db {}

impl Drop for Db {
    fn drop(&mut self) {
        unsafe {
            sqlite3_close_v2(self.raw);
        }
    }
}

struct Stmt {
    raw: *mut sqlite3_stmt,
}
impl Drop for Stmt {
    fn drop(&mut self) {
        unsafe {
            sqlite3_finalize(self.raw);
        }
    }
}

pub fn sqlite_version() -> String {
    unsafe { CStr::from_ptr(sqlite3_libversion()).to_string_lossy().to_string() }
}

impl Db {
    pub fn open(path: &Path) -> DbResult<Db> {
        Self::open_flags(path, SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX)
    }
    pub fn open_readonly(path: &Path) -> DbResult<Db> {
        Self::open_flags(path, SQLITE_OPEN_READONLY | SQLITE_OPEN_FULLMUTEX)
    }
    pub fn open_memory() -> DbResult<Db> {
        Self::open_flags(Path::new(":memory:"), SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX)
    }
    fn open_flags(path: &Path, flags: c_int) -> DbResult<Db> {
        let c = CString::new(path.to_string_lossy().as_bytes()).map_err(|e| e.to_string())?;
        let mut raw = ptr::null_mut();
        let rc = unsafe { sqlite3_open_v2(c.as_ptr(), &mut raw, flags, ptr::null()) };
        let db = Db { raw };
        if rc != SQLITE_OK {
            return Err(format!("cannot open database {}: {}", path.display(), db.errmsg()));
        }
        unsafe {
            sqlite3_busy_timeout(raw, 5000);
        }
        Ok(db)
    }

    fn errmsg(&self) -> String {
        if self.raw.is_null() {
            return "out of memory".into();
        }
        unsafe { CStr::from_ptr(sqlite3_errmsg(self.raw)).to_string_lossy().to_string() }
    }

    /// Executes one or more statements without parameters.
    pub fn batch(&self, sql: &str) -> DbResult<()> {
        let c = CString::new(sql).map_err(|e| e.to_string())?;
        let mut err: *mut c_char = ptr::null_mut();
        let rc = unsafe { sqlite3_exec(self.raw, c.as_ptr(), None, ptr::null_mut(), &mut err) };
        if rc != SQLITE_OK {
            let msg = if err.is_null() {
                self.errmsg()
            } else {
                let m = unsafe { CStr::from_ptr(err).to_string_lossy().to_string() };
                unsafe { sqlite3_free(err as *mut c_void) };
                m
            };
            return Err(msg);
        }
        Ok(())
    }

    fn prepare(&self, sql: &str, params: &[Value]) -> DbResult<Stmt> {
        let c = CString::new(sql).map_err(|e| e.to_string())?;
        let mut raw = ptr::null_mut();
        let rc = unsafe { sqlite3_prepare_v2(self.raw, c.as_ptr(), -1, &mut raw, ptr::null_mut()) };
        if rc != SQLITE_OK {
            return Err(format!("{} -- in: {}", self.errmsg(), sql.trim()));
        }
        let st = Stmt { raw };
        let n = unsafe { sqlite3_bind_parameter_count(raw) } as usize;
        if n != params.len() {
            return Err(format!("expected {n} params, got {} -- in: {}", params.len(), sql.trim()));
        }
        for (i, v) in params.iter().enumerate() {
            let ix = (i + 1) as c_int;
            let rc = unsafe {
                match v {
                    Value::Null => sqlite3_bind_null(raw, ix),
                    Value::Int(x) => sqlite3_bind_int64(raw, ix, *x),
                    Value::Real(x) => sqlite3_bind_double(raw, ix, *x),
                    Value::Text(s) => {
                        sqlite3_bind_text(raw, ix, s.as_ptr() as *const c_char, s.len() as c_int, transient())
                    }
                    Value::Blob(b) => {
                        sqlite3_bind_blob(raw, ix, b.as_ptr() as *const c_void, b.len() as c_int, transient())
                    }
                }
            };
            if rc != SQLITE_OK {
                return Err(self.errmsg());
            }
        }
        Ok(st)
    }

    /// Executes a statement; returns the number of changed rows.
    pub fn exec(&self, sql: &str, params: &[Value]) -> DbResult<usize> {
        let st = self.prepare(sql, params)?;
        loop {
            match unsafe { sqlite3_step(st.raw) } {
                SQLITE_DONE => break,
                SQLITE_ROW => continue,
                _ => return Err(format!("{} -- in: {}", self.errmsg(), sql.trim())),
            }
        }
        unsafe { sqlite3_reset(st.raw) };
        Ok(unsafe { sqlite3_changes(self.raw) } as usize)
    }

    pub fn last_insert_rowid(&self) -> i64 {
        unsafe { sqlite3_last_insert_rowid(self.raw) }
    }

    pub fn query(&self, sql: &str, params: &[Value]) -> DbResult<Vec<Row>> {
        let st = self.prepare(sql, params)?;
        let ncol = unsafe { sqlite3_column_count(st.raw) };
        let cols: Vec<String> = (0..ncol)
            .map(|i| unsafe { CStr::from_ptr(sqlite3_column_name(st.raw, i)).to_string_lossy().to_string() })
            .collect();
        let cols = std::rc::Rc::new(cols);
        let mut rows = Vec::new();
        loop {
            match unsafe { sqlite3_step(st.raw) } {
                SQLITE_ROW => {
                    let mut vals = Vec::with_capacity(ncol as usize);
                    for i in 0..ncol {
                        vals.push(unsafe { read_col(st.raw, i) });
                    }
                    rows.push(Row { cols: cols.clone(), vals });
                }
                SQLITE_DONE => break,
                _ => return Err(format!("{} -- in: {}", self.errmsg(), sql.trim())),
            }
        }
        Ok(rows)
    }

    pub fn one(&self, sql: &str, params: &[Value]) -> DbResult<Option<Row>> {
        Ok(self.query(sql, params)?.into_iter().next())
    }

    pub fn scalar_i64(&self, sql: &str, params: &[Value]) -> DbResult<i64> {
        Ok(self.one(sql, params)?.and_then(|r| r.vals[0].as_i64()).unwrap_or(0))
    }

    /// Runs `f` inside a transaction. Commits on Ok, rolls back on Err.
    /// Uses SAVEPOINTs so calls may nest.
    pub fn tx<T>(&self, f: impl FnOnce(&Db) -> DbResult<T>) -> DbResult<T> {
        let name = format!("sp_{}", &crate::util::new_id()[..8]);
        self.batch(&format!("SAVEPOINT {name}"))?;
        match f(self) {
            Ok(v) => {
                self.batch(&format!("RELEASE {name}"))?;
                Ok(v)
            }
            Err(e) => {
                let _ = self.batch(&format!("ROLLBACK TO {name}; RELEASE {name}"));
                Err(e)
            }
        }
    }
}

unsafe fn read_col(st: *mut sqlite3_stmt, i: c_int) -> Value {
    match sqlite3_column_type(st, i) {
        SQLITE_INTEGER => Value::Int(sqlite3_column_int64(st, i)),
        SQLITE_FLOAT => Value::Real(sqlite3_column_double(st, i)),
        SQLITE_TEXT => {
            let p = sqlite3_column_text(st, i);
            let n = sqlite3_column_bytes(st, i) as usize;
            let s = std::slice::from_raw_parts(p, n);
            Value::Text(String::from_utf8_lossy(s).to_string())
        }
        SQLITE_BLOB => {
            let n = sqlite3_column_bytes(st, i) as usize;
            if n == 0 {
                Value::Blob(Vec::new())
            } else {
                let p = sqlite3_column_blob(st, i) as *const u8;
                Value::Blob(std::slice::from_raw_parts(p, n).to_vec())
            }
        }
        _ => Value::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn basic_and_tx() {
        let db = Db::open_memory().unwrap();
        db.batch("CREATE TABLE t(a INTEGER, b TEXT, c BLOB)").unwrap();
        db.exec("INSERT INTO t VALUES (?,?,?)", p![1, "x'y", Value::Blob(vec![1, 2])]).unwrap();
        let r = db.one("SELECT * FROM t", p![]).unwrap().unwrap();
        assert_eq!(r.i("a"), 1);
        assert_eq!(r.s("b"), "x'y");
        let res: DbResult<()> = db.tx(|d| {
            d.exec("INSERT INTO t VALUES (2,'z',NULL)", p![])?;
            Err("boom".into())
        });
        assert!(res.is_err());
        assert_eq!(db.scalar_i64("SELECT count(*) FROM t", p![]).unwrap(), 1);
        assert!(db.exec("INSERT INTO nope VALUES (1)", p![]).is_err());
        assert!(db.exec("SELECT ?", p![]).is_err());
    }
}
