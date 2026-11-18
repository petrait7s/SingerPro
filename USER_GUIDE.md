# User guide

## First start

On a new installation Singer Pro asks you to create the **Owner** account. Choose your own username and password (at least 10 characters; no common passwords; must not contain the username). There is no default password. Keep the password safe: the Owner can reset other users' passwords, but nobody can recover the Owner's password without a backup.

## Signing in

Sign in with your username and password. After 5 wrong attempts the account is locked for 15 minutes. You are signed out after 30 minutes without activity. New or reset accounts must choose a new password at first sign-in.

## Language

Use the **বাংলা / English** button (top right). The choice is remembered. Shop owners can also set it in Settings.

## Users and roles (Staff & Users)

Owner/Manager-level users can add staff, give them roles, reset passwords and deactivate leavers (a reason is required; history is kept). Roles are bundles of permissions. Create custom roles under _Roles & permissions_; you can only grant permissions you hold yourself. Only an Owner can create or change another Owner, and at least one active Owner always remains.

## Settings

- **Shop identity**: name, address, phone, email, invoice footer and prefix, logo (PNG/JPEG up to 512 KB; use only artwork you may use).
- **Currency & tax**: BDT/৳ by default. Tax is **off (0%)** until an authorised user enables it; every change is audited.
- **Installments**: default markup is **0%**; markup recognition policy; grace days; optional late fee. Nothing is ever added silently.
- **Backup & restore**: see `BACKUP_RESTORE.md`.
- **Audit log**: who did what and when (needs the audit permission).

Further modules are documented here as they are released.

## Products (P2)

Products -> **Brands & categories** to define brands, categories and subcategories. **New product** creates a product with its first variant (SKU, optional barcode, colour/capacity/size, purchase cost, retail, wholesale and optional installment price). Tick _Track each unit by serial number_ for appliances such as refrigerators and ACs: every physical unit then has its own serial record. This cannot be changed once the product has stock history. Price changes are recorded in the audit log.
Enter money as taka, e.g. `45,000.50`; the system stores exact paisa.

## Suppliers and purchases (P2)

1. **Suppliers**: add the companies you buy from. _Statement_ shows what you owe them; _Pay_ records a payment on account.
2. **Purchases -> New purchase**: pick the supplier, enter their invoice number (the same invoice cannot be entered twice for one supplier), add items by scanning/searching, add landed costs (freight etc.) and any supplier discount. **Save draft** keeps it editable; **Confirm invoice** locks it and records what you owe.
3. **Receive goods** when the stock arrives. You may receive in several deliveries. For serial-tracked items enter exactly one serial number per unit (one per line). Nothing enters stock until it is received.
4. **Record payment** against the purchase (cannot exceed the amount due). A wrong payment is reversed, never edited. **Return to supplier** sends received goods back and credits the supplier.

## Inventory (P2)

- **Stock levels** show available, reserved, damaged, service and awaiting-inspection quantities, average cost and value. _History_ shows every movement with who, when and why.
- **Opening** records the starting stock of an item that has no history yet. **Adjust** corrects counts (reason required). **Move** re-classifies units, e.g. available -> damaged.
- **Serial numbers** lets you find any unit, see its full life history, and mark it damaged, in service, repaired or written off. Units cannot be sold again after a return until they are inspected (returns module).
- **Reconciliation** checks that the stock history, the stock balances and the serial numbers agree.

## Selling (P3)

**New Sale / POS**: scan or search a product (Enter adds an exact barcode/SKU match; F2 refocuses search). Serial-tracked items ask you to choose the exact units. Choose a customer (optional for retail). Enter the amount received, or **Split payment** across cash/bank/wallet; any change to hand back is shown but never recorded as a payment. **Complete sale (F9)**, then print the **80mm receipt** or **A4 invoice**.

- Retail sales must be paid in full. Wholesale sales (switch at the top; needs permission) need a customer and may be partly or wholly on credit within the customer's credit limit; a manager can override the limit and the override is audited.
- Discounts need the discount permission and are recorded in the audit log. Prices come from the product's retail or wholesale price. Tax appears only if enabled in Settings.
- Double-clicking _Complete sale_ cannot create two sales.

**Sales** lists every invoice (search, date range, "with balance due") with its receipts. A wrong receipt is _reversed_ (new entry, reason required), never deleted.

**Customers**: profile, invoices, ledger and what they owe. **Collect payment** applies money to the oldest unpaid invoices first and issues one receipt per invoice. **Opening balance** records what a customer owed before you started (once).

**Accounts** (needs accounts permission): chart of accounts with balances, cashbook and ledgers per account, journal (every entry traces to its document), trial balance, manual journals and opening balances (accountants), and a reconciliation page that checks customers, suppliers, stock and tax against the books. See `docs/ACCOUNTING_POLICY.md`.

## Wholesale pricing (P4)

**Customers -> Price tiers** defines tiers such as "Gold dealer -3%". Open a wholesale/dealer customer and click **Pricing** to assign a tier and set special prices for specific products. Order of precedence on wholesale sales: special price, then tier, then the product's wholesale price. Needs the pricing permission.

## Installments (P4)

**Signing**: New Sale / POS -> _Installment_ tab (needs the installment permission). Scan the item (choose the serial number), pick the customer (required), enter the **down payment**, then set the number of installments, weekly/monthly and the first due date. The disclosure box shows, separately: cash price, installment price, down payment, financed amount, markup (default **0%**), any explicit fees, the total payable and the exact schedule. Only authorised users can set a different markup or add fees. Complete the sale to create the agreement.
**Installments** screen: all agreements, and the **Overdue follow-up** list (most overdue first). Open an agreement to see the contract, every installment's status (pending, partial, overdue, paid) and all collections.

- **Collect payment**: any amount up to the outstanding balance; it pays the installment you chose (or the oldest unpaid) first and then continues to the next ones. Part-payments never mark an installment as paid.
- **Settle early**: pays the exact balance and closes the agreement at zero. A manager can waive part of the unearned markup/fees (reason required).
- **Reschedule**: replaces the unpaid part of the schedule (reason required); payments already made stay.
- **Late fee**: only if enabled in Settings; added explicitly to one overdue installment.
- **Add follow-up**: note a call and the date the customer promised to pay.
- **Reverse**: a wrong collection is reversed (reason required), never deleted.
  Customers can hold several agreements; each keeps its own balance.

## Returns and warranty (P5)

**Returns.** Open _Returns & Warranty_, press _New return_ and enter the invoice number or the unit's serial
number. Choose the items and quantities and give a reason. The goods are now held for inspection: they cannot be
sold and no money moves. A user with the approval permission opens the return (_Inspect_) and chooses for each item:
back to stock, damaged (held) or write off (needs a note). They may refund money now, up to what the customer has
overpaid; the rest stays as credit on the customer's account. A rejected return puts the sale back as it was.
Installment sales are not returned here; use the agreement. For an exchange, record a return and then a new sale.

**Warranty.** _Check serial / open case_ shows who bought the unit, when, and until when it is covered. Cases move
from open to in service to resolved or rejected, and every step is kept with who made it and when.

## Expenses and the daily cash session (P5)

**Expenses.** Press _New expense_. Entries by users who cannot approve wait for approval; approvers can approve,
reject (with a reason) or void (with a reason; the record stays and the accounting entry is reversed).

**Cash session.** Open a session with the cash in the drawer when the day starts. When you close, count the cash and
enter it. The screen shows the shortage or overage, and an explanation is required if there is one. Closing locks the
session. In Settings you can require an open session before cash can be taken or paid.
