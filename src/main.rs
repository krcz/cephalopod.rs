use std::io;

use rust_decimal::prelude::*;
use serde::{Deserialize, Serialize};

use log::{error, info, warn};

pub mod model;
#[cfg(test)]
mod tests;

use model::{CephalopodError, State};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct ExportedClient {
    client: u16,
    available: Decimal,
    held: Decimal,
    total: Decimal,
    locked: bool,
}

fn main() -> Result<(), String> {
    pretty_env_logger::init();

    let args: Vec<String> = std::env::args().collect();

    if args.len() != 2 {
        println!("Usage: {} transactions.csv", args[0]);
        return Err(format!("Usage: {} transactions.csv", args[0]));
    }

    let ref path = args[1];

    let mut rdr = csv::Reader::from_path(path).map_err(|err| {
        error!("Problem opening input file: {}", err);
        format!("Problem opening input file: {}", err)
    })?;
    let mut state = State::new();

    for result in rdr.deserialize() {
        if let Ok(transaction) =
            result.map_err(|err| warn!("Ignoring input row because of parse error: {}.", err))
        {
            info!("Processing transaction {:?}", transaction);
            state.apply_transaction(&transaction).or_else(|err| {
                match err {
                    CephalopodError::TransactionError { transaction, error } => {
                        warn!("Error while processing transaction {}: {}. Transaction has not been applied.", transaction.tx, error);
                        Ok(())
                    }
                    CephalopodError::IntegrityError { transaction, error } => {
                        error!("Integrity error while processing transaction {}: {}. Ending processing.", transaction.tx, error);
                        Err(format!("{}", error))
                    }
                }
            })?;
        }
    }

    let mut wtr = csv::Writer::from_writer(io::stdout());

    for (&id, &account) in state.iter_clients() {
        let client = ExportedClient {
            client: id,
            available: account.available,
            held: account.held,
            total: account.available + account.held,
            locked: account.locked,
        };
        wtr.serialize(client).unwrap_or_else(|err| {
            error!("Error serializing record: {}", err);
            ()
        })
    }

    Ok(())
}
