use std::io;

use rust_decimal::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum TransactionType {
    Deposit,
    Withdrawal,
    Dispute,
    Resolve,
    Chargeback,
}

// NOTE: normally I'd choose to represent it as tagged enum,
// but the csv crate doesn't support it correctly:
// https://github.com/BurntSushi/rust-csv/issues/211
// in order to skip implementing manual parsing I've opted for alternative representation
#[derive(Debug, Serialize, Deserialize)]
struct Transaction {
    #[serde(rename = "type")]
    tpe: TransactionType,
    client: u16,
    tx: u32,
    amount: Decimal,
}

fn main() {
    let mut rdr = csv::Reader::from_reader(io::stdin());

    for result in rdr.deserialize() {
        let transaction: Transaction = result.unwrap();
        println!("{:?}", transaction);
    }
}
