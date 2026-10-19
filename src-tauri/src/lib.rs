pub mod chat;
pub mod workspace;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let chat_state = chat::state::ChatState::new_default();

    tauri::Builder::default()
        .manage(chat_state)
        .invoke_handler(tauri::generate_handler![
            workspace::commands::get_workspace_settings,
            workspace::commands::choose_data_root,
            workspace::commands::add_project_root,
            workspace::commands::remove_project_root,
            workspace::commands::toggle_project_provider_sharing,
            workspace::commands::set_project_provider_sharing,
            workspace::commands::list_project_directory,
            workspace::commands::read_local_file_chunk,
            chat::commands::get_chat_settings,
            chat::commands::save_chat_settings,
            chat::commands::list_chat_models,
            chat::commands::list_conversations,
            chat::commands::load_conversation,
            chat::commands::delete_conversation,
            chat::commands::start_chat,
            chat::commands::cancel_chat,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Yuki");
}
