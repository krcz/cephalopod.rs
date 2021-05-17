use std::io;

use std::collections::HashMap;

use rust_decimal::prelude::*;
use serde::{Deserialize, Serialize};

use log::{error, info, warn};

use thiserror::Error;

#[derive(Debug, Clone, Copy)]
enum AccountError {
    AccountLocked,
    NotEnoughFunds {
        available: Decimal,
        required: Decimal,
    },
}

#[derive(Error, Debug, Clone, Copy)]
enum TransactionError {
    #[error("account {client} is locked")]
    AccountLocked { client: u16 },

    #[error("not enough funds, available: {available}, required: {required}")]
    NotEnoughFunds {
        available: Decimal,
        required: Decimal,
    },

    #[error("requested dispute of unknown transaction: {tx}")]
    DisputedTransactionNotFound { tx: u32 },

    #[error("transaction in wrong state: {state:?}")]
    DisputedTransactionInvalidState { state: TransactionState },
}

#[derive(Error, Debug, Clone, Copy)]
enum IntegrityError {
    #[error("state unavailable for transaction {tx}")]
    StateMissingForTransaction { tx: u32 },

    #[error("account information not available for {client}")]
    AccountMissingForTransaction { client: u16 },

    #[error("required funds are not locked, available: {available}, required: {required}")]
    FundsNotLocked {
        available: Decimal,
        required: Decimal,
    },

    #[error("unexpected account error during processing: {error:?}")]
    UnexpectedAccountError { error: AccountError },
}

#[derive(Error, Debug, Clone, Copy)]
enum CephalopodError {
    #[error("error during processing transaction")]
    TransactionError {
        transaction: Transaction,
        #[source]
        error: TransactionError,
    },
    #[error("integrity error during processing transaction")]
    IntegrityError {
        transaction: Transaction,
        #[source]
        error: IntegrityError,
    },
}

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

    fn check_lock(&self) -> Result<(), AccountError> {
        if self.locked {
            Err(AccountError::AccountLocked)?;
        }
        Ok(())
    }

    fn deposit(&mut self, amount: &Decimal) -> Result<(), AccountError> {
        self.check_lock()?;
        self.available += amount;
        Ok(())
    }

    fn withdraw(&mut self, amount: &Decimal) -> Result<(), AccountError> {
        self.check_lock()?;
        if amount < &self.available {
            Err(AccountError::NotEnoughFunds {
                available: self.available,
                required: *amount,
            })?;
        }
        self.available -= amount;
        Ok(())
    }

    fn lock(&mut self, amount: &Decimal) -> Result<(), AccountError> {
        self.check_lock()?;
        if amount < &self.available {
            Err(AccountError::NotEnoughFunds {
                available: self.available,
                required: *amount,
            })?;
        }
        self.available -= amount;
        self.held += amount;
        Ok(())
    }

    fn release(&mut self, amount: &Decimal) -> Result<(), AccountError> {
        self.check_lock()?;
        if amount < &self.available {
            Err(AccountError::NotEnoughFunds {
                available: self.available,
                required: *amount,
            })?;
        }
        self.held -= amount;
        self.available += amount;
        Ok(())
    }

    fn chargeback(&mut self, amount: &Decimal) -> Result<(), AccountError> {
        self.check_lock()?;
        if amount < &self.available {
            Err(AccountError::NotEnoughFunds {
                available: self.available,
                required: *amount,
            })?;
        }
        self.held -= amount;
        self.locked = true;
        Ok(())
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

    fn get_mut_state<'a>(
        data: &'a mut HashMap<u32, TransactionState>,
        tx: &Transaction,
    ) -> Result<&'a mut TransactionState, CephalopodError> {
        data.get_mut(&tx.tx)
            .ok_or_else(|| CephalopodError::IntegrityError {
                transaction: tx.clone(),
                error: IntegrityError::StateMissingForTransaction { tx: tx.tx },
            })
    }

    fn get_mut_account<'a>(
        data: &'a mut HashMap<u16, Account>,
        tx: &Transaction,
    ) -> Result<&'a mut Account, CephalopodError> {
        data.get_mut(&tx.client)
            .ok_or_else(|| CephalopodError::IntegrityError {
                transaction: tx.clone(),
                error: IntegrityError::AccountMissingForTransaction { client: tx.client },
            })
    }

    fn assert_state(
        tx: &Transaction,
        state: &TransactionState,
        expected: TransactionState,
    ) -> Result<(), CephalopodError> {
        if *state != expected {
            Err(CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::DisputedTransactionInvalidState { state: *state },
            })
        } else {
            Ok(())
        }
    }

    fn apply_deposit(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
        let entry = self
            .accounts
            .entry(tx.client)
            .or_insert_with(|| Account::new());
        entry.deposit(&tx.amount).map_err(|err| match err {
            AccountError::AccountLocked => CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::AccountLocked { client: tx.client },
            },
            _ => CephalopodError::IntegrityError {
                transaction: tx.clone(),
                error: IntegrityError::UnexpectedAccountError { error: err },
            },
        })?;
        self.transaction_history.insert(tx.tx, *tx);
        Ok(())
    }

    fn apply_withdrawal(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
        let account = Self::get_mut_account(&mut self.accounts, tx)?;
        account.withdraw(&tx.amount).map_err(|err| match err {
            AccountError::AccountLocked => CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::AccountLocked { client: tx.client },
            },
            AccountError::NotEnoughFunds {
                available,
                required,
            } => CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::NotEnoughFunds {
                    available,
                    required,
                },
            },
        })?;
        self.transaction_history.insert(tx.tx, *tx);
        Ok(())
    }

    fn apply_dispute(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
        match self.transaction_history.get(&tx.tx) {
            Some(disputed_tx) => {
                let tstate = Self::get_mut_state(&mut self.transaction_state, tx)?;
                Self::assert_state(tx, tstate, TransactionState::Executed)?;
                let account = Self::get_mut_account(&mut self.accounts, tx)?;
                account.lock(&disputed_tx.amount).map_err(|err| match err {
                    AccountError::AccountLocked => CephalopodError::TransactionError {
                        transaction: tx.clone(),
                        error: TransactionError::AccountLocked { client: tx.client },
                    },
                    AccountError::NotEnoughFunds {
                        available,
                        required,
                    } => CephalopodError::TransactionError {
                        transaction: tx.clone(),
                        error: TransactionError::NotEnoughFunds {
                            available,
                            required,
                        },
                    },
                })?;
                *tstate = TransactionState::Disputed;
                Ok(())
            }
            None => Err(CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::DisputedTransactionNotFound { tx: tx.tx },
            }),
        }
    }

    fn apply_resolve(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
        match self.transaction_history.get(&tx.tx) {
            Some(resolved_tx) => {
                let tstate = Self::get_mut_state(&mut self.transaction_state, tx)?;
                Self::assert_state(tx, tstate, TransactionState::Disputed)?;
                let account = Self::get_mut_account(&mut self.accounts, tx)?;
                account
                    .release(&resolved_tx.amount)
                    .map_err(|err| match err {
                        AccountError::AccountLocked => CephalopodError::TransactionError {
                            transaction: tx.clone(),
                            error: TransactionError::AccountLocked { client: tx.client },
                        },
                        AccountError::NotEnoughFunds {
                            available,
                            required,
                        } => CephalopodError::IntegrityError {
                            transaction: tx.clone(),
                            error: IntegrityError::FundsNotLocked {
                                available,
                                required,
                            },
                        },
                    })?;
                *tstate = TransactionState::Resolved;
                Ok(())
            }
            None => Err(CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::DisputedTransactionNotFound { tx: tx.tx },
            }),
        }
    }

    fn apply_chargeback(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
        match self.transaction_history.get(&tx.tx) {
            Some(chargebacked_tx) => {
                let tstate = Self::get_mut_state(&mut self.transaction_state, tx)?;
                Self::assert_state(tx, tstate, TransactionState::Disputed)?;
                let account = Self::get_mut_account(&mut self.accounts, tx)?;
                account
                    .chargeback(&chargebacked_tx.amount)
                    .map_err(|err| match err {
                        AccountError::AccountLocked => CephalopodError::TransactionError {
                            transaction: tx.clone(),
                            error: TransactionError::AccountLocked { client: tx.client },
                        },
                        AccountError::NotEnoughFunds {
                            available,
                            required,
                        } => CephalopodError::IntegrityError {
                            transaction: tx.clone(),
                            error: IntegrityError::FundsNotLocked {
                                available,
                                required,
                            },
                        },
                    })?;
                *tstate = TransactionState::Chargebacked;
                Ok(())
            }
            None => Err(CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::DisputedTransactionNotFound { tx: tx.tx },
            }),
        }
    }

    fn apply_transaction(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
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

fn main() -> Result<(), String> {
    pretty_env_logger::init();

    let mut rdr = csv::Reader::from_reader(io::stdin());
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
