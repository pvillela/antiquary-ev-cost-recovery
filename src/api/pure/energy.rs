use crate::{
    hydro_bill::HydroBill,
    session::{RSession, TouKwh},
};

pub enum EnergyError {}

/// Energy consumption by TOU attributable EV charging sessions.
pub fn energy(hydro_bill: &HydroBill, sessions: &[RSession]) -> Result<TouKwh, EnergyError> {
    todo!()
}
