//! Rust vector I/O and horizontal coordinate transformations, with optional GDAL.
mod codec;
pub mod crs;
#[cfg(feature = "gdal-system")]
mod gdal_backend;
#[cfg(feature = "gdal-system")]
mod gdal_read;
#[cfg(feature = "gdal-system")]
mod gdal_runtime;
mod gpkg;
mod native;
mod parquet;
pub mod read;
pub mod registry;
pub mod write;
mod xml;
pub use crs::{Crs, Transform};
