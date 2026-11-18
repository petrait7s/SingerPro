// Single typed gateway to the Rust backend. Components never call `invoke` directly.
// The session token is held in Rust; the webview never sees it.
import { invoke } from "@tauri-apps/api/core";

export interface ApiError {
  code: string;
  message: string;
}

export interface AppInfo {
  product: string;
  developer: string;
  version: string;
  data_dir: string;
}
export interface DbStatus {
  schema_version: number;
  journal_mode: string;
  foreign_keys: boolean;
  integrity_ok: boolean;
  integrity_detail: string[];
}
export interface AuditRow {
  id: string;
  at: string;
  actor_id: string | null;
  action: string;
  entity_type: string;
  entity_id: string | null;
  reason: string | null;
}
export interface AuditPage {
  total: number;
  rows: AuditRow[];
}
export interface Principal {
  user_id: string;
  username: string;
  full_name: string;
  must_change_password: boolean;
  roles: string[];
  permissions: string[];
}
export interface RestoreOutcome {
  applied: boolean;
  message: string;
  replaced_dir: string | null;
}
export interface SetupStatus {
  needs_setup: boolean;
  startup_notice: RestoreOutcome | null;
}
export interface UserRow {
  id: string;
  username: string;
  full_name: string;
  is_active: boolean;
  must_change_password: boolean;
  roles: string[];
  last_login_at: string | null;
  created_at: string;
}
export interface RoleRow {
  id: string;
  name: string;
  description: string;
  is_system: boolean;
  permissions: string[];
}
export interface PermissionRow {
  key: string;
  category: string;
  description: string;
}
export interface NewUser {
  username: string;
  full_name: string;
  password: string;
  role_ids: string[];
}
export interface Branding {
  shop_name: string;
  shop_address: string;
  shop_phone: string;
  shop_email: string;
  invoice_footer: string;
  currency_symbol: string;
  language: string;
}
export interface BackupInfo {
  path: string;
  manifest_path: string;
  size_bytes: number;
  sha256: string;
  schema_version: number;
}
export interface Manifest {
  created_at: string;
  app_version: string;
  schema_version: number;
}
export interface Preview {
  path: string;
  size_bytes: number;
  sha256: string;
  manifest: Manifest | null;
  checksum_ok: boolean | null;
  schema_version: number | null;
  app_schema_version: number;
  users: number | null;
  audit_rows: number | null;
  problems: string[];
  warnings: string[];
  can_restore: boolean;
}
export interface HistoryRow {
  id: string;
  at: string;
  kind: string;
  path: string;
  size_bytes: number | null;
  status: string;
  note: string | null;
}
export interface BackupStatus {
  last_backup_at: string | null;
  reminder_days: number;
  overdue: boolean;
}
export interface RestoreMarker {
  staged_at: string;
  source: string;
  sha256: string;
  safety_copy: string;
}
export interface StagedRestore {
  marker: RestoreMarker;
  preview: Preview;
}

// ---- P2 types
export interface Named {
  id: string;
  name: string;
  is_active: boolean;
}
export interface CategoryRow {
  id: string;
  name: string;
  parent_id: string | null;
  is_active: boolean;
}
export interface NewVariant {
  sku: string;
  barcode: string | null;
  colour: string;
  capacity: string;
  size: string;
  purchase_cost: number;
  retail_price: number;
  wholesale_price: number;
  installment_price: number | null;
}
export interface UpdateVariant extends NewVariant {
  id: string;
}
export interface NewProduct {
  brand_id: string;
  category_id: string;
  name: string;
  model: string;
  description: string;
  warranty_months: number;
  warranty_terms: string;
  min_stock: number;
  is_serialized: boolean;
  variants: NewVariant[];
}
export interface UpdateProduct {
  id: string;
  brand_id: string;
  category_id: string;
  name: string;
  model: string;
  description: string;
  warranty_months: number;
  warranty_terms: string;
  min_stock: number;
  is_serialized: boolean;
}
export interface VariantRow {
  id: string;
  sku: string;
  barcode: string | null;
  colour: string;
  capacity: string;
  size: string;
  purchase_cost: number;
  retail_price: number;
  wholesale_price: number;
  installment_price: number | null;
  is_active: boolean;
  available: number;
  on_hand_total: number;
  stock_value: number;
}
export interface ProductRow {
  id: string;
  brand_id: string;
  brand: string;
  category_id: string;
  category: string;
  name: string;
  model: string;
  description: string;
  warranty_months: number;
  warranty_terms: string;
  min_stock: number;
  is_serialized: boolean;
  is_active: boolean;
  available_total: number;
  low_stock: boolean;
  variants: VariantRow[];
}
export interface ProductPage {
  total: number;
  rows: ProductRow[];
}
export interface ProductFilter {
  search?: string;
  brand_id?: string;
  category_id?: string;
  include_inactive?: boolean;
  low_stock_only?: boolean;
  limit?: number;
  offset?: number;
}
export interface VariantPick {
  variant_id: string;
  product_id: string;
  label: string;
  sku: string;
  barcode: string | null;
  is_serialized: boolean;
  purchase_cost: number;
  retail_price: number;
  wholesale_price: number;
  installment_price: number | null;
  available: number;
}
export interface SupplierIn {
  name: string;
  contact_person: string;
  phone: string;
  alt_phone: string;
  email: string;
  address: string;
  notes: string;
}
export interface SupplierRow extends SupplierIn {
  id: string;
  is_active: boolean;
  balance: number;
}
export interface LedgerLine {
  at: string;
  kind: string;
  doc_type: string;
  doc_id: string;
  note: string;
  amount: number;
  balance: number;
}
export interface Statement {
  supplier_id: string;
  supplier: string;
  lines: LedgerLine[];
  closing_balance: number;
}
export interface LineIn {
  variant_id: string;
  qty: number;
  unit_cost: number;
}
export interface CostIn {
  label: string;
  amount: number;
}
export interface PurchaseIn {
  supplier_id: string;
  supplier_invoice_no: string | null;
  order_date: string;
  discount: number;
  notes: string;
  items: LineIn[];
  costs: CostIn[];
}
export interface PurchaseRow {
  id: string;
  number: string;
  supplier_id: string;
  supplier: string;
  supplier_invoice_no: string | null;
  order_date: string;
  status: string;
  grand_total: number;
  landed_total: number;
  paid: number;
  credits: number;
  due: number;
  ordered_qty: number;
  received_qty: number;
}
export interface PurchasePage {
  total: number;
  rows: PurchaseRow[];
}
export interface PurchaseFilter {
  status?: string;
  supplier_id?: string;
  search?: string;
  limit?: number;
  offset?: number;
}
export interface PurchaseItem {
  id: string;
  variant_id: string;
  label: string;
  sku: string;
  is_serialized: boolean;
  qty: number;
  unit_cost: number;
  landed_alloc: number;
  received_qty: number;
  returned_qty: number;
  line_total: number;
}
export interface PaymentRow {
  id: string;
  amount: number;
  method: string;
  paid_on: string;
  reference: string;
  note: string;
  reverses_id: string | null;
}
export interface PurchaseDetail {
  purchase: PurchaseRow;
  notes: string;
  discount: number;
  subtotal: number;
  cancel_reason: string | null;
  items: PurchaseItem[];
  costs: CostIn[];
  payments: PaymentRow[];
  receipts: { id: string; received_on: string; note: string; qty: number; cost_total: number }[];
  returns: {
    id: string;
    number: string;
    returned_on: string;
    reason: string;
    credit_total: number;
  }[];
}
export interface ReceiveIn {
  received_on: string;
  note: string;
  lines: { item_id: string; qty: number; serials: string[] }[];
}
export interface ReturnIn {
  returned_on: string;
  reason: string;
  lines: { item_id: string; qty: number; serials: string[] }[];
}
export interface PaymentIn {
  supplier_id: string;
  purchase_id: string | null;
  amount: number;
  method: string;
  paid_on: string;
  reference: string;
  note: string;
  client_key: string;
}
export interface StockRow {
  variant_id: string;
  product_id: string;
  label: string;
  brand: string;
  category: string;
  sku: string;
  barcode: string | null;
  is_serialized: boolean;
  min_stock: number;
  available: number;
  reserved: number;
  damaged: number;
  warranty: number;
  pending_inspection: number;
  value: number;
  avg_cost: number;
  low_stock: boolean;
}
export interface StockPage {
  total: number;
  total_value: number;
  rows: StockRow[];
}
export interface MovementRow {
  id: string;
  at: string;
  bucket: string;
  qty_delta: number;
  cost_total: number;
  balance_after: number;
  doc_type: string;
  doc_id: string;
  reason: string;
  serial_no: string | null;
  actor: string | null;
}
export interface SerialRow {
  id: string;
  variant_id: string;
  serial_no: string;
  status: string;
  cost: number;
  label: string;
  sku: string;
  received_at: string;
}
export interface SerialEvent {
  at: string;
  from_status: string | null;
  to_status: string;
  doc_type: string;
  doc_id: string;
  reason: string;
  actor: string | null;
}
export interface Discrepancy {
  variant_id: string;
  sku: string;
  bucket: string;
  check: string;
  expected: number;
  actual: number;
}

// ---- P3 types
export interface AccountRow {
  id: string;
  code: string;
  name: string;
  account_type: string;
  normal_side: string;
  is_system: boolean;
  is_active: boolean;
  debit: number;
  credit: number;
  balance: number;
}
export interface TrialBalance {
  as_of: string | null;
  rows: AccountRow[];
  total_debit: number;
  total_credit: number;
  balanced: boolean;
}
export interface LedgerLine2 {
  entry_id: string;
  number: string;
  date: string;
  memo: string;
  line_memo: string;
  debit: number;
  credit: number;
  balance: number;
}
export interface Ledger {
  account: string;
  name: string;
  opening: number;
  lines: LedgerLine2[];
  closing: number;
}
export interface EntryRow {
  id: string;
  number: string;
  entry_date: string;
  memo: string;
  source_type: string;
  source_id: string;
  total: number;
  reverses_id: string | null;
  reversed: boolean;
}
export interface EntryPage {
  total: number;
  rows: EntryRow[];
}
export interface EntryDetail {
  entry: EntryRow;
  lines: { code: string; name: string; debit: number; credit: number; memo: string }[];
}
export interface ManualLine {
  account_code: string;
  debit: number;
  credit: number;
  memo: string;
}
export interface Mismatch {
  check: string;
  detail: string;
  expected: number;
  actual: number;
}
export interface CustomerIn {
  name: string;
  phone: string;
  alt_phone: string;
  address: string;
  customer_type: string;
  credit_limit: number;
  nid: string;
  notes: string;
}
export interface CustomerRow extends CustomerIn {
  id: string;
  code: string;
  is_active: boolean;
  price_tier_id: string | null;
  registered_on: string;
  balance: number;
}
export interface CustomerStatement {
  customer: CustomerRow;
  lines: {
    at: string;
    kind: string;
    doc_type: string;
    doc_id: string;
    note: string;
    amount: number;
    balance: number;
  }[];
  closing_balance: number;
}
export interface SaleLineIn {
  variant_id: string;
  qty: number;
  discount: number;
  serial_ids: string[];
}
export interface PayLineIn {
  method: string;
  amount: number;
  reference: string;
}
export interface SaleIn {
  mode: string;
  customer_id: string | null;
  sale_date: string | null;
  items: SaleLineIn[];
  payments: PayLineIn[];
  notes: string;
  client_key: string;
  credit_override: boolean;
  installment?: InstallmentTerms | null;
}
export interface InstallmentTerms {
  markup_bp: number | null;
  count: number;
  frequency: string;
  first_due_date: string;
  custom: { due_date: string; amount: number }[];
  fees: { label: string; amount: number }[];
  notes: string;
}
export interface SaleResult {
  id: string;
  number: string;
  grand_total: number;
  paid: number;
  due: number;
  receipts: string[];
}
export interface SalePreview {
  subtotal: number;
  discount_total: number;
  tax_total: number;
  grand_total: number;
  lines: {
    variant_id: string;
    unit_price: number;
    discount: number;
    tax: number;
    line_total: number;
  }[];
}
export interface SaleRow {
  id: string;
  number: string;
  mode: string;
  sale_date: string;
  customer_id: string | null;
  customer: string | null;
  grand_total: number;
  paid: number;
  due: number;
}
export interface SalePage {
  total: number;
  rows: SaleRow[];
}
export interface SaleFilter {
  search?: string;
  mode?: string;
  customer_id?: string;
  from?: string;
  to?: string;
  due_only?: boolean;
  limit?: number;
  offset?: number;
}
export interface SaleDetail {
  sale: SaleRow;
  subtotal: number;
  discount_total: number;
  tax_total: number;
  financing_charges: number;
  notes: string;
  cashier: string | null;
  customer_phone: string | null;
  customer_address: string | null;
  items: {
    label: string;
    sku: string;
    serial_no: string | null;
    qty: number;
    unit_price: number;
    discount: number;
    tax: number;
    line_total: number;
  }[];
  payments: {
    id: string;
    receipt_no: string;
    method: string;
    amount: number;
    paid_on: string;
    reference: string;
    note: string;
    reverses_id: string | null;
  }[];
}
export interface CollectIn {
  customer_id: string;
  sale_id: string | null;
  amount: number;
  method: string;
  reference: string;
  paid_on: string | null;
  client_key: string;
}
export interface CollectResult {
  receipts: string[];
  allocations: [string, number][];
}
export interface SerialChoice {
  id: string;
  serial_no: string;
}

// ---- P4 types
export interface TierRow {
  id: string;
  name: string;
  adjust_bp: number;
  is_active: boolean;
  customers: number;
}
export interface CustomerPriceRow {
  variant_id: string;
  label: string;
  sku: string;
  wholesale_price: number;
  price: number;
}
export interface AgreementPreview {
  cash_price: number;
  price_total: number;
  down_payment: number;
  principal: number;
  markup_bp: number;
  markup: number;
  fees_total: number;
  total_payable: number;
  schedule: [string, number][];
}
export interface AgreementRow {
  id: string;
  number: string;
  sale_id: string;
  sale_number: string;
  customer_id: string;
  customer: string;
  customer_phone: string;
  status: string;
  down_payment: number;
  total_payable: number;
  collected: number;
  outstanding: number;
  next_due_date: string | null;
  next_due_amount: number;
  overdue_amount: number;
  max_days_overdue: number;
  installment_count: number;
  created_on: string;
}
export interface AgreementPage {
  total: number;
  overdue_total: number;
  outstanding_total: number;
  rows: AgreementRow[];
}
export interface AgreementFilter {
  search?: string;
  status?: string;
  customer_id?: string;
  overdue_only?: boolean;
  limit?: number;
  offset?: number;
}
export interface ScheduleItem {
  id: string;
  seq: number;
  due_date: string;
  amount: number;
  late_fee: number;
  paid: number;
  superseded: boolean;
  effective: number;
  remaining: number;
  status: string;
  days_overdue: number;
}
export interface AgreementDetail {
  agreement: AgreementRow;
  cash_price: number;
  price_total: number;
  principal: number;
  markup_bp: number;
  markup: number;
  fees: { label: string; amount: number }[];
  late_fees_total: number;
  rebates: number;
  frequency: string;
  first_due_date: string;
  recognition_policy: string;
  earned_markup: number;
  unearned_markup: number;
  notes: string;
  items: ScheduleItem[];
  collections: {
    id: string;
    receipt_no: string;
    kind: string;
    amount: number;
    method: string;
    paid_on: string;
    reference: string;
    note: string;
    reverses_id: string | null;
  }[];
  reschedules: {
    at: string;
    reason: string;
    outstanding: number;
    old_count: number;
    new_count: number;
  }[];
  followups: { at: string; note: string; promise_date: string | null; actor: string | null }[];
  grace_days: number;
}
export interface InstCollectIn {
  agreement_id: string;
  amount: number;
  method: string;
  reference: string;
  paid_on: string | null;
  target_schedule_id: string | null;
  client_key: string;
}
export interface InstCollectResult {
  receipt: string;
  allocations: [number, number][];
  outstanding: number;
  completed: boolean;
}
export interface SettleQuote {
  outstanding: number;
  max_rebate: number;
}

export function inDesktop(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

export function toApiError(e: unknown): ApiError {
  if (typeof e === "object" && e !== null && "message" in e) {
    const o = e as { code?: unknown; message: unknown };
    return { code: typeof o.code === "string" ? o.code : "unknown", message: String(o.message) };
  }
  return { code: "unknown", message: String(e) };
}

type Listener = () => void;
const sessionLost = new Set<Listener>();
/** Fires when any call reports the session ended (expired, deactivated). */
export function onSessionLost(l: Listener): () => void {
  sessionLost.add(l);
  return () => sessionLost.delete(l);
}

async function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  if (!inDesktop()) {
    throw { code: "no_runtime", message: "Open Singer Pro as the desktop app to access data." };
  }
  try {
    return await invoke<T>(cmd, args);
  } catch (e) {
    const err = toApiError(e);
    if (err.code === "unauthenticated" && cmd !== "session") sessionLost.forEach((l) => l());
    throw err;
  }
}

export const api = {
  appInfo: () => call<AppInfo>("app_info"),
  setupStatus: () => call<SetupStatus>("setup_status"),
  setupOwner: (username: string, fullName: string, password: string) =>
    call<Principal>("setup_owner", { username, fullName, password }),
  login: (username: string, password: string) => call<Principal>("login", { username, password }),
  logout: () => call<void>("logout"),
  session: () => call<Principal | null>("session"),
  changePassword: (old: string, next: string) => call<void>("change_password", { old, new: next }),
  dbStatus: () => call<DbStatus>("db_status"),
  auditList: (limit: number, offset: number, actionPrefix?: string) =>
    call<AuditPage>("audit_list", { limit, offset, actionPrefix }),

  usersList: () => call<UserRow[]>("users_list"),
  rolesList: () => call<RoleRow[]>("roles_list"),
  permissionsList: () => call<PermissionRow[]>("permissions_list"),
  userCreate: (user: NewUser) => call<string>("user_create", { user }),
  userSetActive: (userId: string, active: boolean, reason: string) =>
    call<void>("user_set_active", { userId, active, reason }),
  userSetRoles: (userId: string, roleIds: string[]) =>
    call<void>("user_set_roles", { userId, roleIds }),
  userResetPassword: (userId: string, password: string) =>
    call<void>("user_reset_password", { userId, password }),
  roleCreate: (name: string, description: string, permissions: string[]) =>
    call<string>("role_create", { name, description, permissions }),
  roleSetPermissions: (roleId: string, permissions: string[]) =>
    call<void>("role_set_permissions", { roleId, permissions }),
  roleDelete: (roleId: string) => call<void>("role_delete", { roleId }),

  settingsGet: () => call<Record<string, string>>("settings_get"),
  settingsUpdate: (changes: Record<string, string>) =>
    call<string[]>("settings_update", { changes }),
  branding: () => call<Branding>("branding"),
  logoGet: () => call<string | null>("logo_get"),
  logoSet: (dataBase64: string) => call<void>("logo_set", { dataBase64 }),
  logoClear: () => call<void>("logo_clear"),

  backupCreate: (destDir?: string) => call<BackupInfo>("backup_create", { destDir }),
  backupHistory: () => call<HistoryRow[]>("backup_history"),
  backupStatus: () => call<BackupStatus>("backup_status"),
  backupDefaultDir: () => call<string>("backup_default_dir"),
  backupVerify: (path: string) => call<Preview>("backup_verify", { path }),
  restoreStage: (path: string) => call<StagedRestore>("restore_stage", { path }),
  restorePending: () => call<RestoreMarker | null>("restore_pending"),
  restoreCancel: () => call<void>("restore_cancel"),
  restartApp: () => call<void>("restart_app"),

  brandsList: () => call<Named[]>("brands_list"),
  brandCreate: (name: string) => call<string>("brand_create", { name }),
  brandRename: (brandId: string, name: string) => call<void>("brand_rename", { brandId, name }),
  brandSetActive: (brandId: string, active: boolean) =>
    call<void>("brand_set_active", { brandId, active }),
  categoriesList: () => call<CategoryRow[]>("categories_list"),
  categoryCreate: (name: string, parentId: string | null) =>
    call<string>("category_create", { name, parentId }),
  categorySetActive: (categoryId: string, active: boolean) =>
    call<void>("category_set_active", { categoryId, active }),
  productsList: (filter: ProductFilter) => call<ProductPage>("products_list", { filter }),
  productGet: (productId: string) => call<ProductRow>("product_get", { productId }),
  productCreate: (product: NewProduct) => call<string>("product_create", { product }),
  productUpdate: (product: UpdateProduct) => call<void>("product_update", { product }),
  productSetActive: (productId: string, active: boolean) =>
    call<void>("product_set_active", { productId, active }),
  variantAdd: (productId: string, variant: NewVariant) =>
    call<string>("variant_add", { productId, variant }),
  variantUpdate: (variant: UpdateVariant) => call<void>("variant_update", { variant }),
  variantSetActive: (variantId: string, active: boolean) =>
    call<void>("variant_set_active", { variantId, active }),
  productImageGet: (productId: string) => call<string | null>("product_image_get", { productId }),
  productImageSet: (productId: string, dataBase64: string) =>
    call<void>("product_image_set", { productId, dataBase64 }),
  variantsSearch: (query: string, limit = 15) =>
    call<VariantPick[]>("variants_search", { query, limit }),

  suppliersList: (search?: string, includeInactive = false) =>
    call<SupplierRow[]>("suppliers_list", { search, includeInactive }),
  supplierCreate: (supplier: SupplierIn) => call<string>("supplier_create", { supplier }),
  supplierUpdate: (supplierId: string, supplier: SupplierIn) =>
    call<void>("supplier_update", { supplierId, supplier }),
  supplierSetActive: (supplierId: string, active: boolean) =>
    call<void>("supplier_set_active", { supplierId, active }),
  supplierStatement: (supplierId: string) => call<Statement>("supplier_statement", { supplierId }),
  purchasesList: (filter: PurchaseFilter) => call<PurchasePage>("purchases_list", { filter }),
  purchaseGet: (purchaseId: string) => call<PurchaseDetail>("purchase_get", { purchaseId }),
  purchaseCreateDraft: (purchase: PurchaseIn) =>
    call<string>("purchase_create_draft", { purchase }),
  purchaseUpdateDraft: (purchaseId: string, purchase: PurchaseIn) =>
    call<void>("purchase_update_draft", { purchaseId, purchase }),
  purchaseConfirm: (purchaseId: string) => call<void>("purchase_confirm", { purchaseId }),
  purchaseCancel: (purchaseId: string, reason: string) =>
    call<void>("purchase_cancel", { purchaseId, reason }),
  purchaseReceive: (purchaseId: string, receipt: ReceiveIn) =>
    call<string>("purchase_receive", { purchaseId, receipt }),
  purchaseReturn: (purchaseId: string, ret: ReturnIn) =>
    call<string>("purchase_return", { purchaseId, ret }),
  supplierPay: (payment: PaymentIn) => call<string>("supplier_pay", { payment }),
  supplierPaymentReverse: (paymentId: string, reason: string) =>
    call<string>("supplier_payment_reverse", { paymentId, reason }),

  stockLevels: (filter: {
    search?: string;
    low_only?: boolean;
    in_stock_only?: boolean;
    limit?: number;
    offset?: number;
  }) => call<StockPage>("stock_levels", { filter }),
  stockMovements: (variantId: string, limit = 100, offset = 0) =>
    call<MovementRow[]>("stock_movements", { variantId, limit, offset }),
  serialsList: (filter: {
    variant_id?: string;
    search?: string;
    status?: string;
    limit?: number;
    offset?: number;
  }) => call<SerialRow[]>("serials_list", { filter }),
  serialHistory: (serialId: string) => call<SerialEvent[]>("serial_history", { serialId }),
  stockOpening: (opening: {
    variant_id: string;
    qty: number;
    unit_cost: number;
    serials: string[];
    reason: string;
  }) => call<string>("stock_opening", { opening }),
  stockAdjust: (adjustment: {
    variant_id: string;
    bucket: string;
    qty_delta: number;
    unit_cost: number | null;
    reason: string;
  }) => call<string>("stock_adjust", { adjustment }),
  stockMove: (movement: {
    variant_id: string;
    from_bucket: string;
    to_bucket: string;
    qty: number;
    reason: string;
  }) => call<string>("stock_move", { movement }),
  serialSetStatus: (serialId: string, status: string, reason: string) =>
    call<string>("serial_set_status", { serialId, status, reason }),
  stockReconcile: () => call<Discrepancy[]>("stock_reconcile"),

  accountsList: (asOf?: string) => call<AccountRow[]>("accounts_list", { asOf }),
  trialBalance: (asOf?: string) => call<TrialBalance>("trial_balance", { asOf }),
  accountLedger: (accountCode: string, from?: string, to?: string) =>
    call<Ledger>("account_ledger", { accountCode, from, to }),
  journalList: (filter: {
    from?: string;
    to?: string;
    search?: string;
    source_type?: string;
    limit?: number;
    offset?: number;
  }) => call<EntryPage>("journal_list", { filter }),
  journalGet: (entryId: string) => call<EntryDetail>("journal_get", { entryId }),
  journalManual: (date: string, memo: string, lines: ManualLine[]) =>
    call<string>("journal_manual", { date, memo, lines }),
  journalReverse: (entryId: string, reason: string) =>
    call<string>("journal_reverse", { entryId, reason }),
  accountOpeningBalance: (accountCode: string, amount: number, date: string) =>
    call<string>("account_opening_balance", { accountCode, amount, date }),
  accountingReconcile: () => call<Mismatch[]>("accounting_reconcile"),

  customersList: (
    opts: {
      search?: string;
      customerType?: string;
      includeInactive?: boolean;
      withBalanceOnly?: boolean;
    } = {},
  ) => call<CustomerRow[]>("customers_list", opts),
  customerGet: (customerId: string) => call<CustomerRow>("customer_get", { customerId }),
  customerCreate: (customer: CustomerIn) => call<string>("customer_create", { customer }),
  customerUpdate: (customerId: string, customer: CustomerIn) =>
    call<void>("customer_update", { customerId, customer }),
  customerSetActive: (customerId: string, active: boolean) =>
    call<void>("customer_set_active", { customerId, active }),
  customerStatement: (customerId: string) =>
    call<CustomerStatement>("customer_statement", { customerId }),
  customerOpeningBalance: (customerId: string, amount: number, date: string) =>
    call<void>("customer_opening_balance", { customerId, amount, date }),
  saleCreate: (sale: SaleIn) => call<SaleResult>("sale_create", { sale }),
  salePreview: (sale: SaleIn) => call<SalePreview>("sale_preview", { sale }),
  salesList: (filter: SaleFilter) => call<SalePage>("sales_list", { filter }),
  saleGet: (saleId: string) => call<SaleDetail>("sale_get", { saleId }),
  saleSerials: (variantId: string) => call<SerialChoice[]>("sale_serials", { variantId }),
  customerCollect: (collect: CollectIn) => call<CollectResult>("customer_collect", { collect }),
  salePaymentReverse: (paymentId: string, reason: string) =>
    call<string>("sale_payment_reverse", { paymentId, reason }),

  tiersList: () => call<TierRow[]>("tiers_list"),
  tierCreate: (name: string, adjustBp: number) => call<string>("tier_create", { name, adjustBp }),
  tierUpdate: (tierId: string, adjustBp: number, active: boolean) =>
    call<void>("tier_update", { tierId, adjustBp, active }),
  customerSetTier: (customerId: string, tierId: string | null) =>
    call<void>("customer_set_tier", { customerId, tierId }),
  customerPricesList: (customerId: string) =>
    call<CustomerPriceRow[]>("customer_prices_list", { customerId }),
  customerPriceSet: (customerId: string, variantId: string, price: number) =>
    call<void>("customer_price_set", { customerId, variantId, price }),
  customerPriceRemove: (customerId: string, variantId: string) =>
    call<void>("customer_price_remove", { customerId, variantId }),
  agreementsList: (filter: AgreementFilter) => call<AgreementPage>("agreements_list", { filter }),
  agreementGet: (agreementId: string) => call<AgreementDetail>("agreement_get", { agreementId }),
  installmentsOverdue: () => call<AgreementRow[]>("installments_overdue"),
  installmentPreview: (sale: SaleIn) => call<AgreementPreview>("installment_preview", { sale }),
  installmentCollect: (collect: InstCollectIn) =>
    call<InstCollectResult>("installment_collect", { collect }),
  installmentReverse: (collectionId: string, reason: string) =>
    call<string>("installment_reverse", { collectionId, reason }),
  installmentSettleQuote: (agreementId: string) =>
    call<SettleQuote>("installment_settle_quote", { agreementId }),
  installmentSettle: (settle: {
    agreement_id: string;
    rebate: number;
    reason: string;
    method: string;
    reference: string;
    client_key: string;
  }) => call<InstCollectResult>("installment_settle", { settle }),
  installmentReschedule: (reschedule: {
    agreement_id: string;
    count: number;
    frequency: string;
    first_due_date: string;
    custom: { due_date: string; amount: number }[];
    reason: string;
  }) => call<void>("installment_reschedule", { reschedule }),
  installmentLateFee: (agreementId: string, scheduleId: string, reason: string) =>
    call<number>("installment_late_fee", { agreementId, scheduleId, reason }),
  installmentFollowup: (agreementId: string, note: string, promiseDate: string | null) =>
    call<void>("installment_followup", { agreementId, note, promiseDate }),
};
