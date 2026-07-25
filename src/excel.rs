use std::path::Path;

/// Reads the CSV file at `path`, which should have the same format as one on this project's `data` directory,
/// and transforms it into a `.xlsx` file, with format and column changes.
///
/// Transformation:
/// - The existing timestamp and duration columns are transfomed from type string to the Excel number type for date/time.
/// - All timestamp columns use the format "YYYY-MM-DD HH:MM:SS DDD".
/// - All duration columns use the format "HH:MM:SS".
/// - A new column `Adj_conn_end` is inserted right after the `Conn_DateTime_End` column. It contains
///   `Conn_DateTime_Start + Conn_Duration` rounded up to the closest minute, minus 1 second.
/// - A new column `Adj_conn_duration` is inserted right after the `Adj_conn_end` column. It contains
///   the duration `Adj_conn_end` minus `Conn_DateTime_Start`, implemented as a formula.
/// - A new column `Avg_power` is inserted right after `Energy_Use`. It contains the energy use averaged over the
///   `Active_Charge_Time` duration, implemented as a formula.
pub fn session_csv_to_xlsx(path: &Path) {
    todo!()
}
