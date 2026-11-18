import {
  BarChart3,
  BookOpen,
  Boxes,
  Building2,
  CalendarClock,
  Landmark,
  LayoutDashboard,
  Package,
  Receipt,
  RotateCcw,
  Settings,
  ShoppingBag,
  ShoppingCart,
  Truck,
  Users,
  UserCog,
  Wallet,
  type LucideIcon,
} from "lucide-react";

export type PageId =
  | "dashboard"
  | "pos"
  | "sales"
  | "wholesale"
  | "installments"
  | "customers"
  | "products"
  | "inventory"
  | "purchases"
  | "suppliers"
  | "returns"
  | "expenses"
  | "accounts"
  | "reports"
  | "staff"
  | "settings";

export interface NavItem {
  id: PageId;
  label: string;
  icon: LucideIcon;
  group: string;
  /** Build phase in which the module becomes functional. */
  phase: "P0" | "P1" | "P2" | "P3" | "P4" | "P5" | "P6";
  summary: string;
  /** Visible when the user holds ANY of these (empty = everyone). */
  anyOf: string[];
}

export const NAV: NavItem[] = [
  {
    id: "dashboard",
    label: "Dashboard",
    icon: LayoutDashboard,
    group: "Overview",
    phase: "P0",
    summary: "System health today; business KPIs from P6.",
    anyOf: [],
  },
  {
    id: "pos",
    label: "New Sale / POS",
    icon: ShoppingCart,
    group: "Sales",
    phase: "P3",
    summary: "Barcode-first retail checkout with split payments and receipts.",
    anyOf: ["sales.create"],
  },
  {
    id: "sales",
    label: "Sales",
    icon: Receipt,
    group: "Sales",
    phase: "P3",
    summary: "Invoice history across retail, wholesale and installment.",
    anyOf: ["sales.view"],
  },
  {
    id: "wholesale",
    label: "Wholesale",
    icon: Building2,
    group: "Sales",
    phase: "P4",
    summary: "Dealer price tiers, credit sales and credit limits.",
    anyOf: ["sales.wholesale"],
  },
  {
    id: "installments",
    label: "Installments",
    icon: CalendarClock,
    group: "Sales",
    phase: "P4",
    summary: "Agreements, schedules, collections and overdue follow-up.",
    anyOf: ["installments.view"],
  },
  {
    id: "customers",
    label: "Customers",
    icon: Users,
    group: "Relationships",
    phase: "P3",
    summary: "Customer 360: purchases, payments, ledger and warranty.",
    anyOf: ["customers.view"],
  },
  {
    id: "suppliers",
    label: "Suppliers",
    icon: Truck,
    group: "Relationships",
    phase: "P2",
    summary: "Supplier profiles, ledger and statements.",
    anyOf: ["suppliers.view", "suppliers.manage"],
  },
  {
    id: "products",
    label: "Products",
    icon: Package,
    group: "Stock",
    phase: "P2",
    summary: "Catalogue, variants, prices and serial tracking.",
    anyOf: ["products.view"],
  },
  {
    id: "inventory",
    label: "Inventory",
    icon: Boxes,
    group: "Stock",
    phase: "P2",
    summary: "Stock ledger, serial lifecycle and adjustments.",
    anyOf: ["inventory.view"],
  },
  {
    id: "purchases",
    label: "Purchases",
    icon: ShoppingBag,
    group: "Stock",
    phase: "P2",
    summary: "Purchase orders, receiving and landed cost.",
    anyOf: ["purchases.view", "purchases.manage"],
  },
  {
    id: "returns",
    label: "Returns & Warranty",
    icon: RotateCcw,
    group: "Operations",
    phase: "P5",
    summary: "Returns, inspection, exchanges and warranty cases.",
    anyOf: ["returns.process", "warranty.manage"],
  },
  {
    id: "expenses",
    label: "Expenses",
    icon: Wallet,
    group: "Finance",
    phase: "P5",
    summary: "Expense entry, approval and the daily cash session.",
    anyOf: ["expenses.create", "expenses.approve", "cash.session"],
  },
  {
    id: "accounts",
    label: "Accounts",
    icon: Landmark,
    group: "Finance",
    phase: "P3",
    summary: "Chart of accounts, journals, cashbook, trial balance.",
    anyOf: ["accounts.view"],
  },
  {
    id: "reports",
    label: "Reports",
    icon: BarChart3,
    group: "Finance",
    phase: "P6",
    summary: "Sales, stock, ledgers, P&L with print/PDF/CSV/XLSX.",
    anyOf: ["reports.operational", "reports.financial"],
  },
  {
    id: "staff",
    label: "Staff & Users",
    icon: UserCog,
    group: "Admin",
    phase: "P1",
    summary: "Users, roles and permissions.",
    anyOf: ["users.view"],
  },
  {
    id: "settings",
    label: "Settings",
    icon: Settings,
    group: "Admin",
    phase: "P1",
    summary: "Shop identity, tax, backup/restore, audit log.",
    anyOf: ["settings.view", "backup.create", "backup.restore", "audit.view"],
  },
];

export const NAV_ICON_FALLBACK = BookOpen;
