use std::io;

use std::collections::HashMap;

use rust_decimal::prelude::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct Transaction {
    #[serde(rename = "type")]
    tpe: TransactionType,
    client: u16,
    tx: u32,
    amount: Decimal,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum TransactionState {
    Executed,
    Disputed,
    Resolved,
    Chargebacked,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Account {
    available: Decimal,
    held: Decimal,
    locked: bool,
}

impl Account {
    fn new() -> Account {
        Account {
            available: Decimal::new(0, 0),
            held: Decimal::new(0, 0),
            locked: false,
        }
    }
    fn deposit(&mut self, amount: &Decimal) {
        self.available += amount;
    }

    fn withdraw(&mut self, amount: &Decimal) {
        self.available -= amount;
    }

    fn lock(&mut self, amount: &Decimal) {
        self.available -= amount;
        self.held += amount;
    }

    fn release(&mut self, amount: &Decimal) {
        self.held -= amount;
        self.available += amount;
    }

    fn chargeback(&mut self, amount: &Decimal) {
        self.held -= amount;
        self.locked = true;
    }
}

struct State {
    accounts: HashMap<u16, Account>,
    transaction_history: HashMap<u32, Transaction>,
    transaction_state: HashMap<u32, TransactionState>,
}

impl State {
    fn new() -> State {
        State {
            accounts: HashMap::new(),
            transaction_history: HashMap::new(),
            transaction_state: HashMap::new(),
        }
    }

    fn apply_deposit(&mut self, tx: &Transaction) {
        let entry = self
            .accounts
            .entry(tx.client)
            .or_insert_with(|| Account::new());
        entry.deposit(&tx.amount);
        self.transaction_history.insert(tx.tx, *tx);
    }

    fn apply_withdrawal(&mut self, tx: &Transaction) {
        match self.accounts.get_mut(&tx.client) {
            Some(acc) => acc.withdraw(&tx.amount),
            None => panic!("Unknown client"),
        }
        self.transaction_history.insert(tx.tx, *tx);
    }

    fn apply_dispute(&mut self, tx: &Transaction) {
        match self.transaction_history.get(&tx.tx) {
            Some(disputed_tx) => {
                let tstate = self.transaction_state.get_mut(&tx.tx).unwrap();
                assert!(*tstate == TransactionState::Executed);
                let account = self.accounts.get_mut(&tx.client).unwrap();
                account.lock(&disputed_tx.amount);
                *tstate = TransactionState::Disputed;
            }
            None => (), // Ignore
        }
    }

    fn apply_resolve(&mut self, tx: &Transaction) {
        match self.transaction_history.get(&tx.tx) {
            Some(resolved_tx) => {
                let tstate = self.transaction_state.get_mut(&tx.tx).unwrap();
                assert!(*tstate == TransactionState::Disputed);
                let account = self.accounts.get_mut(&tx.client).unwrap();
                account.release(&resolved_tx.amount);
                *tstate = TransactionState::Resolved;
            }
            None => (), // Ignore
        }
    }

    fn apply_chargeback(&mut self, tx: &Transaction) {
        match self.transaction_history.get(&tx.tx) {
            Some(chargebacked_tx) => {
                let tstate = self.transaction_state.get_mut(&tx.tx).unwrap();
                assert!(*tstate == TransactionState::Disputed);
                let account = self.accounts.get_mut(&tx.client).unwrap();
                account.chargeback(&chargebacked_tx.amount);
                *tstate = TransactionState::Chargebacked;
            }
            None => (), // Ignore
        }
    }

    fn apply_transaction(&mut self, tx: &Transaction) {
        match tx.tpe {
            TransactionType::Deposit => self.apply_deposit(tx),
            TransactionType::Withdrawal => self.apply_withdrawal(tx),
            TransactionType::Dispute => self.apply_dispute(tx),
            TransactionType::Resolve => self.apply_resolve(tx),
            TransactionType::Chargeback => self.apply_chargeback(tx),
        }
    }

    fn iter_clients<'a>(&'a self) -> impl Iterator<Item = (&'a u16, &'a Account)> {
        self.accounts.iter()
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
struct ExportedClient {
    client: u16,
    available: Decimal,
    held: Decimal,
    total: Decimal,
    locked: bool,
}

fn main() {
    let mut rdr = csv::Reader::from_reader(io::stdin());
    let mut state = State::new();

    for result in rdr.deserialize() {
        let transaction: Transaction = result.unwrap();
        println!("{:?}", transaction);
        state.apply_transaction(&transaction);
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
        wtr.serialize(client).unwrap();
    }
}
