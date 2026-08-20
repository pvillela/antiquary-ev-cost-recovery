//! What a front-end asks the library, stated in the terms a front-end has.
//!
//! The modules below are organised by the source of data they read. A caller checking an invoice
//! has none of those in hand — it has a billing period, a meter export and the charging network's
//! monthly reports — so this module is where those become the calls the modules understand, and
//! where the several error types they return become one a front-end can present.

mod common;
pub use common::*;

pub mod io;
pub mod pure;
