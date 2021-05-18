This is solution for a coding test from a recruitment process. It was supposed to take 4 hours, and despite spending some more time I don't feel it's really polished. But it seems to work and should be mostly correct, even though I don't think I've covered all edge cases.

Some highlights:

- `Decimal` type is used to represent amounts in transactions and balances in accounts. Using floating numbers when representing money is out of discussion IMO. In real system, if performance were crucial, it might be better idea to create a wrapper for `i64` or `u64` for fixed-precision computations.
- There are two types of errors. `TransactionError` means that invalid request has been provided to the system and it should be ignored. `IntegrityError` is much nastier and means that there is a bug somewhere in the code.
- Logging is based on standard Rust mechanisms and can be enabled by setting `RUST_LOG=info` environment variable.


Known shortcomings:

- I haven't tested more exotic scenarios such as double-dispute attepmts.
- Error support is somewhat convoluted. That is mostly caused by including the transaction that caused the error. I can now tell it was unnecessary and the code would be simplified by removing it.
- There aren't many comments, but the code should be mostly self-documenting.
- **The task description doesn't explain what "locked account" means. I assumed that no transaction can be applied to such account, but the author might have had something different in mind.** 
- **The description mentions that in case of dispute, available funds should be decreased. That makes only sense when the disputed transaction is a deposit, so I'm making assumption that withdrawals cannot be disputed**
- Once a dispute is resolved, it cannot be disputed again. That semantics made sense to me, but it might not be what was expected either.
