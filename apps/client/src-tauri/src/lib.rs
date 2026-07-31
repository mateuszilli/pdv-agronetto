mod commands;
mod db;
mod fiscal_client;
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
        .invoke_handler(tauri::generate_handler![commands::auth::login])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
