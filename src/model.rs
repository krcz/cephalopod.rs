use std::collections::HashMap;

use rust_decimal::prelude::*;
use serde::{Deserialize, Serialize};

use thiserror::Error;

#[derive(Debug, Clone, Copy)]
pub enum AccountError {
    AccountLocked,
    NotEnoughFunds {
        available: Decimal,
        required: Decimal,
    },
    NegativeAmount {
        amount: Decimal,
    },
}

/// Error type representing some problem with the input data
///
/// Such errors should be logged and ignored
#[derive(Error, Debug, Clone, Copy)]
pub enum TransactionError {
    #[error("account {client} is locked")]
    AccountLocked { client: u16 },

    #[error("amount not provided")]
    AmountNotProvided,

    #[error("amount not provided")]
    NegativeAmountProvided { amount: Decimal },

    #[error("unknown account: {client}")]
    UnknownAccount { client: u16 },

    #[error("not enough funds, available: {available}, required: {required}")]
    NotEnoughFunds {
        available: Decimal,
        required: Decimal,
    },

    #[error("requested dispute of unknown transaction: {tx}")]
    TransactionNotFound { tx: u32 },

    #[error("transaction in wrong state: {state:?}")]
    TransactionInvalidState { state: TransactionState },

    #[error("referenced transaction doesn't match provided client")]
    TransactionClientMismatch { tx: u32, client: u16 },
}

/// Error type representing major problem with the code
///
/// Such errors should never occur. If it happens, the application should stop
/// immediately as it is a symptom of data corruption or application error.
#[derive(Error, Debug, Clone, Copy)]
pub enum IntegrityError {
    #[error("state unavailable for transaction {tx}")]
    StateMissingForTransaction { tx: u32 },

    #[error("amount information not available for {tx}")]
    AmountMissingForTransaction { tx: u32 },

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
pub enum CephalopodError {
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
pub enum TransactionType {
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
pub struct Transaction {
    #[serde(rename = "type")]
    pub tpe: TransactionType,
    pub client: u16,
    pub tx: u32,
    pub amount: Option<Decimal>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TransactionState {
    Withdrawn,
    Deposited,
    Disputed,
    Resolved,
    Chargebacked,
}

/// Representation of a client's account state
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Account {
    /// Funds available to withdrawals
    pub available: Decimal,
    /// Funds locked for disputes
    pub held: Decimal,
    /// Whether or not the account is locked
    pub locked: bool,
}

impl Account {
    pub fn new() -> Account {
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
        if amount < &Decimal::ZERO {
            Err(AccountError::NegativeAmount { amount: *amount })?;
        }

        self.available += amount;
        Ok(())
    }

    fn withdraw(&mut self, amount: &Decimal) -> Result<(), AccountError> {
        self.check_lock()?;
        if amount < &Decimal::ZERO {
            Err(AccountError::NegativeAmount { amount: *amount })?;
        }

        if amount > &self.available {
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
        if amount > &self.available {
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
        if amount > &self.held {
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
        if amount > &self.held {
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

/// Representation of system state
///
/// Stores information of all accounts and past transactions
pub struct State {
    /// Mapping from client's id to their account state
    pub(crate) accounts: HashMap<u16, Account>,
    /// Mapping from transaction id to original transaction (i.e. withdrawal or deposit)
    transaction_history: HashMap<u32, Transaction>,
    /// Mapping from transaction id to transaction state that might me affected by disputes
    transaction_state: HashMap<u32, TransactionState>,
}

impl State {
    pub fn new() -> State {
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

    fn assert_client_match(
        tx: &Transaction,
        referenced_tx: &Transaction,
    ) -> Result<(), CephalopodError> {
        if tx.client != referenced_tx.client {
            Err(CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::TransactionClientMismatch {
                    tx: tx.tx,
                    client: tx.client,
                },
            })
        } else {
            Ok(())
        }
    }

    fn assert_state(
        tx: &Transaction,
        state: &TransactionState,
        expected: TransactionState,
    ) -> Result<(), CephalopodError> {
        if *state != expected {
            Err(CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::TransactionInvalidState { state: *state },
            })
        } else {
            Ok(())
        }
    }

    fn get_amount(tx: &Transaction) -> Result<Decimal, CephalopodError> {
        tx.amount.ok_or_else(|| CephalopodError::IntegrityError {
            transaction: tx.clone(),
            error: IntegrityError::AmountMissingForTransaction { tx: tx.tx },
        })
    }

    fn apply_deposit(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
        let entry = self
            .accounts
            .entry(tx.client)
            .or_insert_with(|| Account::new());

        let amount = tx.amount.ok_or_else(|| CephalopodError::TransactionError {
            transaction: tx.clone(),
            error: TransactionError::AmountNotProvided,
        })?;

        entry.deposit(&amount).map_err(|err| match err {
            AccountError::AccountLocked => CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::AccountLocked { client: tx.client },
            },
            AccountError::NegativeAmount { amount } => CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::NegativeAmountProvided { amount },
            },
            _ => CephalopodError::IntegrityError {
                transaction: tx.clone(),
                error: IntegrityError::UnexpectedAccountError { error: err },
            },
        })?;
        self.transaction_history.insert(tx.tx, *tx);
        self.transaction_state
            .insert(tx.tx, TransactionState::Deposited);
        Ok(())
    }

    fn apply_withdrawal(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
        let account =
            self.accounts
                .get_mut(&tx.client)
                .ok_or_else(|| CephalopodError::TransactionError {
                    transaction: tx.clone(),
                    error: TransactionError::UnknownAccount { client: tx.client },
                })?;

        let amount = tx.amount.ok_or_else(|| CephalopodError::TransactionError {
            transaction: tx.clone(),
            error: TransactionError::AmountNotProvided,
        })?;

        account.withdraw(&amount).map_err(|err| match err {
            AccountError::AccountLocked => CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::AccountLocked { client: tx.client },
            },
            AccountError::NegativeAmount { amount } => CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::NegativeAmountProvided { amount },
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
        self.transaction_state
            .insert(tx.tx, TransactionState::Withdrawn);
        Ok(())
    }

    fn apply_dispute(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
        match self.transaction_history.get(&tx.tx) {
            Some(disputed_tx) => {
                let tstate = Self::get_mut_state(&mut self.transaction_state, tx)?;
                Self::assert_state(tx, tstate, TransactionState::Deposited)?;
                Self::assert_client_match(tx, disputed_tx)?;
                let account = Self::get_mut_account(&mut self.accounts, tx)?;
                account
                    .lock(&Self::get_amount(disputed_tx)?)
                    .map_err(|err| match err {
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
                        _ => CephalopodError::IntegrityError {
                            transaction: tx.clone(),
                            error: IntegrityError::UnexpectedAccountError { error: err },
                        },
                    })?;
                *tstate = TransactionState::Disputed;
                Ok(())
            }
            None => Err(CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::TransactionNotFound { tx: tx.tx },
            }),
        }
    }

    fn apply_resolve(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
        match self.transaction_history.get(&tx.tx) {
            Some(resolved_tx) => {
                let tstate = Self::get_mut_state(&mut self.transaction_state, tx)?;
                Self::assert_state(tx, tstate, TransactionState::Disputed)?;
                Self::assert_client_match(tx, resolved_tx)?;
                let account = Self::get_mut_account(&mut self.accounts, tx)?;
                account
                    .release(&Self::get_amount(resolved_tx)?)
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
                        _ => CephalopodError::IntegrityError {
                            transaction: tx.clone(),
                            error: IntegrityError::UnexpectedAccountError { error: err },
                        },
                    })?;
                *tstate = TransactionState::Resolved;
                Ok(())
            }
            None => Err(CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::TransactionNotFound { tx: tx.tx },
            }),
        }
    }

    fn apply_chargeback(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
        match self.transaction_history.get(&tx.tx) {
            Some(chargebacked_tx) => {
                let tstate = Self::get_mut_state(&mut self.transaction_state, tx)?;
                Self::assert_state(tx, tstate, TransactionState::Disputed)?;
                Self::assert_client_match(tx, chargebacked_tx)?;
                let account = Self::get_mut_account(&mut self.accounts, tx)?;
                account
                    .chargeback(&Self::get_amount(chargebacked_tx)?)
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
                        _ => CephalopodError::IntegrityError {
                            transaction: tx.clone(),
                            error: IntegrityError::UnexpectedAccountError { error: err },
                        },
                    })?;
                *tstate = TransactionState::Chargebacked;
                Ok(())
            }
            None => Err(CephalopodError::TransactionError {
                transaction: tx.clone(),
                error: TransactionError::TransactionNotFound { tx: tx.tx },
            }),
        }
    }

    /// Applies a transaction to the state
    ///
    /// If error is returned it means that the transaction has not been applied
    pub fn apply_transaction(&mut self, tx: &Transaction) -> Result<(), CephalopodError> {
        match tx.tpe {
            TransactionType::Deposit => self.apply_deposit(tx),
            TransactionType::Withdrawal => self.apply_withdrawal(tx),
            TransactionType::Dispute => self.apply_dispute(tx),
            TransactionType::Resolve => self.apply_resolve(tx),
            TransactionType::Chargeback => self.apply_chargeback(tx),
        }
    }

    /// Iterates over all the accounts in the state
    pub fn iter_clients<'a>(&'a self) -> impl Iterator<Item = (&'a u16, &'a Account)> {
        self.accounts.iter()
    }
}
