use std::sync::Arc;

use crate::{Error, Result, Value};

/// An owned row snapshot, independent of a SQLite cursor or pooled connection.
/// Use `get_value` for floats, blobs and NULL or to implement domain decoding.
#[derive(Clone, Debug)]
pub struct Row {
    pub(crate) columns: Arc<[String]>,
    pub(crate) values: Vec<Value>,
}

impl Row {
    /// Column names in projection order (duplicate names are preserved).
    pub fn columns(&self) -> &[String] {
        &self.columns
    }

    /// All owned values in projection order.
    pub fn values(&self) -> &[Value] {
        &self.values
    }

    /// Borrow a value; invalid column indices return an error.
    pub fn get_value(&self, index: usize) -> Result<&Value> {
        self.values.get(index).ok_or(Error::Column)
    }

    /// Borrow a TEXT value without coercing types.
    pub fn get_str(&self, index: usize) -> Result<&str> {
        match self.get_value(index)? {
            Value::Text(value) => Ok(value),
            _ => Err(Error::Column),
        }
    }

    /// Read an INTEGER value without narrowing or coercion.
    pub fn get_i64(&self, index: usize) -> Result<i64> {
        match self.get_value(index)? {
            Value::Integer(value) => Ok(*value),
            _ => Err(Error::Column),
        }
    }
}
