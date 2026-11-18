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
