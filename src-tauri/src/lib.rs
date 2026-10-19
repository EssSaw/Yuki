pub mod workspace;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            workspace::commands::get_workspace_settings,
            workspace::commands::choose_data_root,
            workspace::commands::add_project_root,
            workspace::commands::remove_project_root,
            workspace::commands::list_project_directory,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Yuki");
}
