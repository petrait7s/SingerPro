# Accounting policy and posting rules

Double-entry, accrual basis, BDT, integer paisa. Every business event posts exactly ONE balanced journal entry
per document (`journal_entries.posting_key`, unique -> posting is idempotent). Posted entries and lines are
append-only (DB triggers). Corrections are reversals/new documents with a reason and an audit record.

**Assumptions flagged for your accountant** (none is legal/tax advice): revenue is recognised at sale
(retail/wholesale); tax is a pass-through liability; installment markup recognition is configurable
(see `installment.markup_recognition`) and arrives with P4; purchase landed costs are capitalised to inventory.

## Chart of accounts (seeded, codes fixed; names editable in the database)

1000 Cash in Hand · 1010 Bank · 1020 Mobile Wallets · 1100 Accounts Receivable · 1110 Installment Receivable ·
1200 Inventory · 1210 Inventory in Transit · 2000 Accounts Payable · 2100 Unearned Installment Markup ·
2150 Accrued Landed Costs · 2200 Tax Payable · 2300 Customer Advances/Refunds · 3000 Opening Balance Equity ·
3100 Owner's Capital · 4000 Sales · 4010 Sales Returns · 4100 Installment Markup Income · 4900 Other Income ·
5000 COGS · 5100 Inventory Adjustments · 5110 Purchase Return Variance · 6000-6900 operating expenses.

## Posting rules (implemented)

| Event                                              | Debit                                 | Credit                                                        |
| -------------------------------------------------- | ------------------------------------- | ------------------------------------------------------------- |
| Purchase confirmed                                 | 1210 In Transit (payable + landed)    | 2000 Payable (subtotal - discount); 2150 Accrued landed costs |
| Purchase cancelled (nothing received/paid)         | 2000, 2150                            | 1210                                                          |
| Goods received                                     | 1200 Inventory (allocated unit costs) | 1210                                                          |
| Return to supplier                                 | 2000 (credit = units x invoice cost)  | 1200 (stock cost); difference to 5110                         |
| Supplier payment                                   | 2000                                  | 1000/1010/1020 by method                                      |
| Supplier payment reversal                          | cash/bank/wallet                      | 2000                                                          |
| Opening stock                                      | 1200                                  | 3000                                                          |
| Stock gain / loss / write-off / serial written off | 1200 / 5100                           | 5100 / 1200                                                   |
| Bucket moves, damaged/service transitions          | none (value stays in 1200)            |                                                               |
| Sale (retail/wholesale)                            | 1100 Receivable (total)               | 4000 Sales (net of discount, excl. tax); 2200 Tax payable     |
| Cost of goods sold                                 | 5000                                  | 1200 (serial cost / weighted-average cost)                    |
| Receipt (customer payment)                         | 1000/1010/1020                        | 1100                                                          |
| Receipt reversal                                   | 1100                                  | cash/bank/wallet                                              |
| Customer / cash / bank opening balance             | 1100 / 1000-1020                      | 3000                                                          |
| Manual journal / reversal                          | as entered / mirrored                 | accountant, reason required                                   |

Customers pay through receipts, so a fully paid retail sale nets to zero in 1100. Walk-in retail must be paid in full.

## Costing

Weighted-average cost per variant and bucket; serial-tracked units carry their own exact cost. Removing a whole
bucket removes its whole value (no rounding residue). Each purchase unit's cost is a deterministic slice of its
line (line total + allocated landed costs - allocated discount), remainder on the last unit.

## Tax

Disabled by default (0%). When enabled: exclusive (tax added) or inclusive (tax extracted) per setting; rounding is
half away from zero per line, in paisa. Tax collected is posted to 2200. Tax changes are audited.

## Reconciliation checks (`accounting::reconcile`, Accounts -> Reconciliation)

Entry balance, trial balance, supplier ledger = 2000, customer ledger = 1100+1110, stock value = 1200,
in-transit = ordered - received, and documents without a journal (repaired by `ensure_posted`, which also runs at
every start-up to back-fill documents created by older versions).
