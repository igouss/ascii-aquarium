//! The simulation and the renderers: no I/O, no clock, no terminal. Time
//! comes in as `dt`, randomness from the pond's seeded generator, and frames
//! go out as cells.

pub mod ansi;
pub mod focus;
pub mod frame;
pub mod input;
pub mod koi;
pub mod koi_skin;
pub mod math;
pub mod pond;
pub mod raster;
pub mod scene;
pub mod shark;
pub mod shark_skin;
pub mod species;
pub mod swim;
pub mod water;
