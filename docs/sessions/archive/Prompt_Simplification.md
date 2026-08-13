# Simplification Update Instructions

The specs and core code have been drastically simplified due to the reality that peak values used for billing are actually averages over 15-minute intervals.

See the revised @README.md specs.

An electrical engineering focused module @src/site_load.rs has been added. (That module is a companion to the document `docs/ev-charger-power-factor-and-kva-allocation.md`.) Some portions of that module are used by the core code. There is a companion binary that prints a table produced with the module.

Some tests have been deleted or commented-out to enable the core code to compile.

Critically review the revised `README.md` specs and all source code, identify any corrections and additiona needed, and prepare an implementation plan.
