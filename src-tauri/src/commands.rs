//! Thin command layer. Each command: authenticate (from Rust-held session) -> call a core service.
//! Authorization lives in the services; nothing here decides permissions.

use crate::state::AppState;
use base64::Engine;
use serde::Serialize;
use singerpro_core::authz::Principal;
use singerpro_core::recovery::{Marker, RestoreOutcome, StagedRestore};
use singerpro_core::users::{NewUser, PermissionRow, RoleRow, UserRow};
use singerpro_core::{audit, auth, backup, db, recovery, settings, users, CoreError, Id};
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use tauri::State;

type R<T> = Result<T, CoreError>;

#[derive(Serialize)]
pub struct AppInfo {
    product: &'static str,
    developer: &'static str,
    version: &'static str,
    data_dir: String,
}

#[tauri::command]
pub fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        product: "Singer Pro",
        developer: "PETRA Software Solutions",
        version: env!("CARGO_PKG_VERSION"),
        data_dir: state.paths.root.display().to_string(),
    }
}

#[derive(Serialize)]
pub struct SetupStatus {
    needs_setup: bool,
    startup_notice: Option<RestoreOutcome>,
}

/// Public (pre-login): is this a fresh install, and did a restore just run?
#[tauri::command]
pub async fn setup_status(state: State<'_, AppState>) -> R<SetupStatus> {
    Ok(SetupStatus {
        needs_setup: auth::needs_setup(&state.pool).await?,
        startup_notice: state.startup_notice.lock().expect("lock").take(),
    })
}

#[tauri::command]
pub async fn setup_owner(
    state: State<'_, AppState>,
    username: String,
    full_name: String,
    password: String,
) -> R<Principal> {
    let id = auth::setup_owner(&state.pool, &username, &full_name, &password).await?;
    start_session(&state, id).await
}

async fn start_session(state: &AppState, id: Id) -> R<Principal> {
    let token = state.store.create(id)?;
    state.set_token(Some(token));
    state.principal().await
}

#[tauri::command]
pub async fn login(state: State<'_, AppState>, username: String, password: String) -> R<Principal> {
    let id = auth::login(&state.pool, &username, &password).await?;
    start_session(&state, id).await
}

#[tauri::command]
pub async fn logout(state: State<'_, AppState>) -> R<()> {
    if let Some(t) = state.token() {
        auth::logout(&state.pool, &state.store, &t).await?;
    }
    state.set_token(None);
    Ok(())
}

/// `None` when not signed in or the session expired.
#[tauri::command]
pub async fn session(state: State<'_, AppState>) -> R<Option<Principal>> {
    match state.principal().await {
        Ok(p) => Ok(Some(p)),
        Err(CoreError::Unauthenticated) => Ok(None),
        Err(e) => Err(e),
    }
}

#[tauri::command]
pub async fn change_password(state: State<'_, AppState>, old: String, new: String) -> R<()> {
    let p = state.principal().await?;
    users::change_own_password(&state.pool, &state.store, &p, &old, &new).await?;
    state.set_token(None); // sessions were revoked; the user signs in again with the new password
    Ok(())
}

#[tauri::command]
pub async fn db_status(state: State<'_, AppState>) -> R<db::DbStatus> {
    state.principal().await?;
    db::status(&state.pool).await
}

#[tauri::command]
pub async fn audit_list(
    state: State<'_, AppState>,
    limit: Option<i64>,
    offset: Option<i64>,
    action_prefix: Option<String>,
) -> R<audit::AuditPage> {
    let p = state.principal().await?;
    audit::list(
        &state.pool,
        &p,
        limit.unwrap_or(50),
        offset.unwrap_or(0),
        action_prefix.as_deref(),
    )
    .await
}

// ---- users & roles

#[tauri::command]
pub async fn users_list(state: State<'_, AppState>) -> R<Vec<UserRow>> {
    let p = state.principal().await?;
    users::list_users(&state.pool, &p).await
}

#[tauri::command]
pub async fn roles_list(state: State<'_, AppState>) -> R<Vec<RoleRow>> {
    let p = state.principal().await?;
    users::list_roles(&state.pool, &p).await
}

#[tauri::command]
pub async fn permissions_list(state: State<'_, AppState>) -> R<Vec<PermissionRow>> {
    let p = state.principal().await?;
    users::list_permissions(&state.pool, &p).await
}

#[tauri::command]
pub async fn user_create(state: State<'_, AppState>, user: NewUser) -> R<String> {
    let p = state.principal().await?;
    Ok(users::create_user(&state.pool, &p, user).await?.to_string())
}

#[tauri::command]
pub async fn user_set_active(
    state: State<'_, AppState>,
    user_id: String,
    active: bool,
    reason: String,
) -> R<()> {
    let p = state.principal().await?;
    users::set_user_active(
        &state.pool,
        &state.store,
        &p,
        Id::parse(&user_id)?,
        active,
        &reason,
    )
    .await
}

#[tauri::command]
pub async fn user_set_roles(
    state: State<'_, AppState>,
    user_id: String,
    role_ids: Vec<String>,
) -> R<()> {
    let p = state.principal().await?;
    users::set_user_roles(&state.pool, &p, Id::parse(&user_id)?, role_ids).await
}

#[tauri::command]
pub async fn user_reset_password(
    state: State<'_, AppState>,
    user_id: String,
    password: String,
) -> R<()> {
    let p = state.principal().await?;
    users::reset_password(
        &state.pool,
        &state.store,
        &p,
        Id::parse(&user_id)?,
        &password,
    )
    .await
}

#[tauri::command]
pub async fn role_create(
    state: State<'_, AppState>,
    name: String,
    description: String,
    permissions: Vec<String>,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        users::create_role(&state.pool, &p, &name, &description, permissions)
            .await?
            .to_string(),
    )
}

#[tauri::command]
pub async fn role_set_permissions(
    state: State<'_, AppState>,
    role_id: String,
    permissions: Vec<String>,
) -> R<()> {
    let p = state.principal().await?;
    users::set_role_permissions(&state.pool, &p, Id::parse(&role_id)?, permissions).await
}

#[tauri::command]
pub async fn role_delete(state: State<'_, AppState>, role_id: String) -> R<()> {
    let p = state.principal().await?;
    users::delete_role(&state.pool, &p, Id::parse(&role_id)?).await
}

// ---- settings & logo

#[tauri::command]
pub async fn settings_get(state: State<'_, AppState>) -> R<BTreeMap<String, String>> {
    let p = state.principal().await?;
    settings::get_all(&state.pool, &p).await
}

#[tauri::command]
pub async fn settings_update(
    state: State<'_, AppState>,
    changes: HashMap<String, String>,
) -> R<Vec<String>> {
    let p = state.principal().await?;
    settings::update(&state.pool, &p, changes).await
}

#[tauri::command]
pub async fn branding(state: State<'_, AppState>) -> R<settings::Branding> {
    let p = state.principal().await?;
    settings::branding(&state.pool, &p).await
}

#[tauri::command]
pub async fn logo_get(state: State<'_, AppState>) -> R<Option<String>> {
    let p = state.principal().await?;
    Ok(settings::get_logo(&state.pool, &p)
        .await?
        .map(|(mime, data)| {
            format!(
                "data:{mime};base64,{}",
                base64::engine::general_purpose::STANDARD.encode(data)
            )
        }))
}

#[tauri::command]
pub async fn logo_set(state: State<'_, AppState>, data_base64: String) -> R<()> {
    let p = state.principal().await?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|_| CoreError::Validation("logo data is not valid base64".into()))?;
    settings::set_logo(&state.pool, &p, &bytes).await
}

#[tauri::command]
pub async fn logo_clear(state: State<'_, AppState>) -> R<()> {
    let p = state.principal().await?;
    settings::clear_logo(&state.pool, &p).await
}

// ---- backup / restore

#[tauri::command]
pub async fn backup_create(
    state: State<'_, AppState>,
    dest_dir: Option<String>,
) -> R<backup::BackupInfo> {
    let p = state.principal().await?;
    let dir = dest_dir
        .map(PathBuf::from)
        .unwrap_or_else(|| state.paths.default_backups.clone());
    backup::create_backup(&state.pool, &p, &dir).await
}

#[tauri::command]
pub async fn backup_history(state: State<'_, AppState>) -> R<Vec<backup::HistoryRow>> {
    let p = state.principal().await?;
    backup::history(&state.pool, &p, 50).await
}

#[tauri::command]
pub async fn backup_status(state: State<'_, AppState>) -> R<backup::BackupStatus> {
    let p = state.principal().await?;
    backup::status(&state.pool, &p).await
}

#[tauri::command]
pub async fn backup_default_dir(state: State<'_, AppState>) -> R<String> {
    state.principal().await?;
    Ok(state.paths.default_backups.display().to_string())
}

#[tauri::command]
pub async fn backup_verify(state: State<'_, AppState>, path: String) -> R<backup::Preview> {
    let p = state.principal().await?;
    backup::verify_backup(&p, &PathBuf::from(path)).await
}

#[tauri::command]
pub async fn restore_stage(state: State<'_, AppState>, path: String) -> R<StagedRestore> {
    let p = state.principal().await?;
    recovery::stage_restore(&state.pool, &p, &state.paths, &PathBuf::from(path)).await
}

#[tauri::command]
pub async fn restore_pending(state: State<'_, AppState>) -> R<Option<Marker>> {
    state.principal().await?;
    Ok(recovery::pending(&state.paths))
}

#[tauri::command]
pub async fn restore_cancel(state: State<'_, AppState>) -> R<()> {
    let p = state.principal().await?;
    recovery::cancel_pending(&state.pool, &p, &state.paths).await
}

/// Restarts so a staged restore is applied before the database is opened.
#[tauri::command]
pub async fn restart_app(app: tauri::AppHandle, state: State<'_, AppState>) -> R<()> {
    state.principal().await?;
    state.pool.close().await;
    app.restart()
}

// ---- P2: catalogue

use singerpro_core::catalogue::{
    self, CategoryRow, Named, NewProduct, NewVariant, ProductFilter, ProductPage, ProductRow,
    UpdateProduct, UpdateVariant, VariantPick,
};
use singerpro_core::inventory::{
    self, AdjustIn, MoveIn, MovementRow, OpeningStockIn, SerialEventRow, SerialFilter, SerialRow,
    StockFilter, StockPage,
};
use singerpro_core::purchases::{
    self, PaymentIn, PurchaseDetail, PurchaseFilter, PurchaseIn, PurchasePage, ReceiveIn, ReturnIn,
};
use singerpro_core::suppliers::{self, Statement, SupplierIn, SupplierRow};
use singerpro_core::{stock, Id as CoreId};

fn id(s: &str) -> R<CoreId> {
    CoreId::parse(s)
}

#[tauri::command]
pub async fn brands_list(state: State<'_, AppState>) -> R<Vec<Named>> {
    let p = state.principal().await?;
    catalogue::list_brands(&state.pool, &p).await
}

#[tauri::command]
pub async fn brand_create(state: State<'_, AppState>, name: String) -> R<String> {
    let p = state.principal().await?;
    Ok(catalogue::create_brand(&state.pool, &p, &name)
        .await?
        .to_string())
}

#[tauri::command]
pub async fn brand_rename(state: State<'_, AppState>, brand_id: String, name: String) -> R<()> {
    let p = state.principal().await?;
    catalogue::rename_brand(&state.pool, &p, id(&brand_id)?, &name).await
}

#[tauri::command]
pub async fn brand_set_active(state: State<'_, AppState>, brand_id: String, active: bool) -> R<()> {
    let p = state.principal().await?;
    catalogue::set_brand_active(&state.pool, &p, id(&brand_id)?, active).await
}

#[tauri::command]
pub async fn categories_list(state: State<'_, AppState>) -> R<Vec<CategoryRow>> {
    let p = state.principal().await?;
    catalogue::list_categories(&state.pool, &p).await
}

#[tauri::command]
pub async fn category_create(
    state: State<'_, AppState>,
    name: String,
    parent_id: Option<String>,
) -> R<String> {
    let p = state.principal().await?;
    let parent = parent_id.as_deref().map(CoreId::parse).transpose()?;
    Ok(catalogue::create_category(&state.pool, &p, &name, parent)
        .await?
        .to_string())
}

#[tauri::command]
pub async fn category_set_active(
    state: State<'_, AppState>,
    category_id: String,
    active: bool,
) -> R<()> {
    let p = state.principal().await?;
    catalogue::set_category_active(&state.pool, &p, id(&category_id)?, active).await
}

#[tauri::command]
pub async fn products_list(state: State<'_, AppState>, filter: ProductFilter) -> R<ProductPage> {
    let p = state.principal().await?;
    catalogue::list_products(&state.pool, &p, filter).await
}

#[tauri::command]
pub async fn product_get(state: State<'_, AppState>, product_id: String) -> R<ProductRow> {
    let p = state.principal().await?;
    catalogue::get_product(&state.pool, &p, id(&product_id)?).await
}

#[tauri::command]
pub async fn product_create(state: State<'_, AppState>, product: NewProduct) -> R<String> {
    let p = state.principal().await?;
    Ok(catalogue::create_product(&state.pool, &p, product)
        .await?
        .to_string())
}

#[tauri::command]
pub async fn product_update(state: State<'_, AppState>, product: UpdateProduct) -> R<()> {
    let p = state.principal().await?;
    catalogue::update_product(&state.pool, &p, product).await
}

#[tauri::command]
pub async fn product_set_active(
    state: State<'_, AppState>,
    product_id: String,
    active: bool,
) -> R<()> {
    let p = state.principal().await?;
    catalogue::set_product_active(&state.pool, &p, id(&product_id)?, active).await
}

#[tauri::command]
pub async fn variant_add(
    state: State<'_, AppState>,
    product_id: String,
    variant: NewVariant,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        catalogue::add_variant(&state.pool, &p, id(&product_id)?, variant)
            .await?
            .to_string(),
    )
}

#[tauri::command]
pub async fn variant_update(state: State<'_, AppState>, variant: UpdateVariant) -> R<()> {
    let p = state.principal().await?;
    catalogue::update_variant(&state.pool, &p, variant).await
}

#[tauri::command]
pub async fn variant_set_active(
    state: State<'_, AppState>,
    variant_id: String,
    active: bool,
) -> R<()> {
    let p = state.principal().await?;
    catalogue::set_variant_active(&state.pool, &p, id(&variant_id)?, active).await
}

#[tauri::command]
pub async fn product_image_get(
    state: State<'_, AppState>,
    product_id: String,
) -> R<Option<String>> {
    let p = state.principal().await?;
    Ok(
        catalogue::get_product_image(&state.pool, &p, id(&product_id)?)
            .await?
            .map(|(mime, data)| {
                format!(
                    "data:{mime};base64,{}",
                    base64::engine::general_purpose::STANDARD.encode(data)
                )
            }),
    )
}

#[tauri::command]
pub async fn product_image_set(
    state: State<'_, AppState>,
    product_id: String,
    data_base64: String,
) -> R<()> {
    let p = state.principal().await?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data_base64.trim())
        .map_err(|_| CoreError::Validation("image data is not valid base64".into()))?;
    catalogue::set_product_image(&state.pool, &p, id(&product_id)?, &bytes).await
}

#[tauri::command]
pub async fn variants_search(
    state: State<'_, AppState>,
    query: String,
    limit: Option<i64>,
) -> R<Vec<VariantPick>> {
    let p = state.principal().await?;
    catalogue::search_variants(&state.pool, &p, &query, limit.unwrap_or(15)).await
}

// ---- P2: suppliers & purchases

#[tauri::command]
pub async fn suppliers_list(
    state: State<'_, AppState>,
    search: Option<String>,
    include_inactive: Option<bool>,
) -> R<Vec<SupplierRow>> {
    let p = state.principal().await?;
    suppliers::list_suppliers(
        &state.pool,
        &p,
        search.as_deref(),
        include_inactive.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn supplier_create(state: State<'_, AppState>, supplier: SupplierIn) -> R<String> {
    let p = state.principal().await?;
    Ok(suppliers::create_supplier(&state.pool, &p, supplier)
        .await?
        .to_string())
}

#[tauri::command]
pub async fn supplier_update(
    state: State<'_, AppState>,
    supplier_id: String,
    supplier: SupplierIn,
) -> R<()> {
    let p = state.principal().await?;
    suppliers::update_supplier(&state.pool, &p, id(&supplier_id)?, supplier).await
}

#[tauri::command]
pub async fn supplier_set_active(
    state: State<'_, AppState>,
    supplier_id: String,
    active: bool,
) -> R<()> {
    let p = state.principal().await?;
    suppliers::set_supplier_active(&state.pool, &p, id(&supplier_id)?, active).await
}

#[tauri::command]
pub async fn supplier_statement(state: State<'_, AppState>, supplier_id: String) -> R<Statement> {
    let p = state.principal().await?;
    suppliers::statement(&state.pool, &p, id(&supplier_id)?).await
}

#[tauri::command]
pub async fn purchases_list(state: State<'_, AppState>, filter: PurchaseFilter) -> R<PurchasePage> {
    let p = state.principal().await?;
    purchases::list_purchases(&state.pool, &p, filter).await
}

#[tauri::command]
pub async fn purchase_get(state: State<'_, AppState>, purchase_id: String) -> R<PurchaseDetail> {
    let p = state.principal().await?;
    purchases::get_purchase(&state.pool, &p, id(&purchase_id)?).await
}

#[tauri::command]
pub async fn purchase_create_draft(state: State<'_, AppState>, purchase: PurchaseIn) -> R<String> {
    let p = state.principal().await?;
    Ok(purchases::create_draft(&state.pool, &p, purchase)
        .await?
        .to_string())
}

#[tauri::command]
pub async fn purchase_update_draft(
    state: State<'_, AppState>,
    purchase_id: String,
    purchase: PurchaseIn,
) -> R<()> {
    let p = state.principal().await?;
    purchases::update_draft(&state.pool, &p, id(&purchase_id)?, purchase).await
}

#[tauri::command]
pub async fn purchase_confirm(state: State<'_, AppState>, purchase_id: String) -> R<()> {
    let p = state.principal().await?;
    purchases::confirm(&state.pool, &p, id(&purchase_id)?).await
}

#[tauri::command]
pub async fn purchase_cancel(
    state: State<'_, AppState>,
    purchase_id: String,
    reason: String,
) -> R<()> {
    let p = state.principal().await?;
    purchases::cancel(&state.pool, &p, id(&purchase_id)?, &reason).await
}

#[tauri::command]
pub async fn purchase_receive(
    state: State<'_, AppState>,
    purchase_id: String,
    receipt: ReceiveIn,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        purchases::receive(&state.pool, &p, id(&purchase_id)?, receipt)
            .await?
            .to_string(),
    )
}

#[tauri::command]
pub async fn purchase_return(
    state: State<'_, AppState>,
    purchase_id: String,
    ret: ReturnIn,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        purchases::return_to_supplier(&state.pool, &p, id(&purchase_id)?, ret)
            .await?
            .to_string(),
    )
}

#[tauri::command]
pub async fn supplier_pay(state: State<'_, AppState>, payment: PaymentIn) -> R<String> {
    let p = state.principal().await?;
    Ok(purchases::pay_supplier(&state.pool, &p, payment)
        .await?
        .to_string())
}

#[tauri::command]
pub async fn supplier_payment_reverse(
    state: State<'_, AppState>,
    payment_id: String,
    reason: String,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        purchases::reverse_payment(&state.pool, &p, id(&payment_id)?, &reason)
            .await?
            .to_string(),
    )
}

// ---- P2: inventory

#[tauri::command]
pub async fn stock_levels(state: State<'_, AppState>, filter: StockFilter) -> R<StockPage> {
    let p = state.principal().await?;
    inventory::stock_levels(&state.pool, &p, filter).await
}

#[tauri::command]
pub async fn stock_movements(
    state: State<'_, AppState>,
    variant_id: String,
    limit: Option<i64>,
    offset: Option<i64>,
) -> R<Vec<MovementRow>> {
    let p = state.principal().await?;
    inventory::movements(
        &state.pool,
        &p,
        id(&variant_id)?,
        limit.unwrap_or(100),
        offset.unwrap_or(0),
    )
    .await
}

#[tauri::command]
pub async fn serials_list(state: State<'_, AppState>, filter: SerialFilter) -> R<Vec<SerialRow>> {
    let p = state.principal().await?;
    inventory::serials(&state.pool, &p, filter).await
}

#[tauri::command]
pub async fn serial_history(
    state: State<'_, AppState>,
    serial_id: String,
) -> R<Vec<SerialEventRow>> {
    let p = state.principal().await?;
    inventory::serial_history(&state.pool, &p, id(&serial_id)?).await
}

#[tauri::command]
pub async fn stock_opening(state: State<'_, AppState>, opening: OpeningStockIn) -> R<String> {
    let p = state.principal().await?;
    Ok(inventory::record_opening_stock(&state.pool, &p, opening)
        .await?
        .to_string())
}

#[tauri::command]
pub async fn stock_adjust(state: State<'_, AppState>, adjustment: AdjustIn) -> R<String> {
    let p = state.principal().await?;
    Ok(inventory::adjust_quantity(&state.pool, &p, adjustment)
        .await?
        .to_string())
}

#[tauri::command]
pub async fn stock_move(state: State<'_, AppState>, movement: MoveIn) -> R<String> {
    let p = state.principal().await?;
    Ok(inventory::move_quantity(&state.pool, &p, movement)
        .await?
        .to_string())
}

#[tauri::command]
pub async fn serial_set_status(
    state: State<'_, AppState>,
    serial_id: String,
    status: String,
    reason: String,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        inventory::change_serial_status(&state.pool, &p, id(&serial_id)?, &status, &reason)
            .await?
            .to_string(),
    )
}

#[tauri::command]
pub async fn stock_reconcile(state: State<'_, AppState>) -> R<Vec<stock::Discrepancy>> {
    let p = state.principal().await?;
    stock::reconcile(&state.pool, &p).await
}

// ---- P3: accounting, customers, sales

use singerpro_core::accounting::{
    self, AccountRow, EntryDetail, EntryFilter, EntryPage, Ledger, ManualLine, Mismatch,
    TrialBalance,
};
use singerpro_core::customers::{self, CustomerIn, CustomerRow, Statement as CustomerStatement};
use singerpro_core::sales::{
    self, CollectIn, CollectResult, SaleDetail, SaleFilter, SaleIn, SalePage, SaleResult,
    SerialChoice,
};

#[tauri::command]
pub async fn accounts_list(
    state: State<'_, AppState>,
    as_of: Option<String>,
) -> R<Vec<AccountRow>> {
    let p = state.principal().await?;
    accounting::list_accounts(&state.pool, &p, as_of.as_deref()).await
}

#[tauri::command]
pub async fn trial_balance(state: State<'_, AppState>, as_of: Option<String>) -> R<TrialBalance> {
    let p = state.principal().await?;
    accounting::trial_balance(&state.pool, &p, as_of.as_deref()).await
}

#[tauri::command]
pub async fn account_ledger(
    state: State<'_, AppState>,
    account_code: String,
    from: Option<String>,
    to: Option<String>,
) -> R<Ledger> {
    let p = state.principal().await?;
    accounting::ledger(
        &state.pool,
        &p,
        &account_code,
        from.as_deref(),
        to.as_deref(),
    )
    .await
}

#[tauri::command]
pub async fn journal_list(state: State<'_, AppState>, filter: EntryFilter) -> R<EntryPage> {
    let p = state.principal().await?;
    accounting::list_entries(&state.pool, &p, filter).await
}

#[tauri::command]
pub async fn journal_get(state: State<'_, AppState>, entry_id: String) -> R<EntryDetail> {
    let p = state.principal().await?;
    accounting::get_entry(&state.pool, &p, id(&entry_id)?).await
}

#[tauri::command]
pub async fn journal_manual(
    state: State<'_, AppState>,
    date: String,
    memo: String,
    lines: Vec<ManualLine>,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        accounting::manual_journal(&state.pool, &p, &date, &memo, lines)
            .await?
            .to_string(),
    )
}

#[tauri::command]
pub async fn journal_reverse(
    state: State<'_, AppState>,
    entry_id: String,
    reason: String,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        accounting::reverse_entry(&state.pool, &p, id(&entry_id)?, &reason)
            .await?
            .to_string(),
    )
}

#[tauri::command]
pub async fn account_opening_balance(
    state: State<'_, AppState>,
    account_code: String,
    amount: i64,
    date: String,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        accounting::opening_balance(&state.pool, &p, &account_code, amount, &date)
            .await?
            .to_string(),
    )
}

#[tauri::command]
pub async fn accounting_reconcile(state: State<'_, AppState>) -> R<Vec<Mismatch>> {
    let p = state.principal().await?;
    accounting::reconcile(&state.pool, &p).await
}

#[tauri::command]
pub async fn customers_list(
    state: State<'_, AppState>,
    search: Option<String>,
    customer_type: Option<String>,
    include_inactive: Option<bool>,
    with_balance_only: Option<bool>,
) -> R<Vec<CustomerRow>> {
    let p = state.principal().await?;
    customers::list_customers(
        &state.pool,
        &p,
        search.as_deref(),
        customer_type.as_deref(),
        include_inactive.unwrap_or(false),
        with_balance_only.unwrap_or(false),
    )
    .await
}

#[tauri::command]
pub async fn customer_get(state: State<'_, AppState>, customer_id: String) -> R<CustomerRow> {
    let p = state.principal().await?;
    customers::get_customer(&state.pool, &p, id(&customer_id)?).await
}

#[tauri::command]
pub async fn customer_create(state: State<'_, AppState>, customer: CustomerIn) -> R<String> {
    let p = state.principal().await?;
    Ok(customers::create_customer(&state.pool, &p, customer)
        .await?
        .to_string())
}

#[tauri::command]
pub async fn customer_update(
    state: State<'_, AppState>,
    customer_id: String,
    customer: CustomerIn,
) -> R<()> {
    let p = state.principal().await?;
    customers::update_customer(&state.pool, &p, id(&customer_id)?, customer).await
}

#[tauri::command]
pub async fn customer_set_active(
    state: State<'_, AppState>,
    customer_id: String,
    active: bool,
) -> R<()> {
    let p = state.principal().await?;
    customers::set_customer_active(&state.pool, &p, id(&customer_id)?, active).await
}

#[tauri::command]
pub async fn customer_statement(
    state: State<'_, AppState>,
    customer_id: String,
) -> R<CustomerStatement> {
    let p = state.principal().await?;
    customers::statement(&state.pool, &p, id(&customer_id)?).await
}

#[tauri::command]
pub async fn customer_opening_balance(
    state: State<'_, AppState>,
    customer_id: String,
    amount: i64,
    date: String,
) -> R<()> {
    let p = state.principal().await?;
    customers::opening_balance(&state.pool, &p, id(&customer_id)?, amount, &date).await
}

#[tauri::command]
pub async fn sale_create(state: State<'_, AppState>, sale: SaleIn) -> R<SaleResult> {
    let p = state.principal().await?;
    sales::create_sale(&state.pool, &p, sale).await
}

#[tauri::command]
pub async fn sales_list(state: State<'_, AppState>, filter: SaleFilter) -> R<SalePage> {
    let p = state.principal().await?;
    sales::list_sales(&state.pool, &p, filter).await
}

#[tauri::command]
pub async fn sale_get(state: State<'_, AppState>, sale_id: String) -> R<SaleDetail> {
    let p = state.principal().await?;
    sales::get_sale(&state.pool, &p, id(&sale_id)?).await
}

#[tauri::command]
pub async fn sale_serials(state: State<'_, AppState>, variant_id: String) -> R<Vec<SerialChoice>> {
    let p = state.principal().await?;
    sales::available_serials(&state.pool, &p, id(&variant_id)?).await
}

#[tauri::command]
pub async fn customer_collect(state: State<'_, AppState>, collect: CollectIn) -> R<CollectResult> {
    let p = state.principal().await?;
    sales::collect_payment(&state.pool, &p, collect).await
}

#[tauri::command]
pub async fn sale_payment_reverse(
    state: State<'_, AppState>,
    payment_id: String,
    reason: String,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        sales::reverse_payment(&state.pool, &p, id(&payment_id)?, &reason)
            .await?
            .to_string(),
    )
}

#[tauri::command]
pub async fn sale_preview(state: State<'_, AppState>, sale: SaleIn) -> R<sales::SalePreview> {
    let p = state.principal().await?;
    sales::preview_sale(&state.pool, &p, &sale).await
}

// ---- P4: wholesale pricing and installments

use singerpro_core::installments::{
    self, AgreementDetail, AgreementFilter, AgreementPage, AgreementRow, RescheduleIn, SettleIn,
    SettleQuote,
};
use singerpro_core::pricing::{self, CustomerPriceRow, TierRow};

#[tauri::command]
pub async fn tiers_list(state: State<'_, AppState>) -> R<Vec<TierRow>> {
    let p = state.principal().await?;
    pricing::list_tiers(&state.pool, &p).await
}

#[tauri::command]
pub async fn tier_create(state: State<'_, AppState>, name: String, adjust_bp: i64) -> R<String> {
    let p = state.principal().await?;
    Ok(pricing::create_tier(&state.pool, &p, &name, adjust_bp)
        .await?
        .to_string())
}

#[tauri::command]
pub async fn tier_update(
    state: State<'_, AppState>,
    tier_id: String,
    adjust_bp: i64,
    active: bool,
) -> R<()> {
    let p = state.principal().await?;
    pricing::update_tier(&state.pool, &p, id(&tier_id)?, adjust_bp, active).await
}

#[tauri::command]
pub async fn customer_set_tier(
    state: State<'_, AppState>,
    customer_id: String,
    tier_id: Option<String>,
) -> R<()> {
    let p = state.principal().await?;
    let tier = tier_id.as_deref().map(CoreId::parse).transpose()?;
    pricing::set_customer_tier(&state.pool, &p, id(&customer_id)?, tier).await
}

#[tauri::command]
pub async fn customer_prices_list(
    state: State<'_, AppState>,
    customer_id: String,
) -> R<Vec<CustomerPriceRow>> {
    let p = state.principal().await?;
    pricing::list_customer_prices(&state.pool, &p, id(&customer_id)?).await
}

#[tauri::command]
pub async fn customer_price_set(
    state: State<'_, AppState>,
    customer_id: String,
    variant_id: String,
    price: i64,
) -> R<()> {
    let p = state.principal().await?;
    pricing::set_customer_price(&state.pool, &p, id(&customer_id)?, id(&variant_id)?, price).await
}

#[tauri::command]
pub async fn customer_price_remove(
    state: State<'_, AppState>,
    customer_id: String,
    variant_id: String,
) -> R<()> {
    let p = state.principal().await?;
    pricing::remove_customer_price(&state.pool, &p, id(&customer_id)?, id(&variant_id)?).await
}

#[tauri::command]
pub async fn agreements_list(
    state: State<'_, AppState>,
    filter: AgreementFilter,
) -> R<AgreementPage> {
    let p = state.principal().await?;
    installments::list_agreements(&state.pool, &p, filter).await
}

#[tauri::command]
pub async fn agreement_get(state: State<'_, AppState>, agreement_id: String) -> R<AgreementDetail> {
    let p = state.principal().await?;
    installments::get_agreement(&state.pool, &p, id(&agreement_id)?).await
}

#[tauri::command]
pub async fn installments_overdue(state: State<'_, AppState>) -> R<Vec<AgreementRow>> {
    let p = state.principal().await?;
    installments::overdue_worklist(&state.pool, &p).await
}

#[tauri::command]
pub async fn installment_collect(
    state: State<'_, AppState>,
    collect: installments::CollectIn,
) -> R<installments::CollectResult> {
    let p = state.principal().await?;
    installments::collect(&state.pool, &p, collect).await
}

#[tauri::command]
pub async fn installment_reverse(
    state: State<'_, AppState>,
    collection_id: String,
    reason: String,
) -> R<String> {
    let p = state.principal().await?;
    Ok(
        installments::reverse_collection(&state.pool, &p, id(&collection_id)?, &reason)
            .await?
            .to_string(),
    )
}

#[tauri::command]
pub async fn installment_settle_quote(
    state: State<'_, AppState>,
    agreement_id: String,
) -> R<SettleQuote> {
    let p = state.principal().await?;
    installments::settle_quote(&state.pool, &p, id(&agreement_id)?).await
}

#[tauri::command]
pub async fn installment_settle(
    state: State<'_, AppState>,
    settle: SettleIn,
) -> R<installments::CollectResult> {
    let p = state.principal().await?;
    installments::settle_early(&state.pool, &p, settle).await
}

#[tauri::command]
pub async fn installment_reschedule(state: State<'_, AppState>, reschedule: RescheduleIn) -> R<()> {
    let p = state.principal().await?;
    installments::reschedule(&state.pool, &p, reschedule).await
}

#[tauri::command]
pub async fn installment_late_fee(
    state: State<'_, AppState>,
    agreement_id: String,
    schedule_id: String,
    reason: String,
) -> R<i64> {
    let p = state.principal().await?;
    installments::apply_late_fee(
        &state.pool,
        &p,
        id(&agreement_id)?,
        id(&schedule_id)?,
        &reason,
    )
    .await
}

#[tauri::command]
pub async fn installment_followup(
    state: State<'_, AppState>,
    agreement_id: String,
    note: String,
    promise_date: Option<String>,
) -> R<()> {
    let p = state.principal().await?;
    installments::add_followup(
        &state.pool,
        &p,
        id(&agreement_id)?,
        &note,
        promise_date.as_deref(),
    )
    .await
}
