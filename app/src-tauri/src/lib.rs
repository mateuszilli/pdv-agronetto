mod commands;
mod config;
mod db;
mod fiscal_client;
mod money;
mod outbox;
mod printing;
mod sync;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(
            tauri_plugin_sql::Builder::default()
                .add_migrations(db::DB_URL, db::migrations())
                .build(),
        )
        .invoke_handler(tauri::generate_handler![
            commands::auth::login,
            commands::inventory::list_products,
            commands::inventory::get_product,
            commands::inventory::create_product,
            commands::inventory::update_product,
            commands::inventory::list_product_units,
            commands::inventory::list_all_product_units,
            commands::inventory::create_product_unit,
            commands::inventory::update_product_unit,
            commands::inventory::list_low_stock_products,
            commands::purchasing::list_purchase_orders,
            commands::purchasing::get_purchase_order,
            commands::purchasing::create_purchase_order,
            commands::purchasing::receive_purchase_order_items,
            commands::cash::get_open_cash_session,
            commands::cash::open_cash_session,
            commands::cash::record_cash_movement,
            commands::cash::list_cash_movements,
            commands::cash::close_cash_session,
            commands::sales::finalize_sale,
            commands::returns::search_sales_for_return,
            commands::returns::get_sale_for_return,
            commands::returns::create_return,
            commands::customers::list_customers,
            commands::customers::get_customer,
            commands::customers::create_customer,
            commands::customers::update_customer,
            commands::customers::list_receivables,
            commands::customers::list_ledger_entries,
            commands::customers::record_payment,
            commands::fiscal::get_fiscal_status,
            commands::fiscal::retry_fiscal_emission,
            commands::fiscal::retry_fiscal_cancellation,
            commands::fiscal::print_receipt,
            sync::sync_now,
        ])
        .setup(|app| {
            sync::spawn_worker(app.handle().clone());
            fiscal_client::spawn_worker(app.handle().clone());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
