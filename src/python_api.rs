use std::path;
use pyo3::prelude::*;

#[pyfunction]
fn read(filename: path::PathBuf) -> PyResult<String> {
    Ok(filename.into_os_string().into_string().unwrap())
}

/// A Python module implemented in Rust. The name of this function must match
/// the `lib.name` setting in the `Cargo.toml`, else Python will not be able to
/// import the module.
#[pymodule]
fn _light_speed_io(_py: Python<'_>, m: &PyModule) -> PyResult<()> {
    m.add_function(wrap_pyfunction!(read, m)?)?;
    Ok(())
}