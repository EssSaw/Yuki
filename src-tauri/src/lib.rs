pub mod chat;
pub mod mcp;
pub mod notes;
pub mod workspace;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let chat_state = chat::state::ChatState::new_default();
    let mcp_state = mcp::session::McpSessionManager::default();

    tauri::Builder::default()
        .manage(chat_state)
        .manage(mcp_state)
        .invoke_handler(tauri::generate_handler![
            workspace::commands::get_workspace_settings,
            workspace::commands::choose_data_root,
            workspace::commands::add_project_root,
            workspace::commands::remove_project_root,
            workspace::commands::toggle_project_provider_sharing,
            workspace::commands::set_project_provider_sharing,
            workspace::commands::list_project_directory,
            workspace::commands::read_local_file_chunk,
            notes::commands::create_note,
            notes::commands::list_notes,
            notes::commands::load_note,
            notes::commands::save_note,
            chat::commands::get_chat_settings,
            chat::commands::save_chat_settings,
            chat::commands::list_chat_models,
            chat::commands::list_conversations,
            chat::commands::load_conversation,
            chat::commands::delete_conversation,
            chat::commands::start_chat,
            chat::commands::cancel_chat,
            mcp::commands::list_mcp_candidates,
            mcp::commands::discover_mcp_candidates,
            mcp::commands::approve_mcp_endpoint,
            mcp::commands::revoke_mcp_endpoint,
            mcp::commands::get_mcp_grants,
            mcp::commands::approve_mcp_executable,
            mcp::commands::revoke_mcp_executable,
            mcp::commands::get_mcp_executable_grants,
            mcp::commands::connect_approved_mcp_endpoint,
            mcp::commands::connect_approved_mcp_stdio,
            mcp::commands::list_mcp_stdio_capabilities,
            mcp::commands::list_connected_unity_instances,
            mcp::commands::bind_unity_instance,
            mcp::commands::get_bound_unity_state,
            mcp::commands::disconnect,
            mcp::commands::disconnect_mcp_session,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Yuki");
}
