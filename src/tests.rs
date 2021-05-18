use super::model::{
    Account, CephalopodError, State, Transaction, TransactionError, TransactionType,
};

use assert_matches::assert_matches;
use rust_decimal::prelude::*;

// runs all transactions and returns the final state and the Result of the last one
// fails if one of previous transactions fails
fn run_transactions(tx: Vec<Transaction>) -> (State, Result<(), CephalopodError>) {
    let mut state = State::new();
    let last_result = tx
        .iter()
        .map(|tx| state.apply_transaction(tx))
        .fold(Ok(()), |prev, cur| {
            prev.unwrap();
            cur
        });
    (state, last_result)
}

// creates Decimal with value amount * 0.01
fn dec(amount: i64) -> Decimal {
    Decimal::new(amount, 2)
}

fn tx0(tpe: TransactionType, client: u16, tx: u32) -> Transaction {
    Transaction {
        tpe,
        client,
        tx,
        amount: None,
    }
}

fn tx(tpe: TransactionType, client: u16, tx: u32, amount: i64) -> Transaction {
    Transaction {
        tpe,
        client,
        tx,
        amount: Some(dec(amount)),
    }
}

#[test]
fn withdrawals_and_deposits_should_work() {
    let sequence: Vec<i64> = vec![100, 200, -190, 1000, -800, -20, 200, -490];

    let (state, _) = run_transactions(
        sequence
            .iter()
            .enumerate()
            .map(|(i, am)| -> Transaction {
                if *am < 0 {
                    tx(TransactionType::Withdrawal, 1, i as u32, -*am)
                } else {
                    tx(TransactionType::Deposit, 1, i as u32, *am)
                }
            })
            .collect(),
    );

    assert_eq!(
        state.accounts.get(&1).map(|acc| acc.available),
        Some(dec(sequence.iter().sum()))
    );
}

#[test]
fn empty_amount_deposits_withdrawals_should_fail() {
    for tpe in vec![TransactionType::Deposit, TransactionType::Withdrawal] {
        let (_, res) = run_transactions(vec![
            tx(TransactionType::Deposit, 1, 1, 100),
            tx0(tpe, 1, 2),
        ]);
        assert_matches!(res, Err(CephalopodError::TransactionError { .. }));
    }
}

#[test]
fn negative_amount_deposits_withdrawals_should_fail() {
    for tpe in vec![TransactionType::Deposit, TransactionType::Withdrawal] {
        let (_, res) = run_transactions(vec![
            tx(TransactionType::Deposit, 1, 1, 100),
            tx(tpe, 1, 2, -20),
        ]);
        assert_matches!(res, Err(CephalopodError::TransactionError { .. }));
    }
}

#[test]
fn withdrawal_should_fail_for_unknown_account() {
    let (_, res) = run_transactions(vec![tx(TransactionType::Withdrawal, 1, 1, 100)]);

    assert_matches!(
        res,
        Err(CephalopodError::TransactionError {
            error: TransactionError::UnknownAccount { .. },
            ..
        })
    )
}

#[test]
fn withdrawal_should_fail_on_low_funds() {
    let (state, res) = run_transactions(vec![
        tx(TransactionType::Deposit, 1, 1, 50),
        tx(TransactionType::Withdrawal, 1, 2, 100),
    ]);

    println!("{:?}", state.accounts);
    assert_matches!(
        res,
        Err(CephalopodError::TransactionError {
            error: TransactionError::NotEnoughFunds { .. },
            ..
        })
    )
}

#[test]
fn dispute_resolve_should_work() {
    let (state, res) = run_transactions(vec![
        tx(TransactionType::Deposit, 1, 1, 100),
        tx(TransactionType::Withdrawal, 1, 2, 50),
        tx(TransactionType::Deposit, 1, 3, 50),
        tx0(TransactionType::Dispute, 1, 1),
        tx0(TransactionType::Resolve, 1, 1),
    ]);

    assert_matches!(res, Ok(..));
    assert_matches!(state.accounts.get(&1), Some(Account { available, held, locked: false}) if *available == dec(100) && *held == Decimal::ZERO);
}

#[test]
fn dispute_chargeback_should_work() {
    let (state, res) = run_transactions(vec![
        tx(TransactionType::Deposit, 1, 1, 100),
        tx(TransactionType::Withdrawal, 1, 2, 50),
        tx(TransactionType::Deposit, 1, 3, 50),
        tx0(TransactionType::Dispute, 1, 1),
        tx0(TransactionType::Chargeback, 1, 1),
    ]);

    assert_matches!(res, Ok(..));
    assert_matches!(state.accounts.get(&1), Some(Account { available, held, locked: true}) if *available == Decimal::ZERO && *held == Decimal::ZERO);
}

#[test]
fn dispute_alikes_should_fail_for_unknown_transaction() {
    for tpe in vec![
        TransactionType::Dispute,
        TransactionType::Resolve,
        TransactionType::Chargeback,
    ] {
        let (_, res) = run_transactions(vec![tx0(tpe, 1, 7)]);

        assert_matches!(
            res,
            Err(CephalopodError::TransactionError {
                error: TransactionError::TransactionNotFound { .. },
                ..
            })
        )
    }
}

#[test]
fn dispute_should_fail_when_not_enough_funds() {
    let (state, res) = run_transactions(vec![
        tx(TransactionType::Deposit, 1, 1, 100),
        tx(TransactionType::Withdrawal, 1, 2, 50),
        tx0(TransactionType::Dispute, 1, 1),
    ]);

    assert_matches!(
        res,
        Err(CephalopodError::TransactionError {
            error: TransactionError::NotEnoughFunds { .. },
            ..
        })
    );
    assert_matches!(state.accounts.get(&1), Some(Account { available, held, locked: false}) if *available == dec(50) && *held == Decimal::ZERO);
}

#[test]
fn dispute_should_fail_for_withdrawal() {
    let (state, res) = run_transactions(vec![
        tx(TransactionType::Deposit, 1, 1, 100),
        tx(TransactionType::Withdrawal, 1, 2, 50),
        tx0(TransactionType::Dispute, 1, 2),
    ]);

    assert_matches!(
        res,
        Err(CephalopodError::TransactionError {
            error: TransactionError::TransactionInvalidState { .. },
            ..
        })
    );
    assert_matches!(state.accounts.get(&1), Some(Account { available, held, locked: false}) if *available == dec(50) && *held == Decimal::ZERO);
}

#[test]
fn resolve_and_chargeback_should_fail_without_dispute() {
    for tpe in vec![TransactionType::Resolve, TransactionType::Chargeback] {
        let (state, res) = run_transactions(vec![
            tx(TransactionType::Deposit, 1, 1, 100),
            tx0(tpe, 1, 1),
        ]);

        assert_matches!(
            res,
            Err(CephalopodError::TransactionError {
                error: TransactionError::TransactionInvalidState { .. },
                ..
            })
        );
        assert_matches!(state.accounts.get(&1), Some(Account { available, held, locked: false}) if *available == dec(100) && *held == Decimal::ZERO);
    }
}

#[test]
fn dispute_should_fail_for_client_id_mismatch() {
    let (state, res) = run_transactions(vec![
        tx(TransactionType::Deposit, 1, 1, 100),
        tx(TransactionType::Deposit, 2, 2, 200),
        tx0(TransactionType::Dispute, 2, 1),
    ]);

    assert_matches!(
        res,
        Err(CephalopodError::TransactionError {
            error: TransactionError::TransactionClientMismatch { .. },
            ..
        })
    );
    assert_matches!(state.accounts.get(&1), Some(Account { available, held, locked: false}) if *available == dec(100) && *held == Decimal::ZERO);
}

#[test]
fn resolve_chargeback_should_fail_for_client_id_mismatch() {
    for tpe in vec![TransactionType::Resolve, TransactionType::Chargeback] {
        let (state, res) = run_transactions(vec![
            tx(TransactionType::Deposit, 1, 1, 100),
            tx(TransactionType::Deposit, 2, 2, 200),
            tx0(TransactionType::Dispute, 1, 1),
            tx0(tpe, 2, 1),
        ]);

        assert_matches!(
            res,
            Err(CephalopodError::TransactionError {
                error: TransactionError::TransactionClientMismatch { .. },
                ..
            })
        );
        assert_matches!(state.accounts.get(&1), Some(Account { available, held, locked: false}) if *available == Decimal::ZERO && *held == dec(100));
    }
}

#[test]
fn transactions_cannot_be_applied_to_locked_account() {
    let initial = vec![
        tx(TransactionType::Deposit, 1, 1, 110),
        tx(TransactionType::Deposit, 1, 2, 120),
        tx(TransactionType::Deposit, 1, 3, 130),
        tx0(TransactionType::Dispute, 1, 1),
        tx0(TransactionType::Dispute, 1, 2),
        tx0(TransactionType::Chargeback, 1, 1),
    ];

    let next_txs = vec![
        tx(TransactionType::Deposit, 1, 4, 10),
        tx(TransactionType::Withdrawal, 1, 4, 10),
        tx(TransactionType::Dispute, 1, 3, 100),
        tx(TransactionType::Resolve, 1, 2, 100),
        tx(TransactionType::Chargeback, 1, 2, 100),
    ];

    for next in next_txs {
        let (state, res) =
            run_transactions(initial.iter().chain(vec![next].iter()).cloned().collect());

        assert_matches!(
            res,
            Err(CephalopodError::TransactionError {
                error: TransactionError::AccountLocked { .. },
                ..
            })
        );
        assert_matches!(state.accounts.get(&1), Some(Account { available, held, locked: true }) if *available == dec(130) && *held == dec(120));
    }
}
